// SPDX-License-Identifier: MIT

//! A dedicated native network thread for a synchronous host. No Java packets or
//! registries cross this boundary. World admission/presentation belongs to the host.
use crate::{
    negotiation::{OfflineHttp, OfflineProfile, connect_offline_http, retire},
    startup::{self, InitializedConnection, NetworkReady, Setup},
};
use bedrock_client::spawn::StartupData;
use bedrock_client_tokio::connection::Connection;
use bedrock_protocol::{packets, wire};
use p384::ecdsa::SigningKey;
use serde_json::value::RawValue;
use std::{io, sync::Arc, thread::JoinHandle, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, timeout, timeout_at},
};

pub type Metadata = Box<dyn FnOnce(&Connection) -> io::Result<Box<RawValue>> + Send + 'static>;

/// Explicit offline connection inputs. Trust and identity are caller decisions;
/// the deadline includes time spent waiting for the network thread to start.
pub struct Request {
    pub http: OfflineHttp,
    pub key: Arc<SigningKey>,
    pub profile: OfflineProfile,
    pub deadline: std::time::Instant,
    pub radius: packets::RequestChunkRadius,
    pub metadata: Metadata,
}

pub enum Event {
    /// Original first definitions, copied for the host while the network owner
    /// retains the single initialization capability. This is not a world join.
    NetworkReady(Box<StartupData>),
    /// One original header/body, including unknown packet IDs.
    Frame(Vec<u8>),
    /// The host's presentation completion was acknowledged once on the wire.
    Initialized,
    Disconnected(io::Error),
}

enum Command {
    NextFrame,
    Presented,
    Send(Box<packets::Packet>),
}

pub struct Session {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    cancel: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Session {
    pub fn connect(request: Request) -> io::Result<Self> {
        let (commands, receiver) = mpsc::channel(4);
        let (sender, events) = mpsc::channel(2);
        let (cancel, cancelled) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("horizon-bedrock-net".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(async {
                            let result = tokio::select! {
                                biased;
                                _ = cancelled => Err(io::Error::new(
                                    io::ErrorKind::Interrupted, "Bedrock session cancelled")),
                                result = run(request, receiver, &sender) => result,
                            };
                            if let Err(error) = result {
                                // A host that stopped polling must not retain
                                // this worker just to publish a terminal notice.
                                // Closed poll results remain the fallback.
                                let _ = timeout(
                                    Duration::from_secs(2),
                                    sender.send(Event::Disconnected(error)),
                                )
                                .await;
                            }
                        });
                        runtime.shutdown_timeout(Duration::from_secs(2));
                    }
                    Err(error) => {
                        let _ = sender.blocking_send(Event::Disconnected(error));
                    }
                }
            })?;
        Ok(Self {
            commands,
            events,
            cancel: Some(cancel),
            thread: Some(thread),
        })
    }

    pub fn poll(&mut self) -> Result<Event, mpsc::error::TryRecvError> {
        self.events.try_recv()
    }

    /// Worker completion is independent of whether the host drains events.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Requests one frame. Repeated requests while a read is pending coalesce;
    /// bounded queues apply backpressure to a slow host.
    pub fn next_frame(&self) -> io::Result<()> {
        self.command(Command::NextFrame)
    }

    /// Call only after the current connection's actual world presentation is
    /// ready. Input arrival or NetworkReady alone must not trigger this method.
    pub fn presented(&self) -> io::Result<()> {
        self.command(Command::Presented)
    }

    /// Native play packets are admitted only after the presentation transition.
    pub fn send(&self, packet: packets::Packet) -> io::Result<()> {
        self.command(Command::Send(Box::new(packet)))
    }

    fn command(&self, command: Command) -> io::Result<()> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "Bedrock command queue is full")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    io::Error::new(io::ErrorKind::NotConnected, "Bedrock session closed")
                }
            })
    }

    /// Nonblocking for the game thread; cancellation drops the original owner
    /// even during dial, startup, a pending receive or event backpressure.
    pub fn disconnect(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "Bedrock host closed")
}
async fn emit(sender: &mpsc::Sender<Event>, event: Event) -> io::Result<()> {
    sender.send(event).await.map_err(|_| closed())
}
async fn run(
    request: Request,
    mut commands: mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
) -> io::Result<()> {
    let deadline = Instant::from_std(request.deadline);
    let mut observe = |_: bedrock_client_tokio::handshake::Observation<'_>| {};
    let login = connect_offline_http(
        request.http,
        request.key,
        request.profile,
        deadline,
        request.metadata,
        &mut observe,
    )
    .await?;
    let startup = match startup::begin(login, request.radius, &mut observe).await? {
        Setup::Startup(startup) => startup,
        Setup::AwaitingPacks(offer) => {
            offer.close().await;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Bedrock resource packs require host admission and application",
            ));
        }
    };
    let mut ready: NetworkReady = startup.wait(&mut observe).await?;
    timeout_at(
        deadline,
        emit(
            events,
            Event::NetworkReady(Box::new(StartupData {
                start_game: ready.startup().start_game.clone(),
                item_registry: ready.startup().item_registry.clone(),
            })),
        ),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Bedrock setup deadline elapsed"))??;
    let mut receiving = false;
    enum Input {
        Command(Option<Command>),
        Frame(io::Result<Vec<u8>>),
    }
    let initialized = loop {
        let input = timeout_at(deadline, async {
            tokio::select! {
                biased;
                command = commands.recv() => Input::Command(command),
                frame = ready.receive_frame(), if receiving => Input::Frame(frame),
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Bedrock setup deadline elapsed"))?;
        match input {
            Input::Command(Some(Command::NextFrame)) => receiving = true,
            Input::Frame(frame) => {
                receiving = false;
                timeout_at(deadline, emit(events, Event::Frame(frame?)))
                    .await
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::TimedOut, "Bedrock setup deadline elapsed")
                    })??;
            }
            Input::Command(Some(Command::Presented)) => {
                break ready.finish_loading(async { Ok(()) }, &mut observe).await?;
            }
            Input::Command(Some(Command::Send(_))) => {
                ready.close().await;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Bedrock play precedes host presentation",
                ));
            }
            Input::Command(None) => return Err(closed()),
        }
    };
    let InitializedConnection { mut connection, .. } = initialized;
    let result = async {
        emit(events, Event::Initialized).await?;
        loop {
            let input = tokio::select! {
                biased;
                command = commands.recv() => Input::Command(command),
                frame = connection.next_raw(), if receiving => Input::Frame(frame.map(|frame| frame.as_ref().to_vec())),
            };
            match input {
                Input::Command(Some(Command::NextFrame)) => receiving = true,
                Input::Frame(frame) => {
                    receiving = false;
                    connection.finish_current();
                    emit(events, Event::Frame(frame?)).await?;
                }
                Input::Command(Some(Command::Send(packet))) => {
                    connection
                        .send_packet(&packet, wire::Peer::Client, 0, 0)
                        .await?;
                }
                Input::Command(Some(Command::Presented)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Bedrock presentation already acknowledged",
                    ));
                }
                Input::Command(None) => return Ok(()),
            }
        }
    }
    .await;
    retire(&mut connection, Duration::from_secs(2)).await;
    result
}

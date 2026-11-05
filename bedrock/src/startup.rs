// SPDX-License-Identifier: MIT
// Uses pinned Cinnabar MIT client lifecycle APIs; their full notices are retained.

//! One original connection through explicit pack admission and network startup.
//! Network readiness is separate from host presentation and protocol initialization.
use crate::negotiation::{LoginBoundary, retire};
use bedrock_client::{
    handshake::{ClientPackOffer, ClientPacks, PacksEvent},
    spawn::{SpawnNegotiation, SpawnReady, StartupData},
};
use bedrock_client_tokio::{
    connection::Connection,
    handshake::{self, Observation},
    packs::{self, AcquiredPacks, InBand, PackIo},
};
use bedrock_protocol::{
    enums::PlayStatusType,
    packets::{self, PacketId},
    types::PackInfoData,
    wire,
};
use std::{future::Future, io, time::Duration};
use tokio::time::{Instant, timeout_at};

struct Owner {
    connection: Connection,
    deadline: Instant,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn expired() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Bedrock setup deadline elapsed")
}
fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(expired())
    } else {
        Ok(())
    }
}
async fn finish<T>(
    owner: &mut Owner,
    result: Result<io::Result<T>, tokio::time::error::Elapsed>,
) -> io::Result<T> {
    let result = result.unwrap_or_else(|_| Err(expired())).and_then(|value| {
        check_deadline(owner.deadline)?;
        Ok(value)
    });
    if result.is_err() {
        retire(&mut owner.connection, Duration::from_secs(2)).await;
    }
    result
}
fn header(raw: &impl AsRef<[u8]>) -> io::Result<wire::PacketHeader> {
    let mut reader = wire::Reader::new(raw.as_ref());
    let header = wire::PacketHeader::from_raw(reader.read_var_u32().map_err(io::Error::other)?);
    if header.sender_subclient != 0
        || header.target_subclient != 0
        || PacketId::from_raw(header.id)
            .is_some_and(|id| !id.direction().permits(wire::Peer::Server))
    {
        return Err(invalid("unsupported Bedrock startup direction/subclient"));
    }
    Ok(header)
}
async fn send(
    connection: &mut Connection,
    packet: packets::Packet,
    deadline: Instant,
    observe: &mut (impl FnMut(Observation<'_>) + Send),
) -> io::Result<()> {
    check_deadline(deadline)?;
    timeout_at(
        deadline,
        connection.send_packet(&packet, wire::Peer::Client, 0, 0),
    )
    .await
    .map_err(|_| expired())??;
    check_deadline(deadline)?;
    observe(Observation::Sent(std::slice::from_ref(&packet)));
    Ok(())
}

// Adapted from Cinnabar's MIT bedrock-client-tokio/src/packs.rs at 9126e2b.
// Retain raw unknown frames, which the upstream decoded adapter skips. The
// pinned protocol owners still validate every transfer and key continuation.
async fn acquire(
    owner: &mut Owner,
    mut offer: ClientPackOffer,
    io: &mut impl PackIo,
    observe: &mut (impl FnMut(Observation<'_>) + Send),
) -> io::Result<AcquiredPacks> {
    check_deadline(owner.deadline)?;
    let offers = offer.offers().cloned().collect::<Vec<_>>();
    let preloads = packs::wait_io(&mut owner.connection, io.preload(&offers)).await;
    check_deadline(owner.deadline)?;
    if preloads.len() != offers.len() {
        return Err(invalid("pack preload count differs from original offers"));
    }
    for (original, archive) in offers.iter().zip(preloads) {
        if let Some(archive) = archive {
            offer
                .supply_preloaded(original.pack_id_version.pack_uuid, archive)
                .map_err(io::Error::other)?;
        }
    }
    let mut waiting = offers
        .iter()
        .map(|p| p.pack_id_version.pack_uuid)
        .collect::<Vec<_>>();
    let (mut negotiation, response) = offer.start().map_err(io::Error::other)?;
    notify_completed(&mut waiting, &negotiation, io);
    send(
        &mut owner.connection,
        response.into(),
        owner.deadline,
        observe,
    )
    .await?;
    owner.connection.replay_deferred();
    let mut deferred_stack = false;
    loop {
        check_deadline(owner.deadline)?;
        if deferred_stack && negotiation.ready_for_stack() {
            packs::wait_io(&mut owner.connection, io.flush()).await;
            owner.connection.replay_deferred();
            deferred_stack = false;
        }
        let raw = owner.connection.next_raw().await?;
        check_deadline(owner.deadline)?;
        let header = header(&raw)?;
        let Some(id) = PacketId::from_raw(header.id) else {
            observe(Observation::Unknown(header));
            owner.connection.defer_current()?;
            continue;
        };
        let mut reader = wire::Reader::from_shared(&raw);
        reader.read_var_u32().map_err(io::Error::other)?;
        let packet = owner
            .connection
            .decode_body(id, &raw.slice(reader.position()..))?;
        observe(Observation::Received(header, &packet));
        if matches!(
            packet,
            packets::Packet::Disconnect(_) | packets::Packet::Transfer(_)
        ) {
            return Err(invalid(
                "server ended Bedrock pack negotiation; transfer is not followed",
            ));
        }
        if matches!(packet, packets::Packet::ResourcePackStack(_)) {
            packs::wait_io(&mut owner.connection, io.flush()).await;
        }
        match negotiation.receive(packet).map_err(io::Error::other)? {
            PacksEvent::Send { packs, packets } => {
                negotiation = packs;
                notify_completed(&mut waiting, &negotiation, io);
                for packet in packets {
                    send(&mut owner.connection, packet, owner.deadline, observe).await?;
                }
            }
            PacksEvent::Finished(complete) => {
                let (response, handoff, metadata) = complete.into_parts_with_metadata();
                send(
                    &mut owner.connection,
                    response.into(),
                    owner.deadline,
                    observe,
                )
                .await?;
                return Ok(AcquiredPacks {
                    handoff,
                    metadata: metadata.map(Box::new),
                });
            }
            PacksEvent::KeyExchange(exchange) => {
                let (continuation, response) = exchange.into_parts();
                send(
                    &mut owner.connection,
                    response.into(),
                    owner.deadline,
                    observe,
                )
                .await?;
                negotiation = continuation;
            }
            PacksEvent::DeferredStack { packs, stack } => {
                drop(stack);
                owner.connection.defer_current()?;
                deferred_stack = true;
                negotiation = packs;
            }
            PacksEvent::Unhandled { packs, packet } => {
                drop(packet);
                owner.connection.defer_current()?;
                negotiation = packs;
            }
        }
    }
}
fn notify_completed(waiting: &mut Vec<uuid::Uuid>, packs: &ClientPacks, io: &mut impl PackIo) {
    waiting.retain(|id| {
        if let Some(archive) = packs.completed_archive(*id) {
            io.completed(*id, archive);
            false
        } else {
            true
        }
    });
}

/// Nonempty offers cannot start acquisition until the caller supplies a decision.
#[must_use]
pub enum Setup {
    AwaitingPacks(Box<PackDecision>),
    Startup(Box<Startup>),
}
#[must_use]
pub struct PackDecision {
    owner: Owner,
    offer: ClientPackOffer,
    spawn: SpawnNegotiation,
}
#[must_use]
pub struct Startup {
    owner: Owner,
    spawn: SpawnNegotiation,
    acquired: Option<AcquiredPacks>,
}
#[must_use]
pub struct NetworkReady {
    owner: Owner,
    ready: SpawnReady,
    startup: StartupData,
    acquired: Option<AcquiredPacks>,
}

/// The protocol initialization send completed. This does not assert that a
/// Horizon world adapter, rendering, inventory or account flow is implemented.
#[must_use]
pub struct InitializedConnection {
    pub connection: Connection,
    pub startup: StartupData,
    pub acquired: Option<AcquiredPacks>,
}
impl InitializedConnection {
    pub async fn close(mut self) {
        retire(&mut self.connection, Duration::from_secs(2)).await;
    }
}

/// Retains the first Login deadline; phase changes cannot silently extend it.
/// An empty offer still performs the real stack exchange and acknowledgement.
pub async fn begin(
    boundary: LoginBoundary,
    radius: packets::RequestChunkRadius,
    observe: &mut (impl FnMut(Observation<'_>) + Send),
) -> io::Result<Setup> {
    let LoginBoundary {
        connection,
        packs,
        setup_deadline,
    } = boundary;
    let mut owner = Owner {
        connection,
        deadline: setup_deadline,
    };
    let result = (|| {
        check_deadline(owner.deadline)?;
        SpawnNegotiation::new(radius).map_err(io::Error::other)
    })();
    let spawn = match result {
        Ok(spawn) => spawn,
        Err(error) => {
            retire(&mut owner.connection, Duration::from_secs(2)).await;
            return Err(error);
        }
    };
    match packs {
        Some(offer) if offer.offers().next().is_some() => {
            Ok(Setup::AwaitingPacks(Box::new(PackDecision {
                owner,
                offer,
                spawn,
            })))
        }
        Some(offer) => {
            let result = timeout_at(
                owner.deadline,
                acquire(&mut owner, offer, &mut InBand, observe),
            )
            .await;
            let acquired = finish(&mut owner, result).await?;
            owner.connection.replay_deferred();
            Ok(Setup::Startup(Box::new(Startup {
                owner,
                spawn,
                acquired: Some(acquired),
            })))
        }
        None => {
            owner.connection.replay_deferred();
            Ok(Setup::Startup(Box::new(Startup {
                owner,
                spawn,
                acquired: None,
            })))
        }
    }
}

impl PackDecision {
    pub fn offers(&self) -> impl Iterator<Item = &PackInfoData> {
        self.offer.offers()
    }
    pub async fn close(mut self) {
        retire(&mut self.owner.connection, Duration::from_secs(2)).await;
    }

    /// The supplied future is the host's explicit decision for these exact offers.
    /// While it waits, late verified key exchanges are acknowledged on the same
    /// connection. No preload or download starts before acceptance. Canceling
    /// this consuming future drops the original connection; it cannot reconnect.
    pub async fn acquire_after_decision(
        mut self,
        io: &mut impl PackIo,
        decision: impl Future<Output = bool>,
        observe: &mut (impl FnMut(Observation<'_>) + Send),
    ) -> io::Result<Startup> {
        let mut offer = Some(self.offer);
        let deadline = self.owner.deadline;
        let operation = async {
            check_deadline(self.owner.deadline)?;
            tokio::pin!(decision);
            loop {
                let admitted = tokio::select! {
                    biased;
                    answer=&mut decision=>Some(answer),
                    frame=self.owner.connection.next_raw()=> {
                        let raw=frame?;
                        let header=header(&raw)?;
                        if PacketId::from_raw(header.id).is_none() {
                            observe(Observation::Unknown(header));
                            self.owner.connection.defer_current()?;
                        } else {
                            self.owner.connection.replay_current();
                            offer=Some(handshake::offer_control(&mut self.owner.connection,offer.take().expect("owned continuation"),observe).await.map_err(io::Error::other)?);
                        }
                        None
                    },
                };
                if let Some(answer) = admitted {
                    if !answer {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Bedrock resource packs declined",
                        ));
                    }
                    break;
                }
                check_deadline(self.owner.deadline)?;
            }
            acquire(
                &mut self.owner,
                offer.take().expect("owned continuation"),
                io,
                observe,
            )
            .await
        };
        let result = timeout_at(deadline, operation).await;
        let acquired = finish(&mut self.owner, result).await?;
        self.owner.connection.replay_deferred();
        Ok(Startup {
            owner: self.owner,
            spawn: self.spawn,
            acquired: Some(acquired),
        })
    }
}

impl Startup {
    pub async fn close(mut self) {
        retire(&mut self.owner.connection, Duration::from_secs(2)).await;
    }

    /// Preserves every unrelated original frame once in the connection's bounded
    /// FIFO. Replay happens at phase handoff, never inside this receive loop.
    pub async fn wait(
        mut self,
        observe: &mut (impl FnMut(Observation<'_>) + Send),
    ) -> io::Result<NetworkReady> {
        let operation = async {
            check_deadline(self.owner.deadline)?;
            let mut startup = None;
            loop {
                check_deadline(self.owner.deadline)?;
                let raw = self.owner.connection.next_raw().await?;
                check_deadline(self.owner.deadline)?;
                let mut reader = wire::Reader::from_shared(&raw);
                let header = header(&raw)?;
                reader.read_var_u32().map_err(io::Error::other)?;
                let Some(id) = PacketId::from_raw(header.id) else {
                    observe(Observation::Unknown(header));
                    self.owner.connection.defer_current()?;
                    continue;
                };
                match id {
                    PacketId::Disconnect => {
                        return Err(invalid("server disconnected during Bedrock startup"));
                    }
                    PacketId::Transfer => {
                        return Err(invalid(
                            "Bedrock transfer requires a separately approved connection",
                        ));
                    }
                    PacketId::ServerToClientHandshake
                    | PacketId::NetworkSettings
                    | PacketId::ResourcePacksInfo
                    | PacketId::ResourcePackStack
                    | PacketId::ResourcePackDataInfo
                    | PacketId::ResourcePackChunkData => {
                        return Err(invalid(
                            "unexpected negotiation control after pack completion",
                        ));
                    }
                    PacketId::StartGame
                    | PacketId::ItemRegistry
                    | PacketId::PlayStatus
                    | PacketId::ChunkRadiusUpdated => {}
                    _ => {
                        self.spawn.receive_deferred(id).map_err(io::Error::other)?;
                        self.owner.connection.defer_current()?;
                        continue;
                    }
                }
                let packet = self
                    .owner
                    .connection
                    .decode_body(id, &raw.slice(reader.position()..))?;
                observe(Observation::Received(header, &packet));
                if let packets::Packet::PlayStatus(status) = &packet
                    && status.status != PlayStatusType::PlayerSpawn
                    && status.status != PlayStatusType::LoginSuccess
                {
                    return Err(invalid("server rejected Bedrock startup"));
                }
                let update = self.spawn.receive(packet).map_err(io::Error::other)?;
                if let Some(request) = update.request_radius {
                    check_deadline(self.owner.deadline)?;
                    send(
                        &mut self.owner.connection,
                        request.into(),
                        self.owner.deadline,
                        observe,
                    )
                    .await?;
                }
                if let Some(data) = update.startup {
                    startup = Some(data);
                }
                if update.deferred.is_some() {
                    self.owner.connection.defer_current()?;
                } else {
                    self.owner.connection.finish_current();
                }
                if let Some(ready) = update.ready {
                    return Ok((
                        ready,
                        startup
                            .take()
                            .ok_or_else(|| invalid("startup definitions were not retained"))?,
                    ));
                }
                check_deadline(self.owner.deadline)?;
            }
        };
        let result = timeout_at(self.owner.deadline, operation).await;
        let (ready, startup) = finish(&mut self.owner, result).await?;
        self.owner.connection.replay_deferred();
        Ok(NetworkReady {
            owner: self.owner,
            ready,
            startup,
            acquired: self.acquired,
        })
    }
}

impl NetworkReady {
    pub fn startup(&self) -> &StartupData {
        &self.startup
    }
    pub fn acquired(&self) -> Option<&AcquiredPacks> {
        self.acquired.as_ref()
    }
    /// Input arrival alone is not evidence of decoded or rendered terrain.
    pub fn terrain_received(&self) -> bool {
        self.ready.terrain_received()
    }
    pub async fn close(mut self) {
        retire(&mut self.owner.connection, Duration::from_secs(2)).await;
    }

    /// Gives the host one original header/body without skipping unknown IDs or
    /// granting initialization. The host owns decoding/terminal/world policy.
    pub async fn receive_raw(mut self) -> io::Result<(Self, impl AsRef<[u8]> + Send + 'static)> {
        let result = timeout_at(self.owner.deadline, async {
            check_deadline(self.owner.deadline)?;
            self.owner.connection.next_raw().await
        })
        .await;
        let raw = finish(&mut self.owner, result).await?;
        self.owner.connection.finish_current();
        Ok((self, raw))
    }

    // The native session actor keeps this owner alive while selecting host
    // commands beside a pending receive. next_raw only removes a frame after
    // its final await; cancelling a pending read retains every queued sibling.
    pub(crate) async fn receive_frame(&mut self) -> io::Result<Vec<u8>> {
        let result = timeout_at(self.owner.deadline, async {
            check_deadline(self.owner.deadline)?;
            self.owner.connection.next_raw().await
        })
        .await;
        let raw = finish(&mut self.owner, result).await?;
        self.owner.connection.finish_current();
        Ok(raw.as_ref().to_vec())
    }

    /// The host must complete this future only when its actual presentation is
    /// ready. Network readiness alone cannot send SetLocalPlayerAsInitialized.
    /// Failure/cancellation retains no reusable initialization capability.
    pub async fn finish_loading(
        mut self,
        presentation: impl Future<Output = io::Result<()>>,
        observe: &mut (impl FnMut(Observation<'_>) + Send),
    ) -> io::Result<InitializedConnection> {
        let operation = async {
            check_deadline(self.owner.deadline)?;
            packs::wait_io(&mut self.owner.connection, presentation).await?;
            check_deadline(self.owner.deadline)?;
            send(
                &mut self.owner.connection,
                self.ready.finish_loading().into(),
                self.owner.deadline,
                observe,
            )
            .await?;
            Ok(())
        };
        let result = timeout_at(self.owner.deadline, operation).await;
        finish(&mut self.owner, result).await?;
        Ok(InitializedConnection {
            connection: self.owner.connection,
            startup: self.startup,
            acquired: self.acquired,
        })
    }
}

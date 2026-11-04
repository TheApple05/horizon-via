// SPDX-License-Identifier: MIT
// Uses Cinnabar's MIT signed association helper and server continuation.
//! Synthetic protocol effects only; no game, account, assets or world.
use bedrock_client::server::{
    OfflinePolicy, ServerHandshake, ServerHandshakeOptions, ServerLoginEvent, ServerSettingsEvent,
};
use bedrock_client_tokio::{
    handshake::Observation,
    packs::PackIo,
    server::{AcceptingConnection, test_support::signed_pair_with_key},
    transport::Transport,
};
use bedrock_protocol::{enums::PlayStatusType, packets, types, wire};
use horizon_bedrock::{
    negotiation::{OfflineProfile, login_offline},
    startup::{self, Setup},
};
use p384::{ecdsa::SigningKey, elliptic_curve::Generate};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{
    convert::Infallible,
    io,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::oneshot,
    time::{Instant, timeout},
};

fn profile() -> OfflineProfile {
    OfflineProfile {
        display_name: "HorizonFixture".into(),
        uuid: "00000000-0000-4000-8000-000000000047".into(),
        now: SystemTime::now(),
    }
}
fn metadata() -> io::Result<Box<RawValue>> {
    RawValue::from_string(r#"{"DeviceOS":1,"GameVersion":"1.26.60","LanguageCode":"en_US","SkinId":"synthetic","ThirdPartyName":"HorizonFixture","Nonce":"synthetic-nonce","ServerAddress":"127.0.0.1:19132"}"#.into()).map_err(io::Error::other)
}
fn radius() -> packets::RequestChunkRadius {
    packets::RequestChunkRadius {
        chunk_radius: wire::ZigZag32(8),
        ..Default::default()
    }
}
fn start() -> packets::Packet {
    packets::Packet::StartGame(Box::new(packets::StartGame {
        runtime_id: types::ActorRuntimeID(73),
        ..Default::default()
    }))
}
fn time(value: i32) -> packets::Packet {
    packets::SetTime {
        time: wire::ZigZag32(value),
    }
    .into()
}
async fn send(wire: &mut AcceptingConnection, packets: Vec<packets::Packet>, deadline: Instant) {
    let mut pending = wire.prepare_packets(packets, deadline.into_std()).unwrap();
    while let Some(capacity) = pending.next().await.unwrap() {
        capacity.commit().unwrap();
    }
    pending.finish().unwrap();
}
async fn admit(
    wire: &mut AcceptingConnection,
    server: ServerHandshake<bedrock_net::login::OfflineLogin>,
    deadline: Instant,
    handoff: packets::Packet,
) {
    let policy = OfflinePolicy::new(|_| panic!("literal synthetic address must not resolve DNS"));
    let request = wire.receive(deadline.into_std()).await.unwrap();
    let ServerSettingsEvent::Settings(settings) = server
        .receive(
            request,
            &policy,
            Instant::now().into_std(),
            SystemTime::now(),
        )
        .unwrap()
    else {
        panic!("settings not admitted")
    };
    let compression = settings.compression();
    send(
        wire,
        vec![time(11), settings.packet().clone().into()],
        deadline,
    )
    .await;
    wire.compression_flushed(compression).unwrap();
    let login = settings.flushed(Instant::now().into_std()).unwrap();
    let request = wire.receive(deadline.into_std()).await.unwrap();
    let ServerLoginEvent::Verified(verified) = login
        .receive(
            request,
            &policy,
            Instant::now().into_std(),
            SystemTime::now(),
        )
        .unwrap()
    else {
        panic!("Login not verified")
    };
    let approved = verified.authorize(|_| Ok::<_, Infallible>(())).unwrap();
    let ready = approved
        .accept(&policy, Instant::now().into_std(), SystemTime::now())
        .unwrap();
    let _accepted = ready
        .finish(&policy, Instant::now().into_std(), SystemTime::now())
        .unwrap();
    send(
        wire,
        vec![
            packets::PlayStatus {
                status: PlayStatusType::LoginSuccess,
            }
            .into(),
            handoff,
            time(22),
        ],
        deadline,
    )
    .await;
    let response = wire.receive(deadline.into_std()).await.unwrap();
    let packets::Packet::ClientCacheStatus(status) = response else {
        panic!("cache status missing; got {:?}", response.id())
    };
    assert!(
        !status.is_cache_supported,
        "fixture must not advertise a blob cache"
    );
}

#[tokio::test]
async fn packless_and_empty_offer_preserve_fifo_and_gate_the_single_initialization() {
    ready_case(true, false).await;
    ready_case(false, false).await;
}
#[tokio::test]
async fn presentation_failure_closes_without_initialization() {
    ready_case(true, true).await;
}

async fn ready_case(packless: bool, presentation_fails: bool) {
    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let (pair, original) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        let deadline = Instant::now() + Duration::from_secs(15);
        let server = ServerHandshake::new_offline(
            original.key().clone(),
            Some(*key.verifying_key()),
            ServerHandshakeOptions::default(),
            deadline.into_std(),
            Instant::now().into_std(),
        )
        .unwrap();
        drop(original);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let (presentation_tx, presentation_rx) = oneshot::channel();
        let mut sent = Vec::new();
        let mut observe = |event: Observation<'_>| {
            if let Observation::Sent(packets) = event {
                sent.extend(packets.iter().map(|p| p.id()));
            }
        };
        let unknown = vec![0xff, 0x07, 0x42, 0x0b, 0xab];
        assert!(packets::PacketId::from_raw(1023).is_none());
        let client_task = async {
            let login = login_offline(
                Transport::from_http(client),
                key,
                profile(),
                deadline,
                |_| metadata(),
                &mut observe,
            )
            .await
            .unwrap();
            let Setup::Startup(startup) =
                startup::begin(login, radius(), &mut observe).await.unwrap()
            else {
                panic!("empty/packless join required consent")
            };
            let mut ready = startup.wait(&mut observe).await.unwrap();
            assert_eq!(ready.startup().start_game.runtime_id.0, 73);
            assert!(ready.startup().item_registry.item_data.is_empty());
            assert_eq!(ready.acquired().is_none(), packless);
            for expected in [11, 22] {
                let (next, raw) = ready.receive_raw().await.unwrap();
                ready = next;
                assert_time(raw.as_ref(), expected);
            }
            let (next, raw) = ready.receive_raw().await.unwrap();
            ready = next;
            assert_eq!(
                raw.as_ref(),
                unknown.as_slice(),
                "unknown original frame changed/lost across packs/startup"
            );
            for expected in [33, 44] {
                let (next, raw) = ready.receive_raw().await.unwrap();
                ready = next;
                assert_time(raw.as_ref(), expected);
            }
            ready
                .finish_loading(
                    async {
                        presentation_rx.await.map_err(|_| {
                            io::Error::other("synthetic presentation channel closed")
                        })?;
                        if presentation_fails {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "synthetic presentation failed",
                            ))
                        } else {
                            Ok(())
                        }
                    },
                    &mut observe,
                )
                .await
        };
        let server_task = async {
            admit(
                &mut wire,
                server,
                deadline,
                if packless {
                    start()
                } else {
                    packets::ResourcePacksInfo::default().into()
                },
            )
            .await;
            let mut pending = wire
                .prepare_raw(unknown.clone().into(), deadline.into_std())
                .unwrap();
            while let Some(capacity) = pending.next().await.unwrap() {
                capacity.commit().unwrap();
            }
            pending.finish().unwrap();
            if !packless {
                let packets::Packet::ResourcePackClientResponse(response) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("empty offer was not acknowledged")
                };
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::DownloadingFinished { .. }
                ));
                send(
                    &mut wire,
                    vec![packets::ResourcePackStack::default().into()],
                    deadline,
                )
                .await;
                let packets::Packet::ResourcePackClientResponse(response) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("stack was not acknowledged")
                };
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::ResourcePackStackFinished { .. }
                ));
            }
            let mut packets = vec![
                packets::PlayStatus {
                    status: PlayStatusType::PlayerSpawn,
                }
                .into(),
                time(33),
                packets::ItemRegistry::default().into(),
                time(44),
            ];
            if !packless {
                packets.insert(0, start());
            }
            send(&mut wire, packets, deadline).await;
            let packets::Packet::RequestChunkRadius(request) =
                wire.receive(deadline.into_std()).await.unwrap()
            else {
                panic!("radius request missing")
            };
            assert_eq!(request.chunk_radius.0, 8);
            assert!(
                timeout(Duration::from_millis(80), wire.receive(deadline.into_std()))
                    .await
                    .is_err(),
                "initialization preceded actual presentation"
            );
            presentation_tx.send(()).unwrap();
            if presentation_fails {
                assert_eq!(
                    wire.receive(deadline.into_std())
                        .await
                        .err()
                        .unwrap()
                        .kind(),
                    io::ErrorKind::UnexpectedEof
                );
            } else {
                let packets::Packet::SetLocalPlayerAsInitialized(initialized) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("initialization missing")
                };
                assert_eq!(initialized.player_id.0, 73);
                assert!(
                    timeout(Duration::from_millis(80), wire.receive(deadline.into_std()))
                        .await
                        .is_err(),
                    "duplicate initialization/radius"
                );
            }
        };
        let (result, ()) = tokio::join!(client_task, server_task);
        let initialized = if presentation_fails {
            match result {
                Err(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
                Ok(_) => panic!("failed presentation initialized"),
            };
            None
        } else {
            Some(result.unwrap())
        };
        assert_eq!(
            sent.iter()
                .filter(|&&p| p == packets::PacketId::RequestChunkRadius)
                .count(),
            1
        );
        assert_eq!(
            sent.iter()
                .filter(|&&p| p == packets::PacketId::SetLocalPlayerAsInitialized)
                .count(),
            usize::from(!presentation_fails)
        );
        assert_eq!(
            sent.iter()
                .filter(|&&p| p == packets::PacketId::ResourcePackClientResponse)
                .count(),
            if packless { 0 } else { 2 }
        );
        let mut wire = wire.begin_close();
        let ((), closed) = tokio::join!(
            async {
                if let Some(initialized) = initialized {
                    initialized.close().await;
                }
            },
            wire.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
        );
        closed.unwrap();
        drop(wire);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("startup fixture exceeded overall deadline");
}
fn assert_time(raw: &[u8], expected: i32) {
    let mut reader = wire::Reader::new(raw);
    let header = wire::PacketHeader::from_raw(reader.read_var_u32().unwrap());
    assert_eq!(header.id, packets::PacketId::SetTime as u32);
    let packets::Packet::SetTime(time) =
        packets::Packet::decode_body(packets::PacketId::SetTime, &mut reader).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(time.time.0, expected);
    reader.expect_consumed().unwrap();
}

struct NoPackIo;
impl PackIo for NoPackIo {
    async fn preload(&mut self, _: &[types::PackInfoData]) -> Vec<Option<Vec<u8>>> {
        panic!("preload before explicit acceptance")
    }
    fn completed(&mut self, _: uuid::Uuid, _: Arc<Vec<u8>>) {
        panic!("archive completed before acceptance")
    }
    async fn flush(&mut self) {
        panic!("flush before acceptance")
    }
}
#[tokio::test]
async fn declined_nonempty_offer_performs_no_acquisition_and_retires_original_peer() {
    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let (pair, original) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        let deadline = Instant::now() + Duration::from_secs(10);
        let server = ServerHandshake::new_offline(
            original.key().clone(),
            Some(*key.verifying_key()),
            ServerHandshakeOptions::default(),
            deadline.into_std(),
            Instant::now().into_std(),
        )
        .unwrap();
        drop(original);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let client_task = async {
            let login = login_offline(
                Transport::from_http(client),
                key,
                profile(),
                deadline,
                |_| metadata(),
                &mut |_| {},
            )
            .await
            .unwrap();
            let Setup::AwaitingPacks(offer) =
                startup::begin(login, radius(), &mut |_| {}).await.unwrap()
            else {
                panic!("nonempty offer bypassed admission")
            };
            assert_eq!(offer.offers().count(), 1);
            match offer
                .acquire_after_decision(&mut NoPackIo, async { false }, &mut |_| {})
                .await
            {
                Err(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
                Ok(_) => panic!("declined packs started"),
            }
        };
        let server_task = async {
            admit(
                &mut wire,
                server,
                deadline,
                packets::ResourcePacksInfo {
                    resource_packs: vec![types::PackInfoData::default()],
                    ..Default::default()
                }
                .into(),
            )
            .await;
            assert_eq!(
                wire.receive(deadline.into_std())
                    .await
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::UnexpectedEof
            );
        };
        tokio::join!(client_task, server_task);
        let mut wire = wire.begin_close();
        wire.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
            .await
            .unwrap();
        drop(wire);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("pack decline fixture exceeded overall deadline");
}

#[tokio::test]
async fn startup_timeout_and_cancellation_retire_the_same_original_association() {
    for cancel in [false, true] {
        pending_case(cancel).await;
    }
}
async fn pending_case(cancel: bool) {
    timeout(Duration::from_secs(30),async {
        let key=Arc::new(SigningKey::generate());
        let (pair,original)=signed_pair_with_key(Arc::clone(&key),8*1024*1024).await;
        let deadline=Instant::now()+Duration::from_secs(10);
        let server=ServerHandshake::new_offline(original.key().clone(),Some(*key.verifying_key()),ServerHandshakeOptions::default(),deadline.into_std(),Instant::now().into_std()).unwrap();drop(original);
        let bedrock_client_tokio::server::test_support::SignedPair{mut wire,sender,client,mut listener,signaling}=pair;drop(sender);
        let (radius_tx,radius_rx)=oneshot::channel();
        let client_task=async {
            let cutoff=if cancel {deadline} else {Instant::now()+Duration::from_millis(600)};
            let login=login_offline(Transport::from_http(client),key,profile(),cutoff,|_|metadata(),&mut |_|{}).await.unwrap();
            let Setup::Startup(startup)=startup::begin(login,radius(),&mut |_|{}).await.unwrap() else {panic!("packless required consent")};
            if cancel {
                let mut observe=|_: Observation<'_>|{};
                let mut waiting=Box::pin(startup.wait(&mut observe));
                tokio::select! { _=&mut waiting=>panic!("incomplete startup became ready"),_ = radius_rx=>{} }
                drop(waiting);
            } else {
                let started=Instant::now();
                match startup.wait(&mut |_|{}).await {Err(e)=>assert_eq!(e.kind(),io::ErrorKind::TimedOut),Ok(_)=>panic!("incomplete startup became ready")}
                assert!(started.elapsed()<Duration::from_secs(3));
            }
        };
        let server_task=async {
            admit(&mut wire,server,deadline,start()).await;
            assert!(matches!(wire.receive(deadline.into_std()).await.unwrap(),packets::Packet::RequestChunkRadius(_)));
            let _=radius_tx.send(());
            assert_eq!(wire.receive(deadline.into_std()).await.err().unwrap().kind(),io::ErrorKind::UnexpectedEof);
        };
        tokio::join!(client_task,server_task);
        let mut wire=wire.begin_close();wire.finish_close((Instant::now()+Duration::from_secs(2)).into_std()).await.unwrap();drop(wire);
        listener.close().await.unwrap();drop(listener);Arc::try_unwrap(signaling).ok().expect("signaling owner leaked").shutdown().await;
    }).await.expect("pending startup fixture exceeded overall deadline");
}

struct SyntheticPacks {
    bytes: Vec<u8>,
    preloaded: bool,
    preload_calls: usize,
    completed_calls: usize,
    flush_calls: usize,
    held: Vec<Arc<Vec<u8>>>,
}
impl PackIo for SyntheticPacks {
    async fn preload(&mut self, offers: &[types::PackInfoData]) -> Vec<Option<Vec<u8>>> {
        assert_eq!(offers.len(), 1);
        self.preload_calls += 1;
        tokio::time::sleep(Duration::from_millis(20)).await;
        vec![self.preloaded.then(|| self.bytes.clone())]
    }
    fn completed(&mut self, _: uuid::Uuid, archive: Arc<Vec<u8>>) {
        assert_eq!(archive.as_ref(), &self.bytes);
        self.completed_calls += 1;
        self.held.push(archive);
    }
    async fn flush(&mut self) {
        self.flush_calls += 1;
        self.held.clear();
    }
}

#[tokio::test]
async fn accepted_opaque_pack_keeps_early_stack_unknown_frames_and_exclusive_archive_handoff() {
    for preloaded in [false, true] {
        accepted_pack_case(preloaded).await;
    }
}

async fn accepted_pack_case(preloaded: bool) {
    timeout(Duration::from_secs(30), async {
        // Opaque synthetic bytes exercise ownership/transfer only. They are not
        // a valid game pack and are never parsed, extracted or presented.
        let payload = b"synthetic opaque archive bytes".to_vec();
        let id = uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000089").unwrap();
        let info = types::PackInfoData {
            pack_id_version: types::PackIDVersionData {
                pack_uuid: id,
                pack_version: types::SemVersionData {
                    version: "1.0.0".into(),
                },
            },
            pack_size: wire::U64LE(payload.len() as u64),
            ..Default::default()
        };
        let key = Arc::new(SigningKey::generate());
        let (pair, original) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        let deadline = Instant::now() + Duration::from_secs(15);
        let server = ServerHandshake::new_offline(
            original.key().clone(),
            Some(*key.verifying_key()),
            ServerHandshakeOptions::default(),
            deadline.into_std(),
            Instant::now().into_std(),
        )
        .unwrap();
        drop(original);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let unknown = vec![0xff, 0x07, 0x52, 0x4b];
        let (decision_tx, decision_rx) = oneshot::channel();
        let mut decision_tx = Some(decision_tx);
        let mut observe = |event: Observation<'_>| {
            if matches!(event, Observation::Unknown(_))
                && let Some(tx) = decision_tx.take()
            {
                let _ = tx.send(());
            }
        };
        let client_task = async {
            let login = login_offline(
                Transport::from_http(client),
                key,
                profile(),
                deadline,
                |_| metadata(),
                &mut observe,
            )
            .await
            .unwrap();
            let Setup::AwaitingPacks(offer) =
                startup::begin(login, radius(), &mut observe).await.unwrap()
            else {
                panic!("nonempty offer bypassed decision")
            };
            let mut io = SyntheticPacks {
                bytes: payload.clone(),
                preloaded,
                preload_calls: 0,
                completed_calls: 0,
                flush_calls: 0,
                held: Vec::new(),
            };
            let pending = offer
                .acquire_after_decision(&mut io, async { decision_rx.await.is_ok() }, &mut observe)
                .await
                .unwrap();
            assert_eq!(io.preload_calls, 1);
            assert_eq!(io.completed_calls, 1);
            assert!(io.flush_calls >= 1);
            assert!(io.held.is_empty());
            let mut ready = pending.wait(&mut observe).await.unwrap();
            for expected in [11, 22] {
                let (next, raw) = ready.receive_raw().await.unwrap();
                ready = next;
                assert_time(raw.as_ref(), expected);
            }
            let (next, raw) = ready.receive_raw().await.unwrap();
            ready = next;
            assert_eq!(raw.as_ref(), unknown.as_slice());
            let mut initialized = ready
                .finish_loading(async { Ok(()) }, &mut observe)
                .await
                .unwrap();
            let acquired = initialized.acquired.take().unwrap();
            let archives = acquired.handoff.into_archives();
            assert_eq!(archives.len(), 1);
            assert_eq!(archives[0].pack_id, id);
            assert_eq!(archives[0].archive, payload);
            assert_eq!(archives[0].version, b"1.0.0");
            initialized
        };
        let server_task = async {
            admit(
                &mut wire,
                server,
                deadline,
                packets::ResourcePacksInfo {
                    resource_packs: vec![info],
                    ..Default::default()
                }
                .into(),
            )
            .await;
            let mut pending = wire
                .prepare_raw(unknown.clone().into(), deadline.into_std())
                .unwrap();
            while let Some(capacity) = pending.next().await.unwrap() {
                capacity.commit().unwrap();
            }
            pending.finish().unwrap();
            send(
                &mut wire,
                vec![
                    packets::ResourcePackStack {
                        texture_pack_list: vec![types::PackInstanceId {
                            pack_id: id.to_string().into(),
                            version: "1.0.0".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }
                    .into(),
                ],
                deadline,
            )
            .await;
            let packets::Packet::ResourcePackClientResponse(response) =
                wire.receive(deadline.into_std()).await.unwrap()
            else {
                panic!("initial pack response missing")
            };
            if preloaded {
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::DownloadingFinished { .. }
                ));
            } else {
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::Downloading { .. }
                ));
                send(
                    &mut wire,
                    vec![
                        packets::ResourcePackDataInfo {
                            resource_name: id.to_string().into(),
                            chunk_size: wire::U32LE(64),
                            number_of_chunks: wire::U32LE(1),
                            file_size: wire::U64LE(payload.len() as u64),
                            file_hash: Sha256::digest(&payload).to_vec().into(),
                            ..Default::default()
                        }
                        .into(),
                    ],
                    deadline,
                )
                .await;
                assert!(matches!(
                    wire.receive(deadline.into_std()).await.unwrap(),
                    packets::Packet::ResourcePackChunkRequest(_)
                ));
                send(
                    &mut wire,
                    vec![
                        packets::ResourcePackChunkData {
                            resource_name: id.to_string().into(),
                            chunk_data: payload.clone().into(),
                            ..Default::default()
                        }
                        .into(),
                    ],
                    deadline,
                )
                .await;
                let packets::Packet::ResourcePackClientResponse(response) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("download completion missing")
                };
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::DownloadingFinished { .. }
                ));
            }
            let packets::Packet::ResourcePackClientResponse(response) =
                wire.receive(deadline.into_std()).await.unwrap()
            else {
                panic!("final stack ACK missing")
            };
            assert!(matches!(
                response.response,
                types::ResourcePackClientResponseData::ResourcePackStackFinished { .. }
            ));
            send(
                &mut wire,
                vec![
                    start(),
                    packets::ItemRegistry::default().into(),
                    packets::PlayStatus {
                        status: PlayStatusType::PlayerSpawn,
                    }
                    .into(),
                ],
                deadline,
            )
            .await;
            assert!(matches!(
                wire.receive(deadline.into_std()).await.unwrap(),
                packets::Packet::RequestChunkRadius(_)
            ));
            assert!(matches!(
                wire.receive(deadline.into_std()).await.unwrap(),
                packets::Packet::SetLocalPlayerAsInitialized(_)
            ));
        };
        let (initialized, ()) = tokio::join!(client_task, server_task);
        let mut wire = wire.begin_close();
        let ((), closed) = tokio::join!(
            initialized.close(),
            wire.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
        );
        closed.unwrap();
        drop(wire);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("accepted pack fixture exceeded overall deadline");
}

fn server_exchange_token(key: &SigningKey) -> String {
    let claims = RawValue::from_string("{}".into()).unwrap();
    let identity = bedrock_net::login::OfflineIdentity {
        display_name: "SyntheticServer",
        identity: "00000000-0000-4000-8000-000000000090",
        xuid: "",
        playfab_id: "",
        playfab_title_id: "",
    };
    let request = bedrock_net::login::encode_offline_request(
        key,
        &identity,
        &claims,
        SystemTime::now(),
        16 * 1024,
    )
    .unwrap();
    let length = u32::from_le_bytes(request[..4].try_into().unwrap()) as usize;
    let envelope: serde_json::Value = serde_json::from_slice(&request[4..4 + length]).unwrap();
    let token = envelope["Token"].as_str().unwrap().to_owned();
    bedrock_net::login::verify_server_handshake(&token, 16 * 1024).unwrap();
    token
}
#[tokio::test]
async fn late_verified_exchange_ack_precedes_pack_effects_and_invalid_first_token_retires() {
    for during_acquire in [false, true] {
        for valid in [false, true] {
            late_exchange_case(during_acquire, valid).await;
        }
    }
}
async fn late_exchange_case(during_acquire: bool, valid: bool) {
    timeout(Duration::from_secs(30), async {
        let payload = b"synthetic opaque pack".to_vec();
        let id = uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000090").unwrap();
        let info = types::PackInfoData {
            pack_id_version: types::PackIDVersionData {
                pack_uuid: id,
                pack_version: types::SemVersionData {
                    version: "1.0.0".into(),
                },
            },
            pack_size: wire::U64LE(payload.len() as u64),
            ..Default::default()
        };
        let key = Arc::new(SigningKey::generate());
        let (pair, original) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        let server_key = original.key().clone();
        let deadline = Instant::now() + Duration::from_secs(15);
        let server = ServerHandshake::new_offline(
            server_key.clone(),
            Some(*key.verifying_key()),
            ServerHandshakeOptions::default(),
            deadline.into_std(),
            Instant::now().into_std(),
        )
        .unwrap();
        drop(original);
        let token = if valid {
            server_exchange_token(&server_key)
        } else {
            "invalid-first-token".into()
        };
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let (decision_tx, decision_rx) = oneshot::channel();
        let client_task = async {
            let login = login_offline(
                Transport::from_http(client),
                key,
                profile(),
                deadline,
                |_| metadata(),
                &mut |_| {},
            )
            .await
            .unwrap();
            let Setup::AwaitingPacks(offer) =
                startup::begin(login, radius(), &mut |_| {}).await.unwrap()
            else {
                panic!("nonempty offer bypassed decision")
            };
            let mut io = SyntheticPacks {
                bytes: payload,
                preloaded: true,
                preload_calls: 0,
                completed_calls: 0,
                flush_calls: 0,
                held: Vec::new(),
            };
            let outcome = offer
                .acquire_after_decision(
                    &mut io,
                    async {
                        if during_acquire {
                            true
                        } else {
                            decision_rx.await.is_ok()
                        }
                    },
                    &mut |_| {},
                )
                .await;
            if valid {
                outcome.unwrap().close().await;
                assert_eq!(io.preload_calls, 1);
                assert!(io.held.is_empty());
            } else {
                assert!(outcome.is_err());
                assert_eq!(io.preload_calls, usize::from(during_acquire));
            }
        };
        let server_task = async {
            admit(
                &mut wire,
                server,
                deadline,
                packets::ResourcePacksInfo {
                    resource_packs: vec![info],
                    ..Default::default()
                }
                .into(),
            )
            .await;
            let stack = packets::ResourcePackStack {
                texture_pack_list: vec![types::PackInstanceId {
                    pack_id: id.to_string().into(),
                    version: "1.0.0".into(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            if during_acquire {
                let packets::Packet::ResourcePackClientResponse(response) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("initial acquisition response missing")
                };
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::DownloadingFinished { .. }
                ));
            }
            let mut batch = vec![
                packets::ServerToClientHandshake {
                    handshake_web_token: token.into(),
                }
                .into(),
            ];
            if during_acquire {
                batch.push(stack.clone().into());
            }
            send(&mut wire, batch, deadline).await;
            if valid {
                assert!(
                    matches!(
                        wire.receive(deadline.into_std()).await.unwrap(),
                        packets::Packet::ClientToServerHandshake(_)
                    ),
                    "key ACK did not precede next pack effect"
                );
                if !during_acquire {
                    decision_tx.send(()).unwrap();
                    let packets::Packet::ResourcePackClientResponse(response) =
                        wire.receive(deadline.into_std()).await.unwrap()
                    else {
                        panic!("post-consent pack response missing")
                    };
                    assert!(matches!(
                        response.response,
                        types::ResourcePackClientResponseData::DownloadingFinished { .. }
                    ));
                    send(&mut wire, vec![stack.into()], deadline).await;
                }
                let packets::Packet::ResourcePackClientResponse(response) =
                    wire.receive(deadline.into_std()).await.unwrap()
                else {
                    panic!("single final ACK missing")
                };
                assert!(matches!(
                    response.response,
                    types::ResourcePackClientResponseData::ResourcePackStackFinished { .. }
                ));
            }
            assert_eq!(
                wire.receive(deadline.into_std())
                    .await
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::UnexpectedEof,
                "extra key/pack ACK escaped or original connection was not retired"
            );
        };
        tokio::join!(client_task, server_task);
        let mut wire = wire.begin_close();
        wire.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
            .await
            .unwrap();
        drop(wire);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("late exchange fixture exceeded overall deadline");
}

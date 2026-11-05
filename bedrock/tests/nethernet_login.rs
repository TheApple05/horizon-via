// SPDX-License-Identifier: MIT
// Fixtures use Cinnabar's MIT signed association helper and server continuation.
// Copyright 2026 Cinnabar contributors; full notices retained in ../notices/.

//! Synthetic localhost HTTP/DTLS/SCTP integration. No game, account or service.
use bedrock_client::server::{
    OfflinePolicy, ServerHandshake, ServerHandshakeOptions, ServerLoginEvent, ServerSettingsEvent,
};
use bedrock_client_tokio::{
    handshake::Observation,
    server::{AcceptingConnection, test_support::signed_pair_with_key},
    transport::Transport,
};
use bedrock_protocol::{enums::PlayStatusType, packets, wire};
use horizon_bedrock::negotiation::{OfflineProfile, login_offline};
use p384::{ecdsa::SigningKey, elliptic_curve::Generate};
use serde_json::value::RawValue;
use std::{
    convert::Infallible,
    io,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::time::{Instant, timeout};

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
async fn send(
    wire: &mut AcceptingConnection,
    packets: Vec<packets::Packet>,
    deadline: std::time::Instant,
) {
    let mut pending = wire.prepare_packets(packets, deadline).unwrap();
    while let Some(capacity) = pending.next().await.unwrap() {
        capacity.commit().unwrap();
    }
    pending.finish().unwrap();
}

#[tokio::test]
async fn signed_offline_login_keeps_deferred_and_unread_packets_on_same_connection() {
    check_login_boundary(false).await;
    check_login_boundary(true).await;
}

async fn check_login_boundary(packless: bool) {
    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let (pair, original_server) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        let deadline = Instant::now() + Duration::from_secs(15);
        let server = ServerHandshake::new_offline(
            original_server.key().clone(),
            Some(*key.verifying_key()),
            ServerHandshakeOptions::default(),
            deadline.into_std(),
            Instant::now().into_std(),
        )
        .unwrap();
        drop(original_server);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let mut sent = Vec::new();
        let mut received = Vec::new();
        let mut observe = |event: Observation<'_>| match event {
            Observation::Sent(packets) => sent.extend(packets.iter().map(|p| p.id())),
            Observation::Received(_, packet) => received.push(packet.id()),
            Observation::Unknown(_) => panic!("fixture emitted an unknown packet"),
        };
        let client_task = login_offline(
            Transport::from_http(client),
            key,
            profile(),
            deadline,
            |_| metadata(),
            &mut observe,
        );
        let server_task = async {
            let policy =
                OfflinePolicy::new(|_| panic!("literal localhost metadata must not resolve DNS"));
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
                panic!("settings request not admitted");
            };
            let compression = settings.compression();
            let handoff = if packless {
                packets::Packet::StartGame(Box::new(packets::StartGame {
                    runtime_id: bedrock_protocol::types::ActorRuntimeID(73),
                    ..Default::default()
                }))
            } else {
                packets::ResourcePacksInfo::default().into()
            };
            send(
                &mut wire,
                vec![
                    packets::SetTime {
                        time: wire::ZigZag32(11),
                    }
                    .into(),
                    settings.packet().clone().into(),
                ],
                deadline.into_std(),
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
                panic!("offline Login not verified");
            };
            assert_eq!(verified.login().display_name(), "HorizonFixture");
            assert_eq!(verified.login().nonce(), "synthetic-nonce");
            let approved = verified.authorize(|_| Ok::<_, Infallible>(())).unwrap();
            let ready = approved
                .accept(&policy, Instant::now().into_std(), SystemTime::now())
                .unwrap();
            let _accepted = ready
                .finish(&policy, Instant::now().into_std(), SystemTime::now())
                .unwrap();
            send(
                &mut wire,
                vec![
                    packets::PlayStatus {
                        status: PlayStatusType::LoginSuccess,
                    }
                    .into(),
                    handoff,
                    packets::SetTime {
                        time: wire::ZigZag32(22),
                    }
                    .into(),
                ],
                deadline.into_std(),
            )
            .await;
        };
        let (boundary, ()) = tokio::join!(client_task, server_task);
        let mut boundary = boundary.unwrap();
        assert_eq!(boundary.packs.is_none(), packless);
        assert_eq!(
            sent.iter()
                .filter(|&&id| id == packets::PacketId::RequestNetworkSettings)
                .count(),
            1
        );
        assert_eq!(
            sent.iter()
                .filter(|&&id| id == packets::PacketId::Login)
                .count(),
            1
        );
        assert_eq!(
            received
                .iter()
                .filter(|&&id| id == packets::PacketId::SetTime)
                .count(),
            1
        );
        boundary.connection.replay_deferred();
        for expected in [11, 22] {
            if packless && expected == 22 {
                let (_, start) = timeout(Duration::from_secs(2), boundary.connection.next_packet())
                    .await
                    .unwrap()
                    .unwrap();
                let packets::Packet::StartGame(start) = start else {
                    panic!("packless StartGame was lost or reordered");
                };
                assert_eq!(start.runtime_id.0, 73);
            }
            let (_, packet) = timeout(Duration::from_secs(2), boundary.connection.next_packet())
                .await
                .unwrap()
                .unwrap();
            let packets::Packet::SetTime(time) = packet else {
                panic!("deferred/unread sibling lost or reordered");
            };
            assert_eq!(time.time.0, expected);
        }
        let mut retired_server = wire.begin_close();
        let ((), result) = tokio::join!(
            boundary.close(Duration::from_secs(2)),
            retired_server.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
        );
        result.unwrap();
        drop(retired_server);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("signed fixture exceeded overall deadline");
}

#[tokio::test]
async fn canceling_pending_setup_drops_the_same_association() {
    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let (pair, handshake) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        drop(handshake);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let cutoff = (Instant::now() + Duration::from_secs(5)).into_std();
        {
            let mut observe = |_: Observation<'_>| {};
            let mut setup = Box::pin(login_offline(
                Transport::from_http(client),
                key,
                profile(),
                cutoff.into(),
                |_| metadata(),
                &mut observe,
            ));
            tokio::select! {
                request = wire.receive(cutoff) => {
                    assert!(matches!(request.unwrap(), packets::Packet::RequestNetworkSettings(_)));
                },
                _ = &mut setup => panic!("silent peer unexpectedly completed setup"),
            }
            drop(setup);
        }
        let error = wire.receive_raw(cutoff).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        let mut wire = wire.begin_close();
        wire.finish_close(cutoff).await.unwrap();
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
    .expect("cancellation fixture exceeded overall deadline");
}

#[tokio::test]
async fn silent_settings_peer_times_out_and_is_retired() {
    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let (pair, handshake) = signed_pair_with_key(Arc::clone(&key), 8 * 1024 * 1024).await;
        drop(handshake);
        let bedrock_client_tokio::server::test_support::SignedPair {
            mut wire,
            sender,
            client,
            mut listener,
            signaling,
        } = pair;
        drop(sender);
        let outcome = login_offline(
            Transport::from_http(client),
            key,
            profile(),
            Instant::now() + Duration::from_millis(100),
            |_| metadata(),
            &mut |_| {},
        )
        .await;
        let Err(error) = outcome else {
            panic!("silent settings peer unexpectedly completed Login");
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        // Exactly the original settings request drains; no Login or duplicate follows.
        let cutoff = (Instant::now() + Duration::from_secs(3)).into_std();
        assert!(matches!(
            wire.receive(cutoff).await.unwrap(),
            packets::Packet::RequestNetworkSettings(_)
        ));
        assert_eq!(
            wire.receive_raw(cutoff).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        let mut wire = wire.begin_close();
        wire.finish_close(cutoff).await.unwrap();
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
    .expect("timeout fixture exceeded overall deadline");
}

#[tokio::test]
async fn explicit_http_dial_reuses_proof_address_and_original_packet_queue() {
    use horizon_bedrock::negotiation::{OfflineHttp, connect_offline_http};
    use nethernet_tokio::{
        ClientIceConfig, ConnectionConfig, HttpServerConfig, HttpSignaling, HttpSignalingServer,
        NetherServer, ServerIceConfig, ServerIdentity, TokenTrust,
    };

    timeout(Duration::from_secs(30), async {
        let key = Arc::new(SigningKey::generate());
        let server_key = Arc::new(SigningKey::generate());
        let now = SystemTime::now();
        let signaling = Arc::new(
            HttpSignalingServer::bind("127.0.0.1:0".parse().unwrap(), HttpServerConfig::default())
                .await
                .unwrap(),
        );
        let endpoint = format!("http://{}", signaling.local_addr());
        let mut listener = NetherServer::bind_with(
            Arc::clone(&signaling),
            ConnectionConfig {
                identity: Some(Arc::new(
                    ServerIdentity::from_key(
                        (*server_key).clone(),
                        "horizon-http-loopback",
                        now,
                        Some(Duration::from_secs(30)),
                    )
                    .unwrap(),
                )),
                // Explicit synthetic fixture policy, never account authentication.
                token_trust: Some(TokenTrust::Any),
                server_ice: Some(ServerIceConfig {
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    ice: Default::default(),
                    turn_streams: Vec::new(),
                    turn_tls: None,
                }),
                attempts: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let request = OfflineHttp {
            signaling: HttpSignaling::new("horizon-http-loopback-client".into()).unwrap(),
            endpoint: endpoint.clone(),
            config: ConnectionConfig {
                // The seam must install the original key's identity itself.
                server_trust: Some(TokenTrust::Any),
                client_ice: Some(ClientIceConfig {
                    bind_addr: Some("127.0.0.1:0".parse().unwrap()),
                    ice: Default::default(),
                    turn_streams: Vec::new(),
                    turn_tls: None,
                }),
                attempts: 1,
                ..Default::default()
            },
        };
        let mut sent = Vec::new();
        let mut observe = |event: Observation<'_>| match event {
            Observation::Sent(packets) => sent.extend(packets.iter().map(|packet| packet.id())),
            Observation::Unknown(_) => panic!("loopback peer emitted an unknown packet"),
            Observation::Received(_, _) => {}
        };
        let client_task = connect_offline_http(
            request,
            Arc::clone(&key),
            profile(),
            deadline,
            |_| metadata(), // Intentionally supplies a different ServerAddress.
            &mut observe,
        );
        let server_task = async {
            let accepted = listener.accept().await.unwrap();
            let peer_key = accepted
                .session
                .player()
                .await
                .unwrap()
                .client_public_key()
                .unwrap();
            assert_eq!(peer_key, *key.verifying_key());
            let (mut wire, default_handshake) = AcceptingConnection::from_nether(
                accepted,
                Arc::clone(&server_key),
                ServerHandshakeOptions::default(),
                8 * 1024 * 1024,
                deadline.into_std(),
            )
            .await
            .unwrap();
            drop(default_handshake);
            let server = ServerHandshake::new_offline(
                Arc::clone(&server_key),
                Some(peer_key),
                ServerHandshakeOptions::default(),
                deadline.into_std(),
                Instant::now().into_std(),
            )
            .unwrap();
            let policy = OfflinePolicy::new(|_| panic!("literal loopback must not resolve DNS"));
            let packet = wire.receive(deadline.into_std()).await.unwrap();
            let ServerSettingsEvent::Settings(settings) = server
                .receive(
                    packet,
                    &policy,
                    Instant::now().into_std(),
                    SystemTime::now(),
                )
                .unwrap()
            else {
                panic!("settings request not admitted");
            };
            let compression = settings.compression();
            send(
                &mut wire,
                vec![
                    packets::SetTime {
                        time: wire::ZigZag32(11),
                    }
                    .into(),
                    settings.packet().clone().into(),
                ],
                deadline.into_std(),
            )
            .await;
            wire.compression_flushed(compression).unwrap();
            let login = settings.flushed(Instant::now().into_std()).unwrap();
            let packet = wire.receive(deadline.into_std()).await.unwrap();
            let ServerLoginEvent::Verified(verified) = login
                .receive(
                    packet,
                    &policy,
                    Instant::now().into_std(),
                    SystemTime::now(),
                )
                .unwrap()
            else {
                panic!("offline Login not verified");
            };
            let claims: serde_json::Value =
                serde_json::from_str(verified.login().client_data().get()).unwrap();
            assert_eq!(claims["ServerAddress"], endpoint);
            assert_eq!(claims["SkinId"], "synthetic");
            assert_eq!(verified.login().display_name(), "HorizonFixture");
            assert_eq!(verified.login().nonce(), "synthetic-nonce");
            let approved = verified.authorize(|_| Ok::<_, Infallible>(())).unwrap();
            let ready = approved
                .accept(&policy, Instant::now().into_std(), SystemTime::now())
                .unwrap();
            let _accepted = ready
                .finish(&policy, Instant::now().into_std(), SystemTime::now())
                .unwrap();
            send(
                &mut wire,
                vec![
                    packets::PlayStatus {
                        status: PlayStatusType::LoginSuccess,
                    }
                    .into(),
                    packets::ResourcePacksInfo::default().into(),
                    packets::SetTime {
                        time: wire::ZigZag32(22),
                    }
                    .into(),
                ],
                deadline.into_std(),
            )
            .await;
            wire
        };
        let (boundary, wire) = tokio::join!(client_task, server_task);
        let mut boundary = boundary.unwrap();
        assert!(boundary.packs.is_some());
        for id in [
            packets::PacketId::RequestNetworkSettings,
            packets::PacketId::Login,
        ] {
            assert_eq!(sent.iter().filter(|&&actual| actual == id).count(), 1);
        }
        boundary.connection.replay_deferred();
        for expected in [11, 22] {
            let (_, packet) = timeout(Duration::from_secs(2), boundary.connection.next_packet())
                .await
                .unwrap()
                .unwrap();
            let packets::Packet::SetTime(time) = packet else {
                panic!("the original deferred/unread packet was lost or reordered");
            };
            assert_eq!(time.time.0, expected);
        }
        let mut retired_server = wire.begin_close();
        let ((), closed) = tokio::join!(
            boundary.close(Duration::from_secs(2)),
            retired_server.finish_close((Instant::now() + Duration::from_secs(2)).into_std())
        );
        closed.unwrap();
        drop(retired_server);
        listener.close().await.unwrap();
        drop(listener);
        Arc::try_unwrap(signaling)
            .ok()
            .expect("HTTP signaling owner leaked")
            .shutdown()
            .await;
    })
    .await
    .expect("explicit HTTP dial/Login fixture exceeded its overall deadline");
}

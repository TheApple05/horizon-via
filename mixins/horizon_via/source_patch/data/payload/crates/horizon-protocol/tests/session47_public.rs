//! Public-API regressions. Golden old bytes come from actual official1.8.9 packet writers.
//! No Session internals, private cache mutation, test-only state setter, or live account/server.
use horizon_protocol::{
    State,
    adapter::{ServerVersion, Session},
    buf::{Reader, Writer},
    connection::{Connection, Pipe},
    ids,
    item::{Component, Stack, component},
    packets::{Empty, configuration as cfg, handshake, login, play},
};
use horizon_world::nbt::Tag;
use std::time::Duration;

type Wire = (i32, Vec<u8>);
const UUID: u128 = 0x00112233445566778899aabbccddeeff;

fn golden(name: &str) -> Wire {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("session47-goldens.json")).unwrap();
    let row = &value[name];
    let hex = row["hex"].as_str().unwrap();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    (row["id"].as_i64().unwrap() as i32, bytes)
}
fn body((id, payload): &Wire) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(*id);
    w.bytes(payload);
    w.data
}
fn split(bytes: &[u8]) -> Wire {
    let mut r = Reader::new(bytes);
    let id = r.varint().unwrap();
    (id, r.rest().to_vec())
}
fn drain(s: &Session) -> Vec<Wire> {
    std::iter::from_fn(|| s.next_incoming()).collect()
}
fn feed(s: &Session, name: &str) -> Vec<Wire> {
    let (id, payload) = golden(name);
    let mut out = s
        .clientbound(id, payload)
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();
    out.extend(drain(s));
    out
}
fn queued(s: &Session, packet: &[u8]) -> Vec<Vec<u8>> {
    assert!(
        s.serverbound(packet).unwrap().is_none(),
        "old wire packets belong to the priority FIFO"
    );
    s.take_outgoing()
}
fn handshake_body() -> Vec<u8> {
    handshake::Serverbound::Intention(handshake::Intention {
        protocol: 777,
        host: "fixture.invalid".into(),
        port: 25565,
        intent: handshake::LOGIN,
    })
    .encode()
}
fn hello() -> Vec<u8> {
    login::Serverbound::Hello(login::ServerHello {
        name: "SessionOracle".into(),
        uuid: UUID,
    })
    .encode()
}
fn login_ack() -> Vec<u8> {
    login::Serverbound::LoginAcknowledged(Empty).encode()
}
fn finish() -> Vec<u8> {
    cfg::Serverbound::FinishConfiguration(Empty).encode()
}
fn settings() -> Vec<u8> {
    cfg::Serverbound::ClientInformation(cfg::ClientInformation::new(8)).encode()
}
fn brand() -> Vec<u8> {
    let mut w = Writer::new();
    w.string("SessionOracle");
    cfg::Serverbound::CustomPayload(cfg::CustomPayload {
        channel: "minecraft:brand".into(),
        data: w.data,
    })
    .encode()
}
fn begin() -> Session {
    let s = Session::new(ServerVersion::V1_8_9);
    let mut outgoing = s
        .serverbound(&handshake_body())
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();
    outgoing.extend(s.take_outgoing());
    assert_eq!(
        outgoing.iter().map(|b| split(b)).collect::<Vec<_>>(),
        vec![golden("handshake")]
    );
    assert_eq!((s.reading(), s.writing()), (State::Login, State::Login));
    assert!(s.take_outgoing().is_empty());
    assert_eq!(queued(&s, &hello()), vec![body(&golden("hello"))]);
    let packets = feed(&s, "login_finished");
    assert_eq!(packets.len(), 1);
    let login::Clientbound::LoginFinished(p) =
        login::Clientbound::decode(packets[0].0, &packets[0].1).unwrap()
    else {
        panic!("profile")
    };
    assert_eq!(
        (p.profile.uuid, p.profile.name.as_str(), p.session),
        (UUID, "SessionOracle", 0)
    );
    assert!(p.profile.properties.is_empty());
    assert_eq!(
        (s.reading(), s.writing()),
        (State::Configuration, State::Login)
    );
    assert!(queued(&s, &login_ack()).is_empty());
    assert_eq!(s.writing(), State::Configuration);
    s
}
fn play_session() -> Session {
    let s = begin();
    let packets = feed(&s, "join");
    assert_virtual_join(&packets);
    assert!(queued(&s, &finish()).is_empty());
    assert_eq!((s.reading(), s.writing()), (State::Play, State::Play));
    s
}
fn assert_virtual_join(packets: &[Wire]) {
    let finish_at = packets
        .iter()
        .position(|p| {
            p.0 == ids::configuration::clientbound::FINISH_CONFIGURATION && p.1.is_empty()
        })
        .unwrap();
    assert!(finish_at > 1);
    for (id, payload) in &packets[..finish_at - 1] {
        assert!(matches!(
            cfg::Clientbound::decode(*id, payload).unwrap(),
            cfg::Clientbound::RegistryData(_)
        ));
    }
    assert!(
        matches!(cfg::Clientbound::decode(packets[finish_at-1].0,&packets[finish_at-1].1).unwrap(),cfg::Clientbound::UpdateEnabledFeatures(p) if p.features==["minecraft:vanilla"])
    );
    assert_eq!(packets.len() - finish_at - 1, 3);
    let after: Vec<_> = packets[finish_at + 1..]
        .iter()
        .map(|(id, p)| play::Clientbound::decode(*id, p).unwrap())
        .collect();
    assert!(matches!(&after[0],play::Clientbound::Login(p) if p.player_id==123));
    assert!(matches!(&after[1], play::Clientbound::Commands(_)));
    assert!(matches!(&after[2],play::Clientbound::GameEvent(p) if p.event==13));
}
fn use_block(sequence: i32) -> Vec<u8> {
    play::Serverbound::UseItemOn(play::UseItemOn {
        hand: 0,
        pos: [3, 64, -9],
        face: 1,
        cursor: [0.25, 0.5, 0.75],
        inside: false,
        world_border: false,
        sequence,
    })
    .encode()
}
fn click(slot: i16, button: i8, input: i32) -> Vec<u8> {
    play::Serverbound::ContainerClick(play::ContainerClick {
        container: 0,
        state: 0,
        slot,
        button,
        input,
        changed: vec![],
        carried: None,
    })
    .encode()
}
fn content(packets: &[Wire]) -> play::ContainerSetContent {
    packets
        .iter()
        .find_map(|(id, p)| match play::Clientbound::decode(*id, p).unwrap() {
            play::Clientbound::ContainerSetContent(p) if p.container == 0 => Some(p),
            _ => None,
        })
        .expect("complete inventory correction")
}
fn assert_ack(s: &Session, sequence: i32) {
    let incoming = drain(s);
    assert!(incoming.iter().any(|(id,p)|matches!(play::Clientbound::decode(*id,p).unwrap(),play::Clientbound::BlockChangedAck(p) if p.sequence==sequence)));
}

#[test]
fn session47_virtual_configuration_waits_for_finish_delivery_and_defers_real_writes_in_order() {
    let s = begin();
    assert!(s.serverbound(&finish()).is_err());
    assert_eq!(s.writing(), State::Configuration);
    assert!(queued(&s, &settings()).is_empty());
    assert!(queued(&s, &brand()).is_empty());
    let (id, p) = golden("join");
    let first = s.clientbound(id, p).unwrap().unwrap();
    assert!(matches!(
        cfg::Clientbound::decode(first.0, &first.1).unwrap(),
        cfg::Clientbound::RegistryData(_)
    ));
    assert_eq!(s.reading(), State::Play);
    assert_eq!(s.writing(), State::Configuration);
    // Queuing FinishConfiguration is insufficient; consumer must actually receive it.
    assert!(s.serverbound(&finish()).is_err());
    assert!(s.take_outgoing().is_empty());
    let mut all = vec![first];
    loop {
        let next = s.next_incoming().unwrap();
        let done =
            next.0 == ids::configuration::clientbound::FINISH_CONFIGURATION && next.1.is_empty();
        all.push(next);
        if done {
            break;
        }
    }
    assert_eq!(
        queued(&s, &finish()),
        vec![body(&golden("settings")), body(&golden("brand"))]
    );
    assert_eq!(s.writing(), State::Play);
    all.extend(drain(&s));
    assert_virtual_join(&all);
    assert!(s.take_outgoing().is_empty());
    assert!(s.next_incoming().is_none());
}

#[test]
fn session47_numeric_inventory_creative_default_and_main_hand_reach_exact_old_c08() {
    let s = play_session();
    let inventory = content(&feed(&s, "inventory"));
    assert_eq!(inventory.items.len(), 46);
    assert!(inventory.items[45].is_none());
    assert_eq!(inventory.items[36].as_ref().unwrap().count, 3);
    assert_eq!(inventory.items[37].as_ref().unwrap().item, 1);
    assert_eq!(queued(&s, &use_block(7)), vec![body(&golden("place_wool"))]);
    assert_ack(&s, 7);
    assert_eq!(
        queued(
            &s,
            &play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 1 }).encode()
        ),
        vec![body(&golden("select_hotbar1"))]
    );
    assert_eq!(
        queued(&s, &use_block(70)),
        vec![body(&golden("place_stone2"))]
    );
    assert_ack(&s, 70);
    assert_eq!(
        queued(
            &s,
            &play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 0 }).encode()
        ),
        vec![body(&golden("select_hotbar0"))]
    );
    // New creative defaults have no old-original cache entry; canonical mapping
    // must write and remember an exact old stone before later placement uses it.
    let s = play_session();
    feed(&s, "inventory_empty");
    let creative = play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
        slot: 36,
        item: Some(Stack::new(1, 64)),
    })
    .encode();
    assert_eq!(
        queued(&s, &creative),
        vec![body(&golden("creative_stone64"))]
    );
    assert_eq!(
        queued(&s, &use_block(8)),
        vec![body(&golden("place_stone64"))]
    );
    assert_ack(&s, 8);
    let use_item = play::Serverbound::UseItem(play::UseItem {
        hand: 0,
        sequence: 9,
        yaw: 0.,
        pitch: 0.,
    })
    .encode();
    assert_eq!(
        queued(&s, &use_item),
        vec![body(&golden("air_use_stone64"))]
    );
    assert_ack(&s, 9);
}

#[test]
fn session47_invalid_actions_correct_inventory_atomically_without_guessing_old_items() {
    let s = play_session();
    let before = content(&feed(&s, "inventory"));
    assert!(queued(&s, &click(9, 9, 0)).is_empty());
    let correction = content(&drain(&s));
    assert_eq!(correction.items, before.items);
    assert_eq!(correction.carried, before.carried);
    let mut modified = Stack::new(1, 1);
    modified.added.push((
        component::CUSTOM_DATA,
        Component::Nbt(Tag::Compound(
            [("new".into(), Tag::String("unmapped".into()))].into(),
        )),
    ));
    let unsupported = play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
        slot: 36,
        item: Some(modified),
    })
    .encode();
    assert!(queued(&s, &unsupported).is_empty());
    let correction = content(&drain(&s));
    assert_eq!(correction.items, before.items);
    // Malformed typed packet bytes must leave all caches/action numbers alone.
    let valid = click(9, 0, 0);
    assert!(s.serverbound(&valid[..valid.len() - 1]).is_err());
    assert!(s.take_outgoing().is_empty());
    assert!(drain(&s).is_empty());
    assert_eq!(
        queued(&s, &use_block(10)),
        vec![body(&golden("place_wool"))]
    );
    assert_ack(&s, 10);
    assert_eq!(queued(&s, &valid), vec![body(&golden("pickup_first"))]);
    let prediction = content(&drain(&s));
    assert!(prediction.items[9].is_none());
    assert_eq!(prediction.carried.as_ref().unwrap().count, 13);
}

#[test]
fn session47_old_rejection_ack_precedes_next_click_after_both_resync_halves() {
    let s = play_session();
    feed(&s, "inventory");
    assert_eq!(
        queued(&s, &click(9, 0, 0)),
        vec![body(&golden("pickup_first"))]
    );
    drain(&s);
    assert!(feed(&s, "reject_first").is_empty());
    // No second click while only the content half arrived; do not drain FIFO
    // here, because the reader's ack must still be present on the next send.
    feed(&s, "inventory_after_pickup");
    assert!(s.serverbound(&click(10, 0, 0)).unwrap().is_none());
    feed(&s, "cursor_stone");
    assert!(s.serverbound(&click(10, 0, 0)).unwrap().is_none());
    assert_eq!(
        s.take_outgoing(),
        vec![body(&golden("ack_first")), body(&golden("deposit_second"))]
    );
    let correction = content(&drain(&s));
    assert_eq!(correction.items[10].as_ref().unwrap().count, 13);
    assert!(correction.carried.is_none());
}

#[test]
fn session47_local_prediction_is_read_before_newer_server_inventory_update() {
    let s = play_session();
    feed(&s, "inventory");
    queued(&s, &click(9, 0, 0));
    let packets = feed(&s, "set_slot_stone3");
    assert_eq!(packets.len(), 2);
    assert!(
        matches!(play::Clientbound::decode(packets[0].0,&packets[0].1).unwrap(),play::Clientbound::ContainerSetContent(p) if p.items[9].is_none() && p.carried.as_ref().unwrap().count==13)
    );
    assert!(
        matches!(play::Clientbound::decode(packets[1].0,&packets[1].1).unwrap(),play::Clientbound::ContainerSetSlot(p) if p.slot==9 && p.item.as_ref().unwrap().count==3)
    );
}

#[test]
fn session47_teleport_acceptance_writes_old_c06_and_duplicate_ack_never_leaks() {
    let s = play_session();
    let packets = feed(&s, "teleport");
    assert_eq!(packets.len(), 1);
    let play::Clientbound::PlayerPosition(p) =
        play::Clientbound::decode(packets[0].0, &packets[0].1).unwrap()
    else {
        panic!("teleport")
    };
    assert_eq!(p.position, [1.25, 64., -2.5]);
    assert_eq!((p.yaw, p.pitch), (90., -15.));
    let ack = play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
        id: p.teleport,
        x: p.position[0],
        y: p.position[1],
        z: p.position[2],
        yaw: p.yaw,
        pitch: p.pitch,
    })
    .encode();
    assert_eq!(queued(&s, &ack), vec![body(&golden("teleport_accept"))]);
    assert!(s.serverbound(&ack).is_err());
    assert!(s.take_outgoing().is_empty());
}

#[test]
fn session47_portal_retains_inventory_but_reconnect_has_fresh_items_transactions_and_teleports() {
    let first = play_session();
    feed(&first, "inventory_empty");
    let creative = play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
        slot: 36,
        item: Some(Stack::new(1, 64)),
    })
    .encode();
    assert_eq!(
        queued(&first, &creative),
        vec![body(&golden("creative_stone64"))]
    );
    let packets = feed(&first, "respawn_nether");
    assert!(matches!(
        play::Clientbound::decode(packets[0].0, &packets[0].1).unwrap(),
        play::Clientbound::Respawn(_)
    ));
    assert_eq!(
        queued(&first, &use_block(11)),
        vec![body(&golden("place_stone64"))]
    );
    assert_ack(&first, 11);
    feed(&first, "teleport");
    let second = play_session();
    assert!(queued(&second, &use_block(12)).is_empty());
    assert_ack(&second, 12);
    let old_ack = play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
        id: 0,
        x: 1.25,
        y: 64.,
        z: -2.5,
        yaw: 90.,
        pitch: -15.,
    })
    .encode();
    assert!(second.serverbound(&old_ack).is_err());
    assert!(second.take_outgoing().is_empty());
    feed(&second, "inventory");
    assert_eq!(
        queued(&second, &click(9, 0, 0)),
        vec![body(&golden("pickup_first"))]
    );
    // First connection's retained main hand is also unaffected by second's clicks.
    assert_eq!(
        queued(&first, &use_block(13)),
        vec![body(&golden("place_stone64"))]
    );
    assert_ack(&first, 13);
}

#[test]
fn connection47_frames_deferred_configuration_and_reader_ack_before_next_writer_click() {
    let (a, b) = Pipe::pair();
    let mut client = Connection::local(a);
    let mut peer = Connection::local(b);
    client
        .reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.adapt(ServerVersion::V1_8_9);
    client.send(&handshake_body()).unwrap();
    assert_eq!(peer.recv().unwrap(), golden("handshake"));
    client.send(&hello()).unwrap();
    assert_eq!(peer.recv().unwrap(), golden("hello"));
    peer.send(&body(&golden("login_finished"))).unwrap();
    let p = client.recv().unwrap();
    assert!(matches!(
        login::Clientbound::decode(p.0, &p.1).unwrap(),
        login::Clientbound::LoginFinished(_)
    ));
    client.send(&login_ack()).unwrap();
    client.send(&settings()).unwrap();
    client.send(&brand()).unwrap();
    peer.send(&body(&golden("join"))).unwrap();
    let mut virtual_packets = vec![];
    loop {
        let p = client.recv().unwrap();
        let finish = p.0 == ids::configuration::clientbound::FINISH_CONFIGURATION && p.1.is_empty();
        virtual_packets.push(p);
        if finish {
            break;
        }
    }
    client.send(&finish()).unwrap();
    assert_eq!(peer.recv().unwrap(), golden("settings"));
    assert_eq!(peer.recv().unwrap(), golden("brand"));
    for _ in 0..3 {
        virtual_packets.push(client.recv().unwrap());
    }
    assert_virtual_join(&virtual_packets);
    peer.send(&body(&golden("inventory"))).unwrap();
    let p = client.recv().unwrap();
    assert_eq!(content(&[p]).items.len(), 46);
    client.send(&click(9, 0, 0)).unwrap();
    assert_eq!(peer.recv().unwrap(), golden("pickup_first"));
    peer.send(&body(&golden("reject_first"))).unwrap();
    peer.send(&body(&golden("inventory_after_pickup"))).unwrap();
    peer.send(&body(&golden("cursor_stone"))).unwrap();
    // Prediction first, then the real WindowItems/cursor. False-confirm is a
    // consumed control packet: recv must continue until a native packet exists.
    for _ in 0..3 {
        let p = client.recv().unwrap();
        assert!(matches!(
            play::Clientbound::decode(p.0, &p.1).unwrap(),
            play::Clientbound::ContainerSetContent(_)
        ));
    }
    client.send(&click(10, 0, 0)).unwrap();
    assert_eq!(peer.recv().unwrap(), golden("ack_first"));
    assert_eq!(peer.recv().unwrap(), golden("deposit_second"));
}

#[test]
fn malformed_inventory_packet_cannot_poison_the_previously_valid_main_hand_cache() {
    let s = play_session();
    feed(&s, "inventory");
    assert_eq!(
        queued(&s, &use_block(100)),
        vec![body(&golden("place_wool"))]
    );
    assert_ack(&s, 100);
    // This alternate empty-tag encoding normalizes to the same wool. Trailing
    // garbage invalidates the entire SetSlot, including speculative cache writes.
    let mut w = Writer::new();
    w.i8(0);
    w.i16(10);
    w.i16(35);
    w.i8(3);
    w.i16(14);
    w.named_nbt(&Tag::Compound(Default::default()));
    w.u8(99);
    assert!(s.clientbound(0x2f, w.data).is_err());
    assert!(s.take_outgoing().is_empty());
    assert!(drain(&s).is_empty());
    assert_eq!(
        queued(&s, &use_block(101)),
        vec![body(&golden("place_wool"))]
    );
    assert_ack(&s, 101);
}

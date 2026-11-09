use horizon_protocol::{
    adapter::{SendDisposition as D, ServerVersion, Session},
    buf::{Reader, Writer},
    ids,
    item::Stack,
    packets::{Empty, configuration as cfg, handshake, login, play},
};

fn item(name: &str) -> i32 {
    horizon_protocol::adapter::v1_8_9::ids::CURRENT_NAMES
        .iter()
        .find(|(kind, _)| *kind == "minecraft:item")
        .unwrap()
        .1
        .iter()
        .position(|&value| value == name)
        .unwrap() as i32
}
fn session() -> Session {
    let s = Session::new(ServerVersion::V1_8_9);
    s.serverbound(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: 777,
            host: "test".into(),
            port: 25565,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    let mut p = Writer::new();
    p.string("00000000-0000-0000-0000-000000000001");
    p.string("Checked47");
    s.clientbound(2, p.data).unwrap();
    s.serverbound(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    let mut j = Writer::new();
    j.i32(42);
    j.u8(1);
    j.i8(0);
    j.u8(2);
    j.u8(20);
    j.string("default");
    j.bool(false);
    s.clientbound(1, j.data).unwrap();
    while s.next_incoming().is_some() {}
    s.serverbound(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    s.take_outgoing();
    let mut w = Writer::new();
    w.u8(0);
    w.i16(45);
    for slot in 0..45 {
        if slot == 36 {
            w.i16(35);
            w.i8(4);
            w.i16(11);
            w.u8(0);
        } else {
            w.i16(-1);
        }
    }
    s.clientbound(0x30, w.data).unwrap();
    while s.next_incoming().is_some() {}
    s
}
fn creative(slot: i16, name: &str) -> play::Serverbound {
    play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
        slot,
        item: Some(Stack::new(item(name), 1)),
    })
}
fn checked(s: &Session, p: play::Serverbound) -> D {
    let (body, result) = s.serverbound_checked(&p.encode()).unwrap();
    assert!(body.is_none());
    result
}
fn use_on() -> play::Serverbound {
    play::Serverbound::UseItemOn(play::UseItemOn {
        hand: 0,
        pos: [1, 64, 1],
        face: 1,
        cursor: [0.5; 3],
        inside: false,
        world_border: false,
        sequence: 8,
    })
}
fn raw_slot(r: &mut Reader<'_>) -> (i16, i8, i16) {
    let id = r.i16().unwrap();
    let count = r.i8().unwrap();
    let damage = r.i16().unwrap();
    r.named_nbt().unwrap();
    (id, count, damage)
}

#[test]
fn blue_wool_pick_sends_correct_old_variant_then_selects_that_main_hand() {
    let s = session();
    assert_eq!(checked(&s, creative(37, "minecraft:blue_wool")), D::Sent);
    let wire = s.take_outgoing();
    assert_eq!(wire.len(), 1);
    let mut r = Reader::new(&wire[0]);
    assert_eq!(r.varint().unwrap(), 0x10);
    assert_eq!(r.i16().unwrap(), 37);
    assert_eq!(raw_slot(&mut r), (35, 1, 11));
    assert_eq!(r.remaining(), 0);
    assert_eq!(
        checked(
            &s,
            play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 1 })
        ),
        D::Sent
    );
    let wire = s.take_outgoing();
    let mut r = Reader::new(&wire[0]);
    assert_eq!((r.varint().unwrap(), r.i16().unwrap()), (9, 1));
    assert_eq!(checked(&s, use_on()), D::Sent);
    let wire = s.take_outgoing();
    let mut r = Reader::new(&wire[0]);
    assert_eq!(r.varint().unwrap(), 8);
    r.i64().unwrap();
    r.u8().unwrap();
    assert_eq!(raw_slot(&mut r), (35, 1, 11));
}

#[test]
fn checked_rejection_preserves_prior_reader_fifo_and_suppresses_only_its_own_full_echo() {
    let s = session();
    // An ordinary unchecked rejection retains its existing full-correction contract.
    s.serverbound(&creative(36, "minecraft:netherite_block").encode())
        .unwrap();
    assert!(s.take_outgoing().is_empty());
    assert_eq!(
        checked(&s, creative(37, "minecraft:netherite_sword")),
        D::LocallyRejected
    );
    assert!(s.take_outgoing().is_empty());
    let mut ids_seen = Vec::new();
    while let Some((id, _)) = s.next_incoming() {
        ids_seen.push(id);
    }
    assert_eq!(
        ids_seen
            .iter()
            .filter(|&&id| id == ids::play::clientbound::CONTAINER_SET_CONTENT)
            .count(),
        1,
        "older full correction must remain, newer checked echo must be absent"
    );
    assert_eq!(ids_seen[0], ids::play::clientbound::CONTAINER_SET_CONTENT);
    assert!(ids_seen.contains(&ids::play::clientbound::SYSTEM_CHAT));
    assert_eq!(
        checked(&s, creative(38, "minecraft:netherite_sword")),
        D::LocallyRejected,
        "notice throttling must not conceal rejection disposition"
    );
}

#[test]
fn accepted_creative_slots_survive_a_rejected_slot_between_them() {
    let s = session();
    assert_eq!(checked(&s, creative(36, "minecraft:blue_wool")), D::Sent);
    assert_eq!(
        checked(&s, creative(37, "minecraft:netherite_block")),
        D::LocallyRejected
    );
    assert_eq!(checked(&s, creative(38, "minecraft:red_wool")), D::Sent);
    let wire = s.take_outgoing();
    assert_eq!(wire.len(), 2);
    for (bytes, (slot, damage)) in wire.iter().zip([(36, 11), (38, 14)]) {
        let mut r = Reader::new(bytes);
        assert_eq!(r.varint().unwrap(), 0x10);
        assert_eq!(r.i16().unwrap(), slot);
        assert_eq!(raw_slot(&mut r), (35, 1, damage));
    }
    while let Some((id, _)) = s.next_incoming() {
        assert_ne!(id, ids::play::clientbound::CONTAINER_SET_CONTENT);
    }
}

#[test]
fn reader_transaction_ack_is_retained_but_does_not_classify_a_rejected_call_as_sent() {
    let s = session();
    let mut w = Writer::new();
    w.u8(0);
    w.i16(123);
    w.bool(false);
    s.clientbound(0x32, w.data).unwrap();
    assert_eq!(
        checked(&s, creative(36, "minecraft:netherite_block")),
        D::LocallyRejected
    );
    let wire = s.take_outgoing();
    assert_eq!(wire.len(), 1);
    let mut r = Reader::new(&wire[0]);
    assert_eq!(r.varint().unwrap(), 0x0f);
    assert_eq!(r.u8().unwrap(), 0);
    assert_eq!(r.i16().unwrap(), 123);
    assert!(r.bool().unwrap());
    assert_eq!(
        checked(&s, play::Serverbound::ClientTickEnd(Empty)),
        D::Consumed
    );
    assert!(s.take_outgoing().is_empty());
}

#[test]
fn current_checked_handshake_retains_original_packet_and_has_no_local_outcome_side_effects() {
    let s = Session::new(ServerVersion::Current);
    let body = handshake::Serverbound::Intention(handshake::Intention {
        protocol: 777,
        host: "example".into(),
        port: 25565,
        intent: handshake::STATUS,
    })
    .encode();
    let (wire, outcome) = s.serverbound_checked(&body).unwrap();
    assert_eq!(outcome, D::Sent);
    assert_eq!(wire, Some(body));
    assert!(s.next_incoming().is_none());
}

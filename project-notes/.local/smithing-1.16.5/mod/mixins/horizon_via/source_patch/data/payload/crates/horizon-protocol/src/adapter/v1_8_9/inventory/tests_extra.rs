use super::tests::{Cx, hex, read, write};
use super::*;

struct HorseCx(i32);
impl Context for HorseCx {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        Cx.text(r)
    }
    fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>> {
        Cx.read_old_slot(r)
    }
    fn try_write_old_stack(&self, s: Option<&Stack>, w: &mut Writer) -> Result<()> {
        Cx.try_write_old_stack(s, w)
    }
    fn entity_kind(&self, id: i32) -> Result<i32> {
        if id == 123 {
            Ok(self.0)
        } else {
            Err(Error::new("unknown test horse"))
        }
    }
}
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("extra-fixtures.json")).unwrap()
}
fn snapshot(state: &mut State, slots: &serde_json::Value, cursor: &str, cx: &impl Context) {
    let mut w = Writer::new();
    w.u8(state.active().id);
    w.i16(slots.as_array().unwrap().len() as i16);
    for slot in slots.as_array().unwrap() {
        w.bytes(&hex(slot.as_str().unwrap()));
    }
    state.clientbound(0x30, &w.data, cx).unwrap();
    let mut w = Writer::new();
    w.i8(-1);
    w.i16(-1);
    w.bytes(&hex(cursor));
    state.clientbound(0x2f, &w.data, cx).unwrap();
}
fn reject(state: &mut State, action: i16, cx: &impl Context) {
    let mut w = Writer::new();
    w.u8(1);
    w.i16(action);
    w.bool(false);
    state.clientbound(0x32, &w.data, cx).unwrap();
    let ack = state.take_outgoing();
    assert_eq!(ack.len(), 1);
    assert_eq!(ack[0].id, 0x0f);
}
fn assert_slots(state: &State, expected: &serde_json::Value) {
    let actual: Vec<_> = state
        .active()
        .slots
        .iter()
        .map(|v| write(v.as_ref()))
        .collect();
    let expected: Vec<_> = expected
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(actual, expected);
}
#[test]
fn merchant_payment_scripts_match_actual_vanilla_checksums_and_authoritative_results() {
    let f = fixture();
    for case in f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "merchant" && v["name"] != "merchant_take_result")
    {
        let mut state = State::new();
        state
            .clientbound(
                0x2d,
                &hex(f["packets"]["open_merchant"].as_str().unwrap()),
                &Cx,
            )
            .unwrap();
        snapshot(&mut state, &case["steps"][0]["before"], "ffff", &Cx);
        let out = state
            .clientbound(0x3f, &hex(f["packets"]["offers"].as_str().unwrap()), &Cx)
            .unwrap()
            .unwrap();
        assert!(
            matches!(&out[0],play::Clientbound::MerchantOffers(p) if p.offers.len()==3 && !p.show_progress && !p.can_restock)
        );
        let index = if case["name"] == "merchant_fill_two" {
            1
        } else {
            0
        };
        let out = state
            .serverbound(
                &play::Serverbound::SelectTrade(play::SelectTrade { item: index }),
                &Cx,
            )
            .unwrap()
            .unwrap();
        assert_eq!(out[0].id, 0x17);
        let mut r = Reader::new(&out[0].payload);
        assert_eq!(r.string(20).unwrap(), "MC|TrSel");
        assert_eq!(r.i32().unwrap(), index);
        end(&r).unwrap();
        let steps = case["steps"].as_array().unwrap();
        assert_eq!(out.len(), steps.len() + 1, "{}", case["name"]);
        for (packet, expected) in out[1..].iter().zip(steps) {
            assert_eq!(packet.id, 0x0e);
            let actual = &packet.payload;
            let expected = hex(expected["click_wire"].as_str().unwrap());
            if actual[6] == 3 {
                assert_eq!(&actual[..7], &expected[..7]);
                assert_eq!(&expected[7..], &[255, 255]);
                assert_ne!(&actual[7..], &expected[7..]);
            } else {
                assert_eq!(actual, &expected, "{}", case["name"]);
            }
        }
        let last = steps.last().unwrap();
        reject(&mut state, steps.len() as i16, &Cx);
        snapshot(
            &mut state,
            &last["after"],
            last["carried_after"].as_str().unwrap(),
            &Cx,
        );
        assert!(state.resync.is_none());
        assert_slots(&state, &last["after"]);
        assert_eq!(write(state.carried.as_ref()), last["carried_after"]);
    }
}
#[test]
fn horse_layout_equipment_and_click_resync_follow_actual_all_five_old_subtypes() {
    let f = fixture();
    for case in f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "horse")
    {
        let type_id = case["type"].as_u64().unwrap() as usize;
        let cx = HorseCx([67, 37, 88, 155, 119][type_id]);
        let mut state = State::new();
        let packet = if case["chested"] == true {
            "open_donkey"
        } else {
            "open_horse"
        };
        let open = state
            .clientbound(0x2d, &hex(f["packets"][packet].as_str().unwrap()), &cx)
            .unwrap()
            .unwrap();
        assert!(
            matches!(&open[0],play::Clientbound::MountScreenOpen(p) if p.entity==123 && p.columns==if case["chested"]==true {5}else{0})
        );
        snapshot(&mut state, &case["steps"][0]["before"], "ffff", &cx);
        let updates = state.take_mount_updates();
        assert_eq!(updates.len(), 2);
        if case["name"] == "horse_named_armor" {
            let armor = &updates[1];
            assert_ne!(armor.raw.as_ref().unwrap().tag, Tag::End);
            assert_eq!(
                armor.item,
                read(case["steps"][0]["before"][1].as_str().unwrap())
            );
            assert!(
                matches!(armor.packet(67),play::Clientbound::SetEquipment(p) if p.slots[0].0==6 && p.slots[0].1==armor.item)
            );
        }
        for step in case["steps"].as_array().unwrap() {
            let click = play::Serverbound::ContainerClick(play::ContainerClick {
                container: 1,
                state: state.active().state,
                slot: step["slot"].as_i64().unwrap() as i16,
                button: step["button"].as_i64().unwrap() as i8,
                input: step["mode"].as_i64().unwrap() as i32,
                changed: vec![],
                carried: None,
            });
            let out = state.serverbound(&click, &cx).unwrap().unwrap();
            assert_eq!(out.len(), 1);
            let expected = hex(step["click_wire"].as_str().unwrap());
            assert_eq!(&out[0].payload[..7], &expected[..7]);
            assert_ne!(
                &out[0].payload[7..],
                &expected[7..],
                "authoritative checksum {}",
                case["name"]
            );
            assert!(state.resync.is_some());
            let action = state.active().action;
            reject(&mut state, action, &cx);
            snapshot(
                &mut state,
                &step["after"],
                step["carried_after"].as_str().unwrap(),
                &cx,
            );
            assert_slots(&state, &step["after"]);
            assert_eq!(write(state.carried.as_ref()), step["carried_after"]);
        }
    }
}
#[test]
fn new_owned_packet_truncations_invalid_layouts_and_tagged_costs_are_atomic() {
    let f = fixture();
    for name in ["open_merchant", "open_horse", "open_donkey", "offers"] {
        let bytes = hex(f["packets"][name].as_str().unwrap());
        for n in 0..bytes.len() {
            let mut state = State::new();
            let before = format!("{state:?}");
            assert!(
                state
                    .clientbound(
                        if name == "offers" { 0x3f } else { 0x2d },
                        &bytes[..n],
                        &HorseCx(37)
                    )
                    .is_err(),
                "{name}/{n}"
            );
            assert_eq!(format!("{state:?}"), before);
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        let mut state = State::new();
        let before = format!("{state:?}");
        assert!(
            state
                .clientbound(
                    if name == "offers" { 0x3f } else { 0x2d },
                    &trailing,
                    &HorseCx(37)
                )
                .is_err()
        );
        assert_eq!(format!("{state:?}"), before);
    }
    assert!(mount::kind(17, 123, &HorseCx(67)).is_err());
    assert!(mount::kind(3, 123, &HorseCx(37)).is_err());
    assert!(mount::kind(2, 123, &HorseCx(1)).is_err());
    let f = fixture();
    for id in 0..2300 {
        assert_eq!(
            merchant::damageable(id),
            f["damageable"].get(id.to_string()).is_some(),
            "oldid{id}"
        );
    }
}

#[test]
fn merchant_result_click_uses_server_recipe_consumption_and_preserves_cursor_during_selection() {
    let f = fixture();
    let case = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "merchant_take_result")
        .unwrap();
    let mut state = State::new();
    state
        .clientbound(
            0x2d,
            &hex(f["packets"]["open_merchant"].as_str().unwrap()),
            &Cx,
        )
        .unwrap();
    snapshot(&mut state, &case["steps"][0]["before"], "ffff", &Cx);
    state
        .clientbound(0x3f, &hex(f["packets"]["offers"].as_str().unwrap()), &Cx)
        .unwrap();
    let p = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 1,
        state: state.active().state,
        slot: 2,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    let out = state.serverbound(&p, &Cx).unwrap().unwrap();
    assert_eq!(out.len(), 1);
    assert!(state.resync.is_some());
    reject(&mut state, 1, &Cx);
    let last = &case["steps"][0];
    snapshot(
        &mut state,
        &last["after"],
        last["carried_after"].as_str().unwrap(),
        &Cx,
    );
    assert_slots(&state, &last["after"]);
    assert_eq!(write(state.carried.as_ref()), last["carried_after"]);
    // A held cursor is never dropped/overwritten by payment autofill. Only select
    // plus the exact no-op checksum request runs; the server echoes that cursor.
    let before = state.carried.clone();
    let slots = state.active().slots.clone();
    let out = state
        .serverbound(
            &play::Serverbound::SelectTrade(play::SelectTrade { item: 1 }),
            &Cx,
        )
        .unwrap()
        .unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(state.carried, before);
    assert_eq!(state.active().slots, slots);
    reject(&mut state, 2, &Cx);
    snapshot(
        &mut state,
        &last["after"],
        last["carried_after"].as_str().unwrap(),
        &Cx,
    );
    assert_eq!(state.carried, before);
}

#[test]
fn merchant_latest_selection_waits_for_both_halves_and_full_inventory_never_drops_payments() {
    let f = fixture();
    let case = &f["cases"][0];
    let mut state = State::new();
    state
        .clientbound(
            0x2d,
            &hex(f["packets"]["open_merchant"].as_str().unwrap()),
            &Cx,
        )
        .unwrap();
    snapshot(&mut state, &case["steps"][0]["before"], "ffff", &Cx);
    state
        .clientbound(0x3f, &hex(f["packets"]["offers"].as_str().unwrap()), &Cx)
        .unwrap();
    state
        .serverbound(
            &play::Serverbound::SelectTrade(play::SelectTrade { item: 0 }),
            &Cx,
        )
        .unwrap();
    for index in [1, 2] {
        let out = state
            .serverbound(
                &play::Serverbound::SelectTrade(play::SelectTrade { item: index }),
                &Cx,
            )
            .unwrap()
            .unwrap();
        assert!(out.is_empty());
    }
    let action = state.active().action;
    reject(&mut state, action, &Cx);
    let last = case["steps"].as_array().unwrap().last().unwrap();
    snapshot(&mut state, &last["after"], "ffff", &Cx);
    let queued = state.take_outgoing();
    assert_eq!(queued[0].id, 0x17);
    let mut r = Reader::new(&queued[0].payload);
    assert_eq!(r.string(20).unwrap(), "MC|TrSel");
    assert_eq!(r.i32().unwrap(), 2);
    assert!(state.resync.is_some());
    assert_eq!(state.deferred_trade, None);
    // Independent full player inventory with incompatible inputs: failed clearing
    // restores the remainder to the input and emits no drop/outside click.
    let mut full = State::new();
    full.clientbound(
        0x2d,
        &hex(f["packets"]["open_merchant"].as_str().unwrap()),
        &Cx,
    )
    .unwrap();
    let mut slots = vec!["ffff"; 39];
    slots[0] = "000305000000";
    for v in &mut slots[3..] {
        *v = "000140000000";
    }
    let slots = serde_json::to_value(slots).unwrap();
    snapshot(&mut full, &slots, "ffff", &Cx);
    full.clientbound(0x3f, &hex(f["packets"]["offers"].as_str().unwrap()), &Cx)
        .unwrap();
    let out = full
        .serverbound(
            &play::Serverbound::SelectTrade(play::SelectTrade { item: 0 }),
            &Cx,
        )
        .unwrap()
        .unwrap();
    assert_eq!(out.len(), 4);
    assert_slots(&full, &slots);
    assert!(full.carried.is_none());
    for click in &out[1..] {
        let mut r = Reader::new(&click.payload);
        r.u8().unwrap();
        assert_ne!(r.i16().unwrap(), -999);
    }
    full.on_respawn();
    assert!(full.merchant.is_none());
    assert!(full.deferred_trade.is_none());
    assert!(full.mount_updates.is_empty());
}

#[test]
fn tagged_cost_rejection_keeps_offer_state_and_complete_result_nbt() {
    let f = fixture();
    let mut state = State::new();
    state
        .clientbound(
            0x2d,
            &hex(f["packets"]["open_merchant"].as_str().unwrap()),
            &Cx,
        )
        .unwrap();
    let out = state
        .clientbound(0x3f, &hex(f["packets"]["offers"].as_str().unwrap()), &Cx)
        .unwrap()
        .unwrap();
    let play::Clientbound::MerchantOffers(offers) = &out[0] else {
        panic!()
    };
    assert!(offers.offers[2].out_of_stock);
    assert_eq!(
        offers.offers[2].cost_a.components,
        vec![(
            crate::item::component::DAMAGE,
            crate::item::Component::Int(0)
        )]
    );
    assert!(
        offers.offers[1]
            .result
            .component(crate::item::component::CUSTOM_DATA)
            .is_some()
    );
    let before = format!("{state:?}");
    let mut w = Writer::new();
    w.string("MC|TrList");
    w.i32(1);
    w.u8(1);
    // Reuse the full named armor NBT as a deliberately unsupported tagged COST.
    let named = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "horse_named_armor")
        .unwrap()["steps"][0]["before"][1]
        .as_str()
        .unwrap();
    w.bytes(&hex(named));
    w.bytes(&hex("018401000000"));
    w.bool(false);
    w.bool(false);
    w.i32(0);
    w.i32(7);
    assert!(
        state
            .clientbound(0x3f, &w.data, &Cx)
            .unwrap_err()
            .to_string()
            .contains("subset predicate")
    );
    assert_eq!(format!("{state:?}"), before);
}

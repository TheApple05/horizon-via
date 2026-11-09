use super::*;
use crate::{
    buf::Codec,
    entity_data::{Value, serializer as s},
};
fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(label: &str) -> Vec<u8> {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vanilla-entity-fixtures.json")).unwrap();
    hex(report["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == label)
        .unwrap()["hex"]
        .as_str()
        .unwrap())
}
fn no_item(_: &raw::Slot) -> Result<crate::item::Stack> {
    Err(Error::new("unexpected item"))
}
fn no_state(_: u16) -> Result<i32> {
    Err(Error::new("unexpected state"))
}
#[test]
fn player_reserved_default_does_not_overwrite_skin_customization() {
    // Synthetic watcher records reproduce the independently observed initial
    // vanilla player defaults, including all skin parts and reserved16 Byte0.
    let entries = vec![
        raw::Entry {
            index: 0,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 1,
            value: raw::Value::Short(300),
        },
        raw::Entry {
            index: 2,
            value: raw::Value::String(String::new()),
        },
        raw::Entry {
            index: 3,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 4,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 6,
            value: raw::Value::Float(20.),
        },
        raw::Entry {
            index: 7,
            value: raw::Value::Int(0),
        },
        raw::Entry {
            index: 8,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 9,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 10,
            value: raw::Value::Byte(127),
        },
        raw::Entry {
            index: 16,
            value: raw::Value::Byte(0),
        },
        raw::Entry {
            index: 17,
            value: raw::Value::Float(0.),
        },
        raw::Entry {
            index: 18,
            value: raw::Value::Int(0),
        },
    ];
    let output = semantic::State::new(-1)
        .unwrap()
        .apply(&entries, no_item, no_state)
        .unwrap();
    assert!(output.untranslated.is_empty());
    let skin: Vec<_> = output.values.iter().filter(|v| v.0 == 16).collect();
    assert_eq!(skin, vec![&(16, s::BYTE, Value::Byte(127))]);
}
#[test]
fn player_reserved_field_stays_strict_for_unknown_values_and_wrong_types() {
    let mut state = semantic::State::new(-1).unwrap();
    let entry = raw::Entry {
        index: 16,
        value: raw::Value::Byte(1),
    };
    assert_eq!(
        state
            .apply(std::slice::from_ref(&entry), no_item, no_state)
            .unwrap()
            .untranslated,
        vec![entry]
    );
    assert!(
        state
            .apply(
                &[raw::Entry {
                    index: 16,
                    value: raw::Value::Short(0)
                }],
                no_item,
                no_state
            )
            .is_err()
    );
}
#[test]
fn actual_vanilla_spawn_writers_preserve_uuid_coordinates_angles_and_velocity() {
    let player = spawn_player(&fixture("spawn_player")).unwrap();
    assert_eq!(player.entity.id, 123);
    assert_eq!(player.entity.uuid, 0x00112233445566778899aabbccddeeff);
    assert_eq!(player.entity.kind, 159);
    assert_eq!(player.entity.position, [1.25, 64., -2.5]);
    assert_eq!(
        (
            player.entity.yaw,
            player.entity.pitch,
            player.entity.head_yaw
        ),
        (64, -32, 64)
    );
    assert_eq!(player.held_item_hint, Some(276));
    let zombie = spawn_mob(&fixture("spawn_zombie"), 0x123456).unwrap();
    assert_eq!(
        (zombie.entity.id, zombie.entity.uuid, zombie.entity.kind),
        (456, 0x123456, 154)
    );
    assert_eq!(zombie.entity.position, [1.25, 64., -2.5]);
    assert_eq!(zombie.entity.velocity, [0.1, -0.2, 0.]);
    assert_eq!(
        (
            zombie.entity.yaw,
            zombie.entity.pitch,
            zombie.entity.head_yaw
        ),
        (64, -32, -64)
    );
    let mut state = semantic::State::new(zombie.old_kind).unwrap();
    let updated = state.apply(&zombie.metadata, no_item, no_state).unwrap();
    assert!(updated.untranslated.is_empty());
    assert!(
        updated
            .values
            .contains(&(16, s::BOOLEAN, Value::Bool(true)))
    );
    assert!(updated.values.contains(&(9, s::FLOAT, Value::Float(20.))));
}
#[test]
fn object_namespace_and_conditional_velocity_are_not_confused_with_mob_ids() {
    let arrow = spawn_object(&fixture("spawn_arrow")).unwrap();
    assert_eq!((arrow.id, arrow.object_type, arrow.data), (77, 60, 124));
    assert_eq!(arrow.velocity, Some([0.1, -0.2, 0.]));
    let snowball = spawn_object(&fixture("spawn_snowball_without_velocity")).unwrap();
    assert_eq!(
        (snowball.object_type, snowball.data, snowball.velocity),
        (61, 0, None)
    );
}
#[test]
fn vanilla_equipment_and_motion_preserve_old_scaling_and_ordinals() {
    let value = equipment(&fixture("equipment_helmet")).unwrap();
    assert_eq!((value.entity, value.slot), (123, 5));
    let item = value.item.unwrap();
    assert_eq!((item.id, item.count, item.damage), (306, 1, 7));
    assert_eq!(
        relative_move(&fixture("relative_move"), false).unwrap(),
        RelativeMove {
            entity: 123,
            delta: [-16384, 128, 16256],
            rotation: None,
            on_ground: true
        }
    );
    let value = teleport(&fixture("entity_teleport")).unwrap();
    assert_eq!(value.position, [1.25, 64., -2.5]);
    assert_eq!((value.yaw, value.pitch, value.relatives), (90., -45., 0xe0));
}
#[test]
fn initialized_vanilla_watchers_have_strict_types_and_pinned_metadata_decodes_completely() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vanilla-entity-fixtures.json")).unwrap();
    let mut count = 0;
    for (old_kind, row) in report["watchers"].as_object().unwrap() {
        let Some(wire) = row["metadata_hex"].as_str() else {
            continue;
        };
        let mut packet = vec![123];
        packet.extend(hex(wire));
        let metadata = raw::read(&packet).unwrap();
        let mut state = semantic::State::new(old_kind.parse().unwrap()).unwrap();
        let update = state
            .apply(&metadata.entries, no_item, |old| Ok(i32::from(old)))
            .unwrap();
        assert!(
            update.untranslated.is_empty(),
            "old_kind{old_kind}: {:?}",
            update.untranslated
        );
        let current = play::SetEntityData {
            id: 123,
            values: update.values,
        };
        let mut w = crate::buf::Writer::new();
        play::SetEntityData::write(&current, &mut w);
        let mut r = Reader::new(&w.data);
        let decoded = play::SetEntityData::read(&mut r).unwrap();
        assert_eq!(decoded.id, 123);
        assert_eq!(r.remaining(), 0, "old_kind{old_kind}");
        count += 1;
    }
    assert!(count >= 55);
}
#[test]
fn shared_flag_using_item_is_separated_and_unknown_fields_remain_explicit() {
    let mut state = semantic::State::new(-1).unwrap();
    let update = state
        .apply(
            &[
                raw::Entry {
                    index: 0,
                    value: raw::Value::Byte(0x36),
                },
                raw::Entry {
                    index: 31,
                    value: raw::Value::Int(42),
                },
            ],
            no_item,
            no_state,
        )
        .unwrap();
    assert!(update.values.contains(&(0, s::BYTE, Value::Byte(0x22))));
    assert!(update.values.contains(&(8, s::BYTE, Value::Byte(1))));
    assert_eq!(update.untranslated.len(), 1);
    assert_eq!(update.untranslated[0].index, 31);
    assert!(
        state
            .apply(
                &[raw::Entry {
                    index: 0,
                    value: raw::Value::Float(0.)
                }],
                no_item,
                no_state
            )
            .is_err()
    );
    let again = state.apply(&[], no_item, no_state).unwrap();
    assert_eq!(again.values, update.values);
}
#[test]
fn subtype_changes_require_entity_replacement_and_cached_old_values_survive_partial_updates() {
    let mut state = semantic::State::new(51).unwrap();
    let update = state
        .apply(
            &[raw::Entry {
                index: 13,
                value: raw::Value::Byte(1),
            }],
            no_item,
            no_state,
        )
        .unwrap();
    assert_eq!(update.pinned_kind, 150);
    assert!(update.kind_changed);
    assert!(!state.apply(&[], no_item, no_state).unwrap().kind_changed);
    let mut state = semantic::State::new(58).unwrap();
    let update = state
        .apply(
            &[
                raw::Entry {
                    index: 16,
                    value: raw::Value::Short(1),
                },
                raw::Entry {
                    index: 17,
                    value: raw::Value::Byte(1),
                },
            ],
            no_item,
            |wire| {
                assert_eq!(wire, 17);
                Ok(77)
            },
        )
        .unwrap();
    assert!(
        update
            .values
            .contains(&(16, s::OPTIONAL_BLOCK_STATE, Value::Id(77)))
    );
    let update = state
        .apply(
            &[raw::Entry {
                index: 17,
                value: raw::Value::Byte(2),
            }],
            no_item,
            |wire| {
                assert_eq!(wire, 18);
                Ok(88)
            },
        )
        .unwrap();
    assert!(
        update
            .values
            .contains(&(16, s::OPTIONAL_BLOCK_STATE, Value::Id(88)))
    );
}
#[test]
fn packet_parsers_reject_all_truncations_trailing_bytes_and_illegal_booleans() {
    for name in [
        "spawn_player",
        "spawn_zombie",
        "spawn_arrow",
        "equipment_helmet",
        "relative_move",
        "entity_teleport",
    ] {
        let input = fixture(name);
        let parse = |data: &[u8]| match name {
            "spawn_player" => spawn_player(data).map(|_| ()),
            "spawn_zombie" => spawn_mob(data, 1).map(|_| ()),
            "spawn_arrow" => spawn_object(data).map(|_| ()),
            "equipment_helmet" => equipment(data).map(|_| ()),
            "relative_move" => relative_move(data, false).map(|_| ()),
            _ => teleport(data).map(|_| ()),
        };
        for n in 0..input.len() {
            assert!(parse(&input[..n]).is_err(), "{name} length{n}");
        }
        let mut extra = input.clone();
        extra.push(0);
        assert!(parse(&extra).is_err(), "{name}");
    }
    let mut value = fixture("relative_move");
    *value.last_mut().unwrap() = 2;
    assert!(relative_move(&value, false).is_err());
}

#[test]
fn actual_nbt_loaded_vanilla_entities_reach_the_correct_modern_subtype_and_named_fields() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vanilla-entity-fixtures.json")).unwrap();
    for row in report["semantic_cases"].as_array().unwrap() {
        let mut wire = vec![123];
        wire.extend(hex(row["metadata_hex"].as_str().unwrap()));
        let data = raw::read(&wire).unwrap();
        let mut state = semantic::State::new(row["old_kind"].as_i64().unwrap() as i32).unwrap();
        let update = state.apply(&data.entries, no_item, no_state).unwrap();
        assert!(update.untranslated.is_empty());
        match row["name"].as_str().unwrap() {
            "baby_villager_zombie" => {
                assert_eq!(update.pinned_kind, 157);
                assert!(update.values.contains(&(16, s::BOOLEAN, Value::Bool(true))));
                assert!(update.values.contains(&(
                    2,
                    s::OPTIONAL_COMPONENT,
                    Value::OptText(Some(horizon_world::nbt::Tag::compound([(
                        "text",
                        horizon_world::nbt::Tag::String("Watcher literal".into())
                    )])))
                )));
                assert!(update.values.contains(&(3, s::BOOLEAN, Value::Bool(true))));
                assert!(update.values.contains(&(4, s::BOOLEAN, Value::Bool(true))));
            }
            "charged_ignited_creeper" => {
                assert!(update.values.contains(&(17, s::BOOLEAN, Value::Bool(true))));
                assert!(update.values.contains(&(18, s::BOOLEAN, Value::Bool(true))));
            }
            "wither_skeleton" => assert_eq!(update.pinned_kind, 150),
            _ => panic!("unexpected semantic case"),
        }
    }
}
#[test]
fn actual_watcher_writers_preserve_horse_equipment_owner_and_wolf_anger_and_resolve_only_declared_cat_ids()
 {
    let mut horse = semantic::State::new(100).unwrap();
    let update = horse
        .apply(
            &raw::read(&fixture("horse_metadata")).unwrap().entries,
            no_item,
            no_state,
        )
        .unwrap();
    assert!(update.untranslated.is_empty());
    assert!(
        update
            .values
            .contains(&(18, s::BYTE, Value::Byte(0xf2u8 as i8)))
    );
    assert!(update.values.contains(&(19, s::INT, Value::Int(1027))));
    assert!(
        update
            .equipment
            .contains(&(7, Some(crate::item::Stack::new(949, 1))))
    );
    assert!(
        update
            .equipment
            .contains(&(6, Some(crate::item::Stack::new(1409, 1))))
    );
    let mut wolf = semantic::State::new(95).unwrap();
    let update = wolf
        .apply(
            &raw::read(&fixture("wolf_metadata")).unwrap().entries,
            no_item,
            no_state,
        )
        .unwrap();
    assert!(update.untranslated.is_empty());
    assert!(
        update
            .values
            .contains(&(22, s::LONG, Value::Long(i64::MAX)))
    );
    assert!(update.values.contains(&(21, s::INT, Value::Int(14))));
    assert!(update.values.contains(&(
        19,
        s::OPTIONAL_LIVING_ENTITY_REFERENCE,
        Value::OptUuid(Some(0x00112233445566778899aabbccddeeff))
    )));
    let update = wolf
        .apply(
            &[raw::Entry {
                index: 16,
                value: raw::Value::Byte(5),
            }],
            no_item,
            no_state,
        )
        .unwrap();
    assert!(update.values.contains(&(22, s::LONG, Value::Long(0))));
    let mut cat = semantic::State::new(98).unwrap();
    let mut update = cat
        .apply(
            &raw::read(&fixture("cat_metadata")).unwrap().entries,
            no_item,
            no_state,
        )
        .unwrap();
    assert_eq!(update.pinned_kind, 21);
    update
        .resolve_registries(|registry, name| {
            assert_eq!(
                (registry, name),
                ("minecraft:cat_variant", "minecraft:siamese")
            );
            Ok(2)
        })
        .unwrap();
    // Independent current codec contract: cats use a bare registry ID, unlike paintings.
    assert!(update.values.contains(&(20, s::CAT_VARIANT, Value::Id(2))));
    assert!(update.registry_values.is_empty());
}
fn run_bridge(state: &mut lifecycle::Entities, id: i32, bytes: &[u8]) -> lifecycle::Output {
    let mut item = |slot: &raw::Slot| {
        Ok(crate::item::Stack::new(
            i32::from(slot.id) + 100,
            i32::from(slot.count),
        ))
    };
    let mut block = |wire: u16| Ok(i32::from(wire) + 1000);
    let mut registry = |registry: &str, name: &str| match (registry, name) {
        ("minecraft:painting_variant", "minecraft:donkey_kong") => Ok(25),
        ("minecraft:cat_variant", "minecraft:siamese") => Ok(2),
        _ => Err(Error::new("undeclared fixture registry entry")),
    };
    let mut component = |json: &str| {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|_| Error::new("fixture component JSON"))?;
        Ok(horizon_world::nbt::Tag::String(
            value["text"].as_str().unwrap_or("").into(),
        ))
    };
    state
        .clientbound(
            id,
            bytes,
            |id| 0x100000000 | id as u128,
            &mut lifecycle::Callbacks {
                item: &mut item,
                block: &mut block,
                registry: &mut registry,
                component: &mut component,
            },
        )
        .unwrap()
        .unwrap()
}
fn decode_every_packet(out: &lifecycle::Output) {
    for packet in &out.packets {
        let encoded = packet.encode();
        let mut r = Reader::new(&encoded);
        let id = r.varint().unwrap();
        let payload = r.rest();
        play::Clientbound::decode(id, payload).unwrap();
    }
}
#[test]
fn all_actual_object_minecart_painting_xp_and_effect_fixtures_decode_as_current_packets() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vanilla-entity-fixtures.json")).unwrap();
    let mut count = 0;
    for row in report["fixtures"].as_array().unwrap() {
        let label = row["name"].as_str().unwrap();
        if !(label.starts_with("object_type_")
            || label.starts_with("minecart_type_")
            || label == "spawn_painting"
            || label == "spawn_xp"
            || label.starts_with("effect_"))
        {
            continue;
        }
        let mut state = lifecycle::Entities::default();
        let out = run_bridge(
            &mut state,
            row["id"].as_i64().unwrap() as i32,
            &hex(row["hex"].as_str().unwrap()),
        );
        decode_every_packet(&out);
        count += 1;
        if label == "object_type_70" {
            let play::Clientbound::AddEntity(entity) = &out.packets[0] else {
                panic!("falling block")
            };
            assert_eq!(entity.data, 1017);
        }
        if label == "object_type_60" {
            let play::Clientbound::AddEntity(entity) = &out.packets[0] else {
                panic!("arrow")
            };
            assert_eq!(entity.data, 123);
        }
        if label == "spawn_painting" {
            let play::Clientbound::AddEntity(entity) = &out.packets[0] else {
                panic!("painting")
            };
            assert_eq!(
                (entity.kind, entity.position, entity.data),
                (94, [3., 64., -9.], 2)
            );
        }
        if label == "spawn_xp" {
            let play::Clientbound::SetEntityData(data) = &out.packets[1] else {
                panic!("xp")
            };
            assert!(data.values.contains(&(8, s::INT, Value::Int(37))));
        }
    }
    assert_eq!(count, 32);
    assert_eq!(
        effects::update(&fixture("effect_hidden_ambient"))
            .unwrap()
            .flags,
        4
    );
    assert_eq!(
        effects::update(&fixture("effect_visible")).unwrap().flags,
        6
    );
}
#[test]
fn subtype_replacement_preserves_uuid_position_equipment_and_passengers_then_destroy_and_reuse_clear_caches()
 {
    let mut entities = lifecycle::Entities::default();
    run_bridge(&mut entities, 12, &fixture("spawn_player"));
    run_bridge(&mut entities, 15, &fixture("spawn_zombie"));
    run_bridge(&mut entities, 4, &fixture("equipment_helmet"));
    run_bridge(&mut entities, 21, &fixture("relative_move"));
    run_bridge(&mut entities, 27, &fixture("attach_passenger"));
    // Player123 changes no subtype. Reuse a zombie fixture with skeleton's entity kind and id123.
    let mut skeleton = fixture("spawn_zombie");
    skeleton.splice(0..3, [123, 51]);
    run_bridge(&mut entities, 15, &skeleton);
    run_bridge(&mut entities, 4, &fixture("equipment_helmet"));
    run_bridge(&mut entities, 21, &fixture("relative_move"));
    run_bridge(&mut entities, 27, &fixture("attach_passenger"));
    let uuid = entities.uuid(123);
    let out = run_bridge(&mut entities, 28, &[123, 13, 1, 127]);
    decode_every_packet(&out);
    assert!(matches!(
        out.packets[0],
        play::Clientbound::RemoveEntities(_)
    ));
    let play::Clientbound::AddEntity(entity) = &out.packets[1] else {
        panic!("replacement")
    };
    assert_eq!(
        (entity.kind, entity.uuid, entity.position),
        (150, uuid.unwrap(), [-2.75, 64.03125, 1.46875])
    );
    assert!(out.packets.iter().any(|packet|matches!(packet,play::Clientbound::SetEquipment(data) if data.id==123&&data.slots[0].0==5)));
    assert!(out.packets.iter().any(|packet|matches!(packet,play::Clientbound::SetPassengers(data) if data.vehicle==456&&data.passengers==[123])));
    run_bridge(&mut entities, 19, &fixture("destroy_entities"));
    assert!(entities.is_empty());
    let out = run_bridge(&mut entities, 12, &fixture("spawn_player"));
    assert!(
        !out.packets
            .iter()
            .any(|packet| matches!(packet, play::Clientbound::SetPassengers(_)))
    );
    assert!(out.packets.iter().any(|packet|matches!(packet,play::Clientbound::SetEquipment(data) if data.slots.len()==1&&data.slots[0].0==0)));
}
#[test]
fn local_metadata_pickup_counts_and_malformed_updates_are_connection_local_and_atomic() {
    let mut entities = lifecycle::Entities::default();
    entities.register_local_player(123, 7).unwrap();
    let out = run_bridge(&mut entities, 28, &[123, 0, 0x12, 127]);
    assert!(
        matches!(&out.packets[0],play::Clientbound::SetEntityData(data) if data.values.contains(&(6,s::POSE,Value::Id(5))))
    );
    run_bridge(&mut entities, 14, &fixture("object_type_2"));
    run_bridge(&mut entities, 28, &fixture("item_metadata_wool"));
    let mut pickup = crate::buf::Writer::new();
    pickup.varint(1002);
    pickup.varint(123);
    let out = run_bridge(&mut entities, 13, &pickup.data);
    assert!(matches!(
        out.packets[0],
        play::Clientbound::TakeItemEntity(play::TakeItemEntity { amount: 3, .. })
    ));
    let before = entities.len();
    let bad = [123, 0x60, 0, 0, 0, 0, 127];
    let mut item = no_item;
    let mut block = no_state;
    let mut registry = |_: &str, _: &str| Err(Error::new("no registry"));
    let mut component = |_: &str| Err(Error::new("no component"));
    assert!(
        entities
            .metadata(
                &bad,
                &mut lifecycle::Callbacks {
                    item: &mut item,
                    block: &mut block,
                    registry: &mut registry,
                    component: &mut component
                }
            )
            .is_err()
    );
    assert_eq!(entities.len(), before);
    let out = run_bridge(&mut entities, 26, &fixture("hurt_status"));
    assert!(matches!(
        out.packets[0],
        play::Clientbound::HurtAnimation(_)
    ));
    entities.clear_remote();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities.uuid(123), Some(7));
    let second = lifecycle::Entities::default();
    assert!(second.is_empty());
}

#[test]
fn remaining_framed_packets_reject_every_truncation_and_trailing_byte() {
    type Parser = fn(&[u8]) -> Result<()>;
    for (name, parse) in [
        (
            "spawn_painting",
            (|data| objects::painting(data, 1, |_, _| Ok(0)).map(|_| ())) as Parser,
        ),
        (
            "spawn_xp",
            (|data| objects::experience_orb(data, 1).map(|_| ())) as Parser,
        ),
        (
            "attach_passenger",
            (|data| objects::attachment(data).map(|_| ())) as Parser,
        ),
        (
            "destroy_entities",
            (|data| objects::destroy(data).map(|_| ())) as Parser,
        ),
        (
            "effect_visible",
            (|data| effects::update(data).map(|_| ())) as Parser,
        ),
        (
            "effect_remove",
            (|data| effects::remove(data).map(|_| ())) as Parser,
        ),
        (
            "head_rotation",
            (|data| head_rotation(data).map(|_| ())) as Parser,
        ),
        (
            "entity_velocity",
            (|data| entity_velocity(data).map(|_| ())) as Parser,
        ),
    ] {
        let data = fixture(name);
        assert!(parse(&data).is_ok());
        for n in 0..data.len() {
            assert!(parse(&data[..n]).is_err(), "{name} len{n}");
        }
        let mut extra = data.clone();
        extra.push(0);
        assert!(parse(&extra).is_err(), "{name}");
    }
}

#[test]
fn horse_inventory_full_item_nbt_survives_partial_metadata_until_authoritative_material_changes() {
    use crate::item::{Component, Stack, component};
    use horizon_world::nbt::Tag;
    let mut entities = lifecycle::Entities::default();
    let mut spawn = fixture("spawn_zombie");
    spawn.splice(0..3, [123, 100]);
    run_bridge(&mut entities, 15, &spawn);
    run_bridge(&mut entities, 28, &fixture("horse_metadata"));
    let armor = raw::Slot {
        id: 419,
        count: 1,
        damage: 7,
        tag: Tag::Compound(
            [("custom".into(), Tag::String("saved armor".into()))]
                .into_iter()
                .collect(),
        ),
    };
    let saddle = raw::Slot {
        id: 329,
        count: 1,
        damage: 0,
        tag: Tag::Compound(
            [("custom".into(), Tag::String("saved saddle".into()))]
                .into_iter()
                .collect(),
        ),
    };
    let mut full_armor = Stack::new(1409, 1);
    full_armor
        .added
        .push((component::CUSTOM_DATA, Component::Nbt(armor.tag.clone())));
    let mut full_saddle = Stack::new(949, 1);
    full_saddle
        .added
        .push((component::CUSTOM_DATA, Component::Nbt(saddle.tag.clone())));
    entities
        .remember_horse_slot(123, 0, Some(&saddle), Some(full_saddle.clone()))
        .unwrap();
    entities
        .remember_horse_slot(123, 1, Some(&armor), Some(full_armor.clone()))
        .unwrap();
    // HP-only update recomputes cached flags/armor, but must retain full items.
    run_bridge(&mut entities, 28, &[123, 0x66, 0x41, 0x20, 0, 0, 127]);
    assert_eq!(
        entities.equipment_snapshot(123, 7),
        Some(Some(full_saddle.clone()))
    );
    assert_eq!(
        entities.equipment_snapshot(123, 6),
        Some(Some(full_armor.clone()))
    );
    // Repeated material/presence metadata carries no new NBT and cannot erase it.
    run_bridge(&mut entities, 28, &fixture("horse_metadata"));
    assert_eq!(
        entities.equipment_snapshot(123, 6),
        Some(Some(full_armor.clone()))
    );
    // Horse->donkey removes body; switching back can reuse the known exact armor.
    run_bridge(&mut entities, 28, &[123, 19, 1, 127]);
    assert_eq!(entities.kind(123), Some(37));
    assert_eq!(entities.equipment_snapshot(123, 6), Some(None));
    run_bridge(&mut entities, 28, &[123, 19, 0, 127]);
    assert_eq!(entities.equipment_snapshot(123, 6), Some(Some(full_armor)));
    // Genuine armor material change drops override and emits bare new material.
    run_bridge(&mut entities, 28, &[123, 0x56, 0, 0, 0, 1, 127]);
    assert_eq!(
        entities.equipment_snapshot(123, 6),
        Some(Some(Stack::new(1407, 1)))
    );
    // Genuine saddle removal drops the full saddle override.
    run_bridge(&mut entities, 28, &[123, 0x50, 0, 0, 0, 0, 127]);
    assert_eq!(entities.equipment_snapshot(123, 7), Some(None));
    let before = format!("{entities:?}");
    assert!(
        entities
            .remember_horse_slot(123, 1, Some(&armor), Some(Stack::new(1, 1)))
            .is_err()
    );
    assert_eq!(format!("{entities:?}"), before);
    // Inventory emptiness updates semantic bits too: HP-only update must not resurrect it.
    entities.remember_horse_slot(123, 1, None, None).unwrap();
    run_bridge(&mut entities, 28, &[123, 0x66, 0x41, 0x20, 0, 0, 127]);
    assert_eq!(entities.equipment_snapshot(123, 6), Some(None));
    run_bridge(&mut entities, 19, &fixture("destroy_entities"));
    run_bridge(&mut entities, 15, &spawn);
    assert_eq!(entities.equipment_snapshot(123, 6), None);
    assert_eq!(entities.equipment_snapshot(123, 7), None);
}

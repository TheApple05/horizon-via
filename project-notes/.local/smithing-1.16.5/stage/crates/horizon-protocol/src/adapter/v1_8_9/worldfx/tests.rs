use super::*;
#[path = "fixture_ids.rs"]
mod ids;
struct Ctx;
impl Context for Ctx {
    fn registry(&self, registry: &str, name: &str) -> Result<i32> {
        ids::IDS
            .iter()
            .find(|row| row.0 == registry && row.1 == name)
            .map(|row| row.2)
            .ok_or_else(|| Error::new(format!("missing fixture id{registry}:{name}")))
    }
    fn block(&self, wire: u16) -> Result<i32> {
        Ok(i32::from(wire) + 1000)
    }
    fn item(&self, id: i16, damage: i16) -> Result<Stack> {
        assert_eq!((id, damage), (35, 14));
        Ok(Stack::new(1, 1))
    }
    fn modifier_id(&self, uuid: u128) -> String {
        if uuid == 0xcb3f55d3645c4f38a4979c13a33db5cf {
            "minecraft:base_attack_damage".into()
        } else {
            format!("minecraft:{uuid:032x}")
        }
    }
}
fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/vanilla-worldfx-fixtures.json")).unwrap()
}
fn hex(data: &str) -> Vec<u8> {
    data.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(name: &str) -> Vec<u8> {
    hex(fixtures()["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap()["hex"]
        .as_str()
        .unwrap())
}
fn check(output: &Output) {
    for packet in &output.packets {
        let bytes = packet.encode();
        let mut r = Reader::new(&bytes);
        let id = r.varint().unwrap();
        play::Clientbound::decode(id, r.rest()).unwrap();
    }
}
#[test]
fn actual_attributes_preserve_named_attribute_and_canonical_modifier_identity() {
    let p = attributes(&fixture("attributes"), &Ctx).unwrap();
    assert_eq!(p.id, 123);
    assert_eq!(p.values.len(), 1);
    let a = &p.values[0];
    assert_eq!(
        a.attribute,
        Ctx.registry("minecraft:attribute", "minecraft:attack_damage")
            .unwrap()
    );
    assert_eq!(a.base, 2.);
    assert_eq!(
        (
            a.modifiers[0].id.as_str(),
            a.modifiers[0].amount,
            a.modifiers[0].operation
        ),
        ("minecraft:base_attack_damage", 7., 0)
    );
    assert_eq!(a.modifiers[1].operation, 2);
}
#[test]
fn actual_negative_explosion_uses_java_truncation_keeps_knockback_and_emits_cache_changes() {
    let out = explosion(&fixture("explosion_negative"), &Ctx).unwrap();
    check(&out);
    assert_eq!(out.block_changes, vec![([-1, 64, -2], 0), ([0, 65, -1], 0)]);
    let play::Clientbound::Explode(explosion) = &out.packets[0] else {
        panic!("explosion")
    };
    assert_eq!(explosion.center, [-0.5, 64.25, -1.75]);
    assert_eq!(explosion.radius, 4.);
    assert_eq!(explosion.block_count, 2);
    assert!(explosion.knockback.unwrap()[0] > 0.099);
}
#[test]
fn actual_named_sounds_keep_eighth_block_coordinates_pitch_quantization_and_old_category() {
    let p = sound(&fixture("sound_levelup"), &Ctx).unwrap();
    assert_eq!((p.x, p.y, p.z, p.source), (10, 512, -20, 7));
    assert_eq!(p.pitch, 78. / 63.);
    assert_eq!(p.sound.1.unwrap().0, "minecraft:entity.player.levelup");
    let p = sound(&fixture("sound_explode"), &Ctx).unwrap();
    assert_eq!(p.sound.1.unwrap().0, "minecraft:entity.generic.explode");
}
#[test]
fn every_actual_vanilla_particle_writer_is_handled_or_explicitly_unsupported() {
    let data = fixtures();
    let mut count = 0;
    for row in data["fixtures"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let Some(id) = name
            .strip_prefix("particle_")
            .and_then(|id| id.parse::<i32>().ok())
        else {
            continue;
        };
        let wire = hex(row["hex"].as_str().unwrap());
        let result = particles::read(&wire, &Ctx);
        if matches!(id, 8 | 28 | 32 | 40) {
            assert!(result.unwrap_err().to_string().contains("unsupported"));
        } else {
            let p = result.unwrap();
            check(&Output {
                packets: vec![play::Clientbound::LevelParticles(p.clone())],
                ..Default::default()
            });
            if matches!(id, 37 | 38) {
                assert_eq!(
                    p.particle.data,
                    Some(crate::entity_data::ParticleData::State(1017))
                );
            }
            count += 1;
        }
    }
    assert_eq!(count, 38);
}
#[test]
fn actual_maps_lightning_and_block_break_are_pinned_typed_packets() {
    let p = map(&fixture("map_patch"), &Ctx).unwrap();
    assert_eq!((p.map_id, p.scale, p.locked), (7, 2, false));
    let patch = p.patch.unwrap();
    assert_eq!(
        (
            patch.start_x,
            patch.start_y,
            patch.width,
            patch.height,
            patch.colors
        ),
        (3, 4, 2, 1, vec![9, 10])
    );
    let p = map(&fixture("map_clear_icons"), &Ctx).unwrap();
    assert_eq!(p.decorations, Some(vec![]));
    assert_eq!(p.patch, None);
    let p = lightning(&fixture("lightning"), 77, &Ctx).unwrap();
    assert_eq!((p.uuid, p.position), (77, [1.25, 64., -2.5]));
    let p = level_event(&fixture("effect_block_break"), &Ctx).unwrap();
    assert_eq!((p.kind, p.pos, p.data), (2001, [3, 64, -9], 1017));
}
#[test]
fn actual_auxiliary_sound_ids_keep_old_choices_and_do_not_become_new_event_numbers() {
    for (id, expected, volume) in [
        (1000, "block.lever.click", 1.),
        (1001, "block.lever.click", 1.),
        (1002, "entity.arrow.shoot", 1.),
        (1003, "block.wooden_door.open", 1.),
        (1004, "block.fire.extinguish", 0.5),
        (1006, "block.wooden_door.close", 1.),
        (1007, "entity.ghast.warn", 10.),
        (1008, "entity.ghast.shoot", 10.),
        (1009, "entity.ghast.shoot", 2.),
        (1010, "entity.zombie.attack_wooden_door", 2.),
        (1011, "entity.zombie.attack_iron_door", 2.),
        (1012, "entity.zombie.break_wooden_door", 2.),
        (1014, "entity.wither.shoot", 2.),
        (1015, "entity.bat.takeoff", 0.05),
        (1016, "entity.zombie.infect", 2.),
        (1017, "entity.zombie_villager.converted", 2.),
        (1020, "block.anvil.destroy", 1.),
        (1021, "block.anvil.use", 1.),
        (1022, "block.anvil.land", 0.3),
    ] {
        let out = auxiliary_event(&fixture(&format!("event_{id}")), &Ctx).unwrap();
        check(&out);
        let play::Clientbound::Sound(sound) = &out.packets[0] else {
            panic!()
        };
        assert_eq!(
            sound.sound.1.as_ref().unwrap().0,
            format!("minecraft:{expected}")
        );
        assert_eq!((sound.x, sound.y, sound.z), (28, 516, -68));
        assert_eq!(sound.volume, volume);
        assert!(sound.pitch.is_finite() && sound.pitch > 0.);
    }
    for id in [1005, 2000, 2002, 2005] {
        assert!(
            auxiliary_event(&fixture(&format!("event_{id}")), &Ctx)
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }
    for id in [1013, 1018] {
        assert!(
            auxiliary_event(&fixture(&format!("event_{id}")), &Ctx)
                .unwrap()
                .known_noop
                .is_some()
        );
        assert!(
            auxiliary_event(&fixture(&format!("global_event_{id}")), &Ctx)
                .unwrap_err()
                .to_string()
                .contains("listener-relative")
        );
    }
    assert!(
        auxiliary_event(&fixture("global_event_2001"), &Ctx)
            .unwrap()
            .known_noop
            .is_some()
    );
}
#[test]
fn sleep_survives_other_metadata_until_wake_and_noop_requires_strict_consumption() {
    let mut state = State::default();
    let out = state
        .clientbound(10, &fixture("use_bed"), &Ctx, |_| 1)
        .unwrap()
        .unwrap();
    check(&out);
    let mut update = play::Clientbound::SetEntityData(play::SetEntityData {
        id: 123,
        values: vec![(6, s::POSE, Value::Id(5))],
    });
    state.filter_entity_packet(&mut update);
    let play::Clientbound::SetEntityData(data) = update else {
        panic!()
    };
    assert!(data.values.contains(&(6, s::POSE, Value::Id(2))));
    let out = state
        .clientbound(11, &fixture("animation_2"), &Ctx, |_| 1)
        .unwrap()
        .unwrap();
    check(&out);
    let mut update = play::Clientbound::SetEntityData(play::SetEntityData {
        id: 123,
        values: vec![(6, s::POSE, Value::Id(5))],
    });
    state.filter_entity_packet(&mut update);
    let play::Clientbound::SetEntityData(data) = update else {
        panic!()
    };
    assert_eq!(data.values, vec![(6, s::POSE, Value::Id(5))]);
    for (id, name) in [(11, "animation_3"), (73, "entity_nbt_noop")] {
        let out = state
            .clientbound(id, &fixture(name), &Ctx, |_| 1)
            .unwrap()
            .unwrap();
        assert!(out.packets.is_empty());
        assert!(out.known_noop.is_some());
    }
    assert!(
        state
            .clientbound(70, &fixture("play_compression"), &Ctx, |_| 1)
            .unwrap_err()
            .to_string()
            .contains("compression")
    );
}
#[test]
fn owned_supported_framing_rejects_all_truncations_and_trailing_bytes() {
    for (id, name) in [
        (10, "use_bed"),
        (11, "animation_0"),
        (32, "attributes"),
        (39, "explosion_negative"),
        (40, "effect_block_break"),
        (41, "sound_explode"),
        (42, "particle_37"),
        (44, "lightning"),
        (52, "map_patch"),
        (73, "entity_nbt_noop"),
    ] {
        let data = fixture(name);
        for n in 0..data.len() {
            assert!(
                State::default()
                    .clientbound(id, &data[..n], &Ctx, |_| 1)
                    .is_err(),
                "{name}len{n}"
            );
        }
        let mut extra = data.clone();
        extra.push(0);
        assert!(
            State::default()
                .clientbound(id, &extra, &Ctx, |_| 1)
                .is_err(),
            "{name}"
        );
    }
}

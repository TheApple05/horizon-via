use super::*;
#[path = "fixture_ids.rs"]
mod ids;
struct Ctx {
    raw: u16,
    listener: [f64; 3],
}
impl Default for Ctx {
    fn default() -> Self {
        Self {
            raw: 59 << 4,
            listener: [0.; 3],
        }
    }
}
impl base::Context for Ctx {
    fn registry(&self, registry: &str, name: &str) -> Result<i32> {
        if registry == "minecraft:jukebox_song" {
            return [
                "13", "cat", "blocks", "chirp", "far", "mall", "mellohi", "stal", "strad", "ward",
                "11", "wait",
            ]
            .iter()
            .position(|value| name == format!("minecraft:{value}"))
            .map(|i| i as i32)
            .ok_or_else(|| Error::new("unknown test song"));
        }
        ids::IDS
            .iter()
            .find(|row| row.0 == registry && row.1 == name)
            .map(|row| row.2)
            .ok_or_else(|| Error::new("unknown test registry value"))
    }
    fn block(&self, _: u16) -> Result<i32> {
        Err(Error::new("unexpected block callback"))
    }
    fn item(&self, id: i16, damage: i16) -> Result<crate::item::Stack> {
        assert_eq!(id, 373);
        Ok(crate::item::Stack::new(i32::from(damage), 1))
    }
    fn modifier_id(&self, _: u128) -> String {
        unreachable!()
    }
}
impl Context for Ctx {
    fn raw_block(&self, _: [i32; 3]) -> Result<u16> {
        Ok(self.raw)
    }
    fn listener_position(&self) -> Result<[f64; 3]> {
        Ok(self.listener)
    }
}
fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/vanilla-extra-fixtures.json")).unwrap()
}
fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
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
fn get(name: &str, ctx: &Ctx) -> Output {
    clientbound(
        if name.starts_with("particle_") {
            42
        } else {
            40
        },
        &fixture(name),
        ctx,
    )
    .unwrap()
    .unwrap()
}
fn decode(packets: &[play::Clientbound]) {
    for packet in packets {
        let bytes = packet.encode();
        let mut r = Reader::new(&bytes);
        let id = r.varint().unwrap();
        play::Clientbound::decode(id, r.rest()).unwrap();
    }
}
#[test]
fn actual_all_32768_potion_damage_results_and_high_aliases_match_the_client_color_routines() {
    let bytes = hex(include_str!("fixtures/potion-oracle.hex").trim());
    assert_eq!(bytes.len(), 32768 * 4);
    for (damage, bytes) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let value = u32::from_le_bytes(*bytes);
        assert_eq!(
            potion_color(damage as i32),
            ((value & 0xff_ffff) as i32, value >> 24 != 0)
        );
    }
    for row in fixtures()["alias_checks"].as_array().unwrap() {
        assert_eq!(
            potion_color(row[0].as_i64().unwrap() as i32),
            (row[1].as_i64().unwrap() as i32, row[2].as_bool().unwrap())
        );
    }
    assert_eq!(potion_color(8197), (0xf82423, true));
    assert_eq!(potion_color(8204), (0x430a09, true));
    assert_eq!(potion_color(8193), (0xcd5cab, false));
}
#[test]
fn actual_bounds_table_preserves_old_wheat_height_and_explicit_neighbour_dependency() {
    let bytes = hex(include_str!("fixtures/bounds-oracle.hex").trim());
    assert_eq!(bytes.len() % 11, 0);
    for row in bytes.as_chunks::<11>().0 {
        let raw = u16::from_le_bytes(row[..2].try_into().unwrap());
        let height = f64::from_le_bytes(row[2..10].try_into().unwrap());
        let flags = row[10];
        let actual = bounds::lookup(raw).unwrap();
        assert_eq!(
            actual,
            bounds::Bounds {
                height,
                air: flags & 1 != 0,
                requires_neighbors: flags & 2 != 0
            }
        );
    }
    for age in 0..8 {
        assert_eq!(bounds::lookup((59 << 4) | age).unwrap().height, 0.25);
    }
    assert!(bounds::lookup(54 << 4).unwrap().requires_neighbors);
    assert!(bounds::lookup(0).unwrap().air);
    assert!(bounds::lookup((59 << 4) | 15).is_err());
}
#[test]
fn records_resolve_declared_bare_song_ids_and_stop_only_the_target_position() {
    for (name, kind, data) in [
        ("event_1005_2256", 1010, 0),
        ("event_1005_2267", 1010, 11),
        ("event_1005_0", 1011, 0),
        ("event_1005_1", 1011, 0),
    ] {
        let out = get(name, &Ctx::default());
        decode(&out.packets);
        let play::Clientbound::LevelEvent(p) = &out.packets[0] else {
            panic!()
        };
        assert_eq!(
            (p.kind, p.data, p.pos, p.global),
            (kind, data, [3, 64, -9], false)
        );
    }
}
#[test]
fn smoke_uses_old_three_by_three_direction_grid_and_ten_individual_particles() {
    let out = get("event_2000_4", &Ctx::default());
    assert_eq!(out.packets.len(), 10);
    decode(&out.packets);
    for p in &out.packets {
        let play::Clientbound::LevelParticles(p) = p else {
            panic!()
        };
        assert_eq!((p.pos[0], p.pos[2]), (3.5, -8.5));
        assert!((64.25..64.75).contains(&p.pos[1]));
        assert_eq!((p.count, p.max_speed), (0, [1.; 3]));
    }
    for direction in 0..9 {
        let out = get(&format!("event_2000_{direction}"), &Ctx::default());
        assert_eq!(out.packets.len(), 10);
        decode(&out.packets);
    }
}
#[test]
fn potion_splash_keeps_old_integer_origin_crumb_damage_color_and_smash_sound() {
    let out = get("event_2002_8197", &Ctx::default());
    assert_eq!(out.packets.len(), 109);
    decode(&out.packets);
    for packet in &out.packets[..8] {
        let play::Clientbound::LevelParticles(p) = packet else {
            panic!()
        };
        assert_eq!(p.pos, [3., 64., -9.]);
        let Some(ParticleData::Item(item)) = &p.particle.data else {
            panic!()
        };
        assert_eq!(item.item, 8197);
    }
    for packet in &out.packets[8..108] {
        let play::Clientbound::LevelParticles(p) = packet else {
            panic!()
        };
        assert_eq!(p.pos[1], 64.3);
        let Some(ParticleData::ColorPower(color, power)) = p.particle.data else {
            panic!()
        };
        assert!((0.0..4.0).contains(&power));
        assert!((186..=248).contains(&((color >> 16) & 255)));
    }
    let play::Clientbound::Sound(sound) = out.packets.last().unwrap() else {
        panic!()
    };
    assert_eq!(
        sound.sound.1.as_ref().unwrap().0,
        "minecraft:entity.splash_potion.break"
    );
    assert_eq!((sound.x, sound.y, sound.z, sound.source), (28, 516, -68, 7));
}
#[test]
fn bonemeal_uses_fifteen_default_sparks_old_bounds_and_no_added_sound_or_modern_spread() {
    let out = get("event_2005_0", &Ctx::default());
    assert_eq!(out.packets.len(), 15);
    decode(&out.packets);
    for packet in &out.packets {
        let play::Clientbound::LevelParticles(p) = packet else {
            panic!()
        };
        assert!((3.0..4.0).contains(&p.pos[0]));
        assert!((64.0..64.25).contains(&p.pos[1]));
        assert!((-9.0..-8.0).contains(&p.pos[2]));
    }
    assert_eq!(get("event_2005_5", &Ctx::default()).packets.len(), 5);
    assert!(
        get(
            "event_2005_5",
            &Ctx {
                raw: 0,
                ..Default::default()
            }
        )
        .known_noop
        .is_some()
    );
    let out = get(
        "event_2005_5",
        &Ctx {
            raw: 54 << 4,
            ..Default::default()
        },
    );
    assert!(out.packets.is_empty() && out.omitted_cosmetic.is_some());
}
#[test]
fn global_boss_sounds_use_camera_base_position_and_old_two_block_direction() {
    for (id, volume) in [(1013, 1.), (1018, 5.)] {
        let out = get(&format!("global_{id}"), &Ctx::default());
        decode(&out.packets);
        let play::Clientbound::Sound(p) = &out.packets[0] else {
            panic!()
        };
        assert_eq!((p.x, p.y, p.z, p.volume), (4, 8, 12, volume));
        let out = get(
            &format!("global_{id}"),
            &Ctx {
                listener: [10., 20., 30.],
                ..Default::default()
            },
        );
        let play::Clientbound::Sound(p) = &out.packets[0] else {
            panic!()
        };
        assert_eq!((p.x, p.y, p.z), (80, 160, 240));
    }
}
#[test]
fn removed_selectors_are_aliases_noops_or_explicit_cosmetic_omissions_and_colored_counts_expand() {
    let ctx = Ctx::default();
    let out = get("particle_8", &ctx);
    decode(&out.packets);
    assert_eq!(out.packets.len(), 1);
    let play::Clientbound::LevelParticles(p) = &out.packets[0] else {
        panic!()
    };
    assert_eq!(
        p.particle.id,
        base::Context::registry(&ctx, "minecraft:particle_type", "minecraft:mycelium").unwrap()
    );
    for id in [28, 32] {
        let out = get(&format!("particle_{id}"), &ctx);
        assert!(out.packets.is_empty() && out.omitted_cosmetic.is_some());
    }
    assert!(get("particle_40", &ctx).known_noop.is_some());
    for id in [15, 16, 30] {
        let out = get(&format!("particle_{id}"), &ctx);
        assert_eq!(out.packets.len(), 3);
        decode(&out.packets);
        assert_ne!(out.packets[0], out.packets[1]);
    }
}
#[test]
fn all_actual_owned_fixtures_reject_every_truncation_and_trailing_byte() {
    for row in fixtures()["fixtures"].as_array().unwrap() {
        let id = row["id"].as_i64().unwrap() as i32;
        let bytes = hex(row["hex"].as_str().unwrap());
        for length in 0..bytes.len() {
            assert!(
                clientbound(id, &bytes[..length], &Ctx::default()).is_err(),
                "{}len{length}",
                row["name"]
            );
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(clientbound(id, &trailing, &Ctx::default()).is_err());
    }
}

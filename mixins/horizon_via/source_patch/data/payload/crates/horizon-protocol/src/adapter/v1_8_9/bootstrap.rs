//! A consumer-side configuration for a protocol which has no configuration phase.
use super::{Join, facts, ids};
use crate::{
    Error, Result,
    buf::Reader,
    packets::{configuration as cfg, play},
    registry,
};
use horizon_world::nbt::Tag;
use std::sync::{Arc, OnceLock};

pub const DIMENSIONS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

fn compound<const N: usize>(values: [(&str, Tag); N]) -> Tag {
    Tag::Compound(
        values
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}
fn string(value: &str) -> Tag {
    Tag::String(value.into())
}

pub fn dimension(raw: i32) -> Result<usize> {
    match raw {
        0 => Ok(0),
        -1 => Ok(1),
        1 => Ok(2),
        _ => Err(Error::new("unknown legacy dimension")),
    }
}

pub fn spawn(join: &Join) -> Result<play::SpawnInfo> {
    let dimension = dimension(i32::from(join.dimension))?;
    Ok(play::SpawnInfo {
        dimension_type: dimension as i32,
        dimension: DIMENSIONS[dimension].into(),
        seed: 0,
        game_type: i32::from(join.mode),
        previous_game_type: None,
        debug: false,
        flat: join.level_type == "flat",
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    })
}

pub fn login(join: &Join, online_mode: bool) -> Result<play::Login> {
    Ok(play::Login {
        player_id: join.entity,
        hardcore: join.hardcore,
        levels: DIMENSIONS.iter().map(|s| (*s).into()).collect(),
        max_players: i32::from(join.max_players),
        chunk_radius: 10,
        simulation_distance: 10,
        reduced_debug_info: join.reduced_debug,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn(join)?,
        online_mode,
        enforces_secure_chat: false,
    })
}

pub fn biome_id(raw: u8) -> Result<i32> {
    facts::BIOMES
        .binary_search_by_key(&raw, |v| v.0)
        .map(|i| i as i32)
        .map_err(|_| Error::new(format!("unknown protocol47 biome {raw}")))
}

/// These IDs are local to the consumer, never sent to or represented as server-synchronized IDs.
pub fn registries() -> Vec<cfg::RegistryData> {
    let dimensions = DIMENSIONS
        .iter()
        .enumerate()
        .map(|(index, &name)| {
            let mut data = compound([
                ("min_y", Tag::Int(0)),
                ("height", Tag::Int(256)),
                (
                    "logical_height",
                    Tag::Int(if index == 1 { 128 } else { 256 }),
                ),
                ("has_skylight", Tag::Byte(i8::from(index == 0))),
                ("has_ceiling", Tag::Byte(i8::from(index == 1))),
                (
                    "ambient_light",
                    Tag::Float(if index == 1 { 0.1f32 } else { 0.0f32 }.to_bits()),
                ),
                (
                    "default_clock",
                    string(if index == 2 {
                        "minecraft:the_end"
                    } else {
                        "minecraft:overworld"
                    }),
                ),
            ]);
            if index == 1
                && let Tag::Compound(fields) = &mut data
            {
                fields.remove(&"default_clock".into());
            }
            cfg::RegistryEntry {
                id: name.into(),
                data: Some(data),
            }
        })
        .collect();
    let biomes = facts::BIOMES
        .iter()
        .map(|&(raw, temperature, rainfall)| {
            // Distinct local names retain the actual old climate even for biomes later merged by vanilla.
            let mut effects = vec![
                ("fog_color", Tag::Int(12638463)),
                ("water_color", Tag::Int(0x3f76e4)),
                ("water_fog_color", Tag::Int(0x050533)),
                ("sky_color", Tag::Int(sky_color(temperature))),
            ];
            if matches!(raw, 6 | 134) {
                effects.push(("grass_color_modifier", string("swamp")));
                effects.push(("foliage_color", Tag::Int(0x6a7039)));
                effects.push(("water_color", Tag::Int(0xe0ffae)));
            } else if matches!(raw, 29 | 157) {
                effects.push(("grass_color_modifier", string("dark_forest")));
            } else if matches!(raw, 37..=39 | 165..=167) {
                effects.push(("grass_color", Tag::Int(0x90814d)));
                effects.push(("foliage_color", Tag::Int(0x9e814d)));
            }
            cfg::RegistryEntry {
                id: format!("horizon_via:legacy_biome_{raw}"),
                data: Some(compound([
                    ("temperature", Tag::Float(temperature.to_bits())),
                    ("downfall", Tag::Float(rainfall.to_bits())),
                    (
                        "has_precipitation",
                        Tag::Byte(i8::from(
                            !matches!(raw, 2 | 8 | 9 | 17 | 35..=39 | 130 | 163..=167),
                        )),
                    ),
                    (
                        "effects",
                        Tag::Compound(effects.into_iter().map(|(k, v)| (k.into(), v)).collect()),
                    ),
                ])),
            }
        })
        .collect();
    let mut out = vec![
        cfg::RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: dimensions,
        },
        cfg::RegistryData {
            registry: "minecraft:worldgen/biome".into(),
            entries: biomes,
        },
    ];
    for (registry, names) in dynamic_names() {
        out.push(cfg::RegistryData {
            registry: registry.into(),
            entries: names
                .iter()
                .map(|&name| cfg::RegistryEntry {
                    id: name.into(),
                    data: None,
                })
                .collect(),
        });
    }
    out
}

fn sky_color(temperature: f32) -> i32 {
    let value = (temperature / 3.0).clamp(-1.0, 1.0);
    let h = (0.62222224 - value * 0.05) * 6.0;
    let s = 0.5 + value * 0.1;
    let f = h - h.floor();
    let p = 1.0 - s;
    let q = 1.0 - s * f;
    let t = 1.0 - s * (1.0 - f);
    let [r, g, b] = match h.floor() as i32 % 6 {
        0 => [1.0, t, p],
        1 => [q, 1.0, p],
        2 => [p, 1.0, t],
        3 => [p, q, 1.0],
        4 => [t, p, 1.0],
        _ => [1.0, p, q],
    };
    ((r * 255.0 + 0.5) as i32) << 16 | ((g * 255.0 + 0.5) as i32) << 8 | (b * 255.0 + 0.5) as i32
}

fn dynamic_names() -> [(&'static str, &'static [&'static str]); 7] {
    use super::super::v1_20_1_ids as names;
    [
        ("minecraft:enchantment", names::ENCHANTMENT_NAMES),
        ("minecraft:painting_variant", names::PAINTING_VARIANT_NAMES),
        ("minecraft:banner_pattern", names::BANNER_PATTERN_NAMES),
        ("minecraft:instrument", names::INSTRUMENT_NAMES),
        (
            "minecraft:world_clock",
            &["minecraft:overworld", "minecraft:the_end"],
        ),
        (
            "minecraft:cat_variant",
            &["minecraft:black", "minecraft:red", "minecraft:siamese"],
        ),
        (
            "minecraft:jukebox_song",
            &[
                "minecraft:13",
                "minecraft:cat",
                "minecraft:blocks",
                "minecraft:chirp",
                "minecraft:far",
                "minecraft:mall",
                "minecraft:mellohi",
                "minecraft:stal",
                "minecraft:strad",
                "minecraft:ward",
                "minecraft:11",
                "minecraft:wait",
            ],
        ),
    ]
}

pub(crate) fn names() -> Arc<super::super::ids::LegacyRegistries> {
    static NAMES: OnceLock<Arc<super::super::ids::LegacyRegistries>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut names = super::super::ids::LegacyRegistries::new();
            for &(registry, entries) in ids::CURRENT_NAMES.iter().chain(facts::OTHER_NAMES.iter()) {
                names.insert(
                    registry.into(),
                    registry::Ids::from_names(entries.iter().map(|&s| s.into()).collect()),
                );
            }
            for (registry, entries) in dynamic_names() {
                names.insert(
                    registry.into(),
                    registry::Ids::from_names(entries.iter().map(|&s| s.into()).collect()),
                );
            }
            Arc::new(names)
        })
        .clone()
}

pub fn packet(encoded: Vec<u8>) -> (i32, Vec<u8>) {
    let mut r = Reader::new(&encoded);
    let id = r.varint().expect("typed packet ID");
    (id, r.rest().to_vec())
}

/// Old servers advertise no Brigadier tree. A greedy argument requests their actual suggestions.
pub fn commands() -> play::Clientbound {
    let mut w = crate::buf::Writer::new();
    w.varint(2);
    w.u8(0);
    w.varint(1);
    w.varint(1); // Root with the one server-backed argument.
    w.u8(0x16);
    w.varint(0);
    w.string("command");
    w.varint(5); // Pinned builtin brigadier:string, verified against the official report.
    w.varint(2); // StringArgumentType.greedyString.
    w.string("minecraft:ask_server");
    w.varint(0);
    play::Clientbound::Unknown(crate::ids::play::clientbound::COMMANDS, w.data)
}

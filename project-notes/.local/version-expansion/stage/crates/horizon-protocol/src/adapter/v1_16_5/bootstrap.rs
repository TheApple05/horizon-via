//! Protocol 754's embedded registries and consumer-only configuration.
use super::*;
use horizon_world::datafix::dynamic as d;

pub(super) struct Join {
    pub login: play::Login,
    pub registries: Vec<cfg::RegistryData>,
    pub dimensions: Vec<cfg::RegistryEntry>,
    pub biomes: Vec<i32>,
}

pub(super) fn join(payload: &[u8], online: bool) -> Result<Join> {
    let mut r = Reader::new(payload);
    let player_id = r.i32()?;
    let hardcore = boolean(&mut r)?;
    let mode = r.u8()?;
    let previous = r.i8()?;
    if mode > 3 || !(-1..=3).contains(&previous) {
        return Err(Error::new("invalid 1.16.5 game mode"));
    }
    let n = r.count(1024)?;
    let levels = (0..n)
        .map(|_| r.string(32767))
        .collect::<Result<Vec<_>>>()?;
    let registry = r.named_nbt()?;
    let dimension = r.named_nbt()?;
    let world = r.string(32767)?;
    let seed = r.i64()?;
    let max_players = r.varint()?;
    let view = r.varint()?;
    if !(2..=32).contains(&view) || max_players < 0 {
        return Err(Error::new("invalid 1.16.5 login distances"));
    }
    let reduced_debug_info = boolean(&mut r)?;
    let show_death_screen = boolean(&mut r)?;
    let debug = boolean(&mut r)?;
    let flat = boolean(&mut r)?;
    finish(&r)?;
    let (mut registries, biomes) = source_registries(&registry)?;
    let dimension_type = dimension_id(&registries, &dimension)?;
    let dimensions = registries
        .iter()
        .find(|r| r.registry == "minecraft:dimension_type")
        .unwrap()
        .entries
        .clone();
    upgrade_registries(&mut registries);
    // The source has no damage-type or modern dynamic registries. Use the same
    // consumer-local defaults as the independently implemented 1.8 bootstrap.
    for fallback in super::super::v1_8_9::bootstrap::registries() {
        if !registries.iter().any(|v| v.registry == fallback.registry) {
            registries.push(fallback);
        }
    }
    if let Some(songs) = registries
        .iter_mut()
        .find(|v| v.registry == "minecraft:jukebox_song")
    {
        songs.entries.push(cfg::RegistryEntry {
            id: "minecraft:pigstep".into(),
            data: None,
        });
    }
    if let Some(cats) = registries
        .iter_mut()
        .find(|v| v.registry == "minecraft:cat_variant")
    {
        cats.entries = CAT_VARIANTS
            .iter()
            .map(|name| cfg::RegistryEntry {
                id: (*name).into(),
                data: None,
            })
            .collect();
    }
    Ok(Join {
        login: play::Login {
            player_id,
            hardcore,
            levels,
            max_players,
            chunk_radius: view,
            simulation_distance: view,
            reduced_debug_info,
            show_death_screen,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type,
                dimension: world,
                seed,
                game_type: i32::from(mode),
                previous_game_type: (previous >= 0).then_some(i32::from(previous)),
                debug,
                flat,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: online,
            enforces_secure_chat: false,
        },
        registries,
        dimensions,
        biomes,
    })
}

fn source_registries(tag: &Tag) -> Result<(Vec<cfg::RegistryData>, Vec<i32>)> {
    let Tag::Compound(values) = tag else {
        return Err(Error::new("invalid old registry compound"));
    };
    if values.len() > 128 {
        return Err(Error::new("too many old registries"));
    }
    let mut registries = Vec::with_capacity(values.len());
    let mut biomes = vec![];
    for (kind, value) in values {
        let kind = kind.to_string_lossy();
        let Some(Tag::List(entries)) = value.get("value") else {
            return Err(Error::new("old registry has no entries"));
        };
        if entries.is_empty() || entries.len() > 65536 {
            return Err(Error::new("invalid old registry size"));
        }
        let mut ordered = std::collections::BTreeMap::new();
        let mut names = std::collections::BTreeSet::new();
        for entry in entries {
            let Some(Tag::Int(id)) = entry.get("id") else {
                return Err(Error::new("old registry entry has no id"));
            };
            if !(0..65536).contains(id) {
                return Err(Error::new("old registry id outside budget"));
            }
            let Some(Tag::String(name)) = entry.get("name") else {
                return Err(Error::new("old registry entry has no name"));
            };
            let name = name.to_string_lossy();
            if !names.insert(name.clone())
                || ordered
                    .insert(
                        *id,
                        cfg::RegistryEntry {
                            id: name,
                            data: entry.get("element").cloned(),
                        },
                    )
                    .is_some()
            {
                return Err(Error::new("duplicate old registry id or name"));
            }
        }
        if kind == "minecraft:worldgen/biome" {
            biomes = vec![-1; *ordered.last_key_value().unwrap().0 as usize + 1];
            for (local, source) in ordered.keys().enumerate() {
                biomes[*source as usize] = local as i32;
            }
        }
        registries.push(cfg::RegistryData {
            registry: kind,
            entries: ordered.into_values().collect(),
        });
    }
    if biomes.is_empty() {
        return Err(Error::new("old login has no biome registry"));
    }
    Ok((registries, biomes))
}

pub(super) fn dimension_id(registries: &[cfg::RegistryData], dimension: &Tag) -> Result<i32> {
    let registry = registries
        .iter()
        .find(|v| v.registry == "minecraft:dimension_type")
        .ok_or_else(|| Error::new("1.16.5 login has no dimension registry"))?;
    registry
        .entries
        .iter()
        .position(|entry| entry.data.as_ref() == Some(dimension))
        .map(|n| n as i32)
        .ok_or_else(|| Error::new("1.16.5 dimension is outside its declared registry"))
}

pub(super) fn upgrade_registries(registries: &mut [cfg::RegistryData]) {
    for registry in registries {
        for entry in &mut registry.entries {
            let Some(data) = &mut entry.data else {
                continue;
            };
            match registry.registry.as_str() {
                "minecraft:dimension_type" => {
                    d::set(data, "min_y", Tag::Int(0));
                    d::set(data, "height", Tag::Int(256));
                    if d::as_bool(data.get("natural"), false) {
                        d::set(data, "default_clock", d::string("minecraft:overworld"));
                    } else if entry.id == "minecraft:the_end" {
                        d::set(data, "default_clock", d::string("minecraft:the_end"));
                    }
                }
                "minecraft:worldgen/biome" => {
                    let rain = d::as_str(data.get("precipitation")).as_deref() != Some("none");
                    d::set(data, "has_precipitation", Tag::Byte(i8::from(rain)));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimension_identity_uses_the_declared_element() {
        let element = Tag::compound([("logical_height", Tag::Int(128))]);
        let registries = vec![cfg::RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![cfg::RegistryEntry {
                id: "custom:dimension".into(),
                data: Some(element.clone()),
            }],
        }];
        assert_eq!(dimension_id(&registries, &element).unwrap(), 0);
        assert!(dimension_id(&registries, &Tag::End).is_err());
    }
    #[test]
    fn sparse_biomes_have_an_explicit_dense_map_and_reject_duplicate_ids() {
        let entry = |id, name: &str| {
            Tag::compound([
                ("id", Tag::Int(id)),
                ("name", d::string(name)),
                ("element", d::empty_map()),
            ])
        };
        let make = |rows| {
            Tag::compound([(
                "minecraft:worldgen/biome",
                Tag::compound([("value", Tag::List(rows))]),
            )])
        };
        let (registries, ids) = source_registries(&make(vec![
            entry(173, "minecraft:basalt_deltas"),
            entry(1, "minecraft:plains"),
        ]))
        .unwrap();
        assert_eq!(
            registries[0]
                .entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["minecraft:plains", "minecraft:basalt_deltas"]
        );
        assert_eq!((ids[0], ids[1], ids[172], ids[173]), (-1, 0, -1, 1));
        assert!(
            source_registries(&make(vec![
                entry(1, "minecraft:plains"),
                entry(1, "minecraft:desert")
            ]))
            .is_err()
        );
    }
}

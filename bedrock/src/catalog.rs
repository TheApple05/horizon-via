// SPDX-License-Identifier: MIT

//! Versioned CC0 native identities. Rendering and gameplay mappings are separate.

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const REVISION: &str = "1e041bf547b7c12e94d2b998f68d5ed146975742";
const SHA256: &str = "1982736a2ed020c718c9f53cfe1d03f173b8e2142b8287d3d04616a78e821e1a";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "(String, Value)")]
pub enum Property {
    Byte(i8),
    Int(i32),
    String(String),
}

impl TryFrom<(String, Value)> for Property {
    type Error = &'static str;
    fn try_from((kind, value): (String, Value)) -> Result<Self, Self::Error> {
        match (kind.as_str(), value) {
            ("b", Value::Number(n)) => n
                .as_i64()
                .and_then(|n| i8::try_from(n).ok())
                .map(Self::Byte),
            ("i", Value::Number(n)) => n
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .map(Self::Int),
            ("s", Value::String(s)) if s.len() <= 256 => Some(Self::String(s)),
            _ => None,
        }
        .ok_or("invalid typed native property")
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockState {
    pub ordinal: u32,
    pub hash: u32,
    pub name: String,
    pub properties: BTreeMap<String, Property>,
    pub state_version: i32,
    pub light: [u8; 2],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemIdentity {
    pub name: String,
    pub id: i32,
    /// Raw native measurement; not an effective gameplay stack rule.
    pub raw_max_stack: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    schema: u32,
    version: String,
    protocol: i32,
    license: String,
    native_revision: String,
    states: Vec<BlockState>,
    items: Vec<ItemIdentity>,
    biomes: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeIds {
    Ordinal,
    Hashed,
}

#[derive(Debug)]
pub struct Catalog {
    states: Vec<BlockState>,
    hashes: HashMap<u32, usize>,
    items: HashMap<i32, ItemIdentity>,
    biomes: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCatalog(&'static str);

impl fmt::Display for InvalidCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for InvalidCatalog {}

fn identifier(s: &str) -> bool {
    let Some((namespace, name)) = s.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !name.is_empty()
        && s.len() <= 256
        && namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_./-".contains(&c))
}

impl Catalog {
    /// Loads the developer extractor's pinned identity-only output.
    /// Authenticates the complete extracted file before interpreting any fields.
    pub fn load(bytes: &[u8]) -> Result<Self, InvalidCatalog> {
        if bytes.len() > MAX_BYTES {
            return Err(InvalidCatalog("native facts exceed byte limit"));
        }
        if format!("{:x}", Sha256::digest(bytes)) != SHA256 {
            return Err(InvalidCatalog("native facts digest differs"));
        }
        let facts: Facts = serde_json::from_slice(bytes)
            .map_err(|_| InvalidCatalog("invalid native facts JSON"))?;
        if facts.schema != 1
            || facts.version != bedrock_protocol::GAME_VERSION
            || facts.protocol != bedrock_protocol::PROTOCOL_VERSION
            || facts.license != "CC0-1.0"
            || facts.native_revision != REVISION
        {
            return Err(InvalidCatalog("native facts provenance/version differs"));
        }
        if facts.states.len() != 23960 || facts.items.len() != 2080 || facts.biomes.len() != 89 {
            return Err(InvalidCatalog("native facts domain differs"));
        }
        Self::validate(facts.states, facts.items, facts.biomes)
    }

    fn validate(
        states: Vec<BlockState>,
        items: Vec<ItemIdentity>,
        biomes: BTreeMap<String, u32>,
    ) -> Result<Self, InvalidCatalog> {
        let mut hashes = HashMap::with_capacity(states.len());
        let mut identities = HashSet::with_capacity(states.len());
        for (index, state) in states.iter().enumerate() {
            if state.ordinal as usize != index
                || !identifier(&state.name)
                || state.state_version <= 0
                || state.light.iter().any(|&v| v > 15)
                || state.properties.len() > 64
                || state.properties.keys().any(|k| {
                    k.is_empty()
                        || k.len() > 256
                        || !k.bytes().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || b"_:./-".contains(&c)
                        })
                })
                || hashes.insert(state.hash, index).is_some()
                || !identities.insert((&state.name, &state.properties))
            {
                return Err(InvalidCatalog("invalid or duplicate native block state"));
            }
        }
        drop(identities);
        let mut by_id = HashMap::with_capacity(items.len());
        let mut names = HashSet::new();
        for item in items {
            if !identifier(&item.name)
                || !(1..=64).contains(&item.raw_max_stack)
                || !names.insert(item.name.clone())
                || by_id.insert(item.id, item).is_some()
            {
                return Err(InvalidCatalog("invalid or duplicate native item"));
            }
        }
        if biomes.keys().any(|s| !identifier(s))
            || biomes.values().any(|&v| v > i32::MAX as u32)
            || biomes.values().collect::<HashSet<_>>().len() != biomes.len()
        {
            return Err(InvalidCatalog("invalid or duplicate native biome"));
        }
        Ok(Self {
            states,
            hashes,
            items: by_id,
            biomes,
        })
    }

    /// Hashes use the entire unsigned wire domain, including its high bit.
    /// Unknown/custom IDs remain unknown rather than becoming air or a wrong block.
    pub fn block(&self, mode: RuntimeIds, wire_id: u32) -> Option<&BlockState> {
        let index = match mode {
            RuntimeIds::Ordinal => usize::try_from(wire_id).ok()?,
            RuntimeIds::Hashed => *self.hashes.get(&wire_id)?,
        };
        self.states.get(index)
    }

    pub fn item(&self, wire_id: i32) -> Option<&ItemIdentity> {
        self.items.get(&wire_id)
    }
    pub fn biome(&self, name: &str) -> Option<u32> {
        self.biomes.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(ordinal: u32, hash: u32, name: &str) -> BlockState {
        BlockState {
            ordinal,
            hash,
            name: name.into(),
            properties: BTreeMap::new(),
            state_version: 18168865,
            light: [0, 0],
        }
    }

    #[test]
    fn unsigned_hash_and_ordinal_domains_do_not_alias() {
        let c = Catalog::validate(
            vec![
                state(0, 3690217760, "minecraft:air"),
                state(1, 2150698529, "minecraft:stone"),
            ],
            vec![ItemIdentity {
                name: "minecraft:stone".into(),
                id: -1,
                raw_max_stack: 64,
            }],
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            c.block(RuntimeIds::Hashed, 3690217760).unwrap().name,
            "minecraft:air"
        );
        assert_eq!(
            c.block(RuntimeIds::Ordinal, 1).unwrap().name,
            "minecraft:stone"
        );
        assert!(c.block(RuntimeIds::Ordinal, 3690217760).is_none());
        assert!(c.block(RuntimeIds::Hashed, 1).is_none());
        assert!(c.block(RuntimeIds::Hashed, u32::MAX).is_none());
        assert_eq!(c.item(-1).unwrap().name, "minecraft:stone");
        assert!(c.item(1).is_none());
    }

    #[test]
    fn typed_properties_preserve_signed_bytes_and_reject_coercion() {
        let values: BTreeMap<String, Property> =
            serde_json::from_str(r#"{"b":["b",-1],"i":["i",-7],"s":["s","x"]}"#).unwrap();
        assert_eq!(values["b"], Property::Byte(-1));
        assert_eq!(values["i"], Property::Int(-7));
        assert_eq!(values["s"], Property::String("x".into()));
        for invalid in [
            r#"["b",128]"#,
            r#"["b",true]"#,
            r#"["i",2147483648]"#,
            r#"["i",1.5]"#,
            r#"["s",0]"#,
            r#"["x",0]"#,
        ] {
            assert!(
                serde_json::from_str::<Property>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn malformed_state_domains_are_rejected() {
        for rows in [
            vec![state(1, 1, "minecraft:air")],
            vec![state(0, 1, "minecraft:air"), state(1, 1, "minecraft:stone")],
            vec![state(0, 1, "minecraft:air"), state(1, 2, "minecraft:air")],
            vec![state(0, 1, "minecraft:Air")],
        ] {
            assert!(Catalog::validate(rows, vec![], BTreeMap::new()).is_err());
        }
        let mut bad = state(0, 1, "minecraft:air");
        bad.light = [0, 16];
        assert!(Catalog::validate(vec![bad], vec![], BTreeMap::new()).is_err());
    }

    #[test]
    fn duplicate_item_ids_and_biome_ids_are_rejected() {
        let items = vec![
            ItemIdentity {
                name: "minecraft:stone".into(),
                id: -1,
                raw_max_stack: 64,
            },
            ItemIdentity {
                name: "minecraft:dirt".into(),
                id: -1,
                raw_max_stack: 64,
            },
        ];
        assert!(Catalog::validate(vec![], items, BTreeMap::new()).is_err());
        let biomes = BTreeMap::from([
            ("minecraft:plains".into(), 1),
            ("minecraft:desert".into(), 1),
        ]);
        assert!(Catalog::validate(vec![], vec![], biomes).is_err());
    }
}

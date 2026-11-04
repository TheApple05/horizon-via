// SPDX-License-Identifier: MIT

//! Deliberately limited semantic rules for ordinary dry natural terrain.
//! Every result must exist exactly in the host's current block registry.
use crate::{
    catalog::{BlockState, Property},
    translation::{HostBlockId, Mapper},
};

/// Exact name and complete-property lookup, never a default-state lookup.
/// The registry and these results must remain stable during one translation.
pub trait HostRegistry {
    fn resolve(&self, name: &str, properties: &[(&str, &str)]) -> Option<HostBlockId>;
}

pub struct TerrainRules<'a, R> {
    registry: &'a R,
}
impl<'a, R: HostRegistry> TerrainRules<'a, R> {
    pub fn new(registry: &'a R) -> Self {
        Self { registry }
    }
}
impl<R: HostRegistry> Mapper for TerrainRules<'_, R> {
    fn map_cell(&mut self, layers: &[&BlockState]) -> Option<HostBlockId> {
        let (base, additional) = layers.split_first()?;
        if layers.len() > 8
            || additional
                .iter()
                .any(|s| s.name != "minecraft:air" || !s.properties.is_empty())
        {
            return None;
        }
        // A whitelist prevents a new/custom or context-dependent same-name
        // block from acquiring guessed semantics merely because lookup works.
        if propertyless_ground(&base.name) {
            if !base.properties.is_empty() {
                return None;
            }
            return self.registry.resolve(&base.name, &[]);
        }
        if axial_ground(&base.name) && base.properties.len() == 1 {
            let Property::String(axis) = base.properties.get("pillar_axis")? else {
                return None;
            };
            if matches!(axis.as_str(), "x" | "y" | "z") {
                return self.registry.resolve(&base.name, &[("axis", axis)]);
            }
        }
        None
    }
}

fn propertyless_ground(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air"
            | "minecraft:stone"
            | "minecraft:granite"
            | "minecraft:polished_granite"
            | "minecraft:diorite"
            | "minecraft:polished_diorite"
            | "minecraft:andesite"
            | "minecraft:polished_andesite"
            | "minecraft:dirt"
            | "minecraft:coarse_dirt"
            | "minecraft:sand"
            | "minecraft:red_sand"
            | "minecraft:gravel"
            | "minecraft:clay"
            | "minecraft:cobblestone"
            | "minecraft:mossy_cobblestone"
            | "minecraft:obsidian"
            | "minecraft:crying_obsidian"
            | "minecraft:netherrack"
            | "minecraft:end_stone"
            | "minecraft:soul_sand"
            | "minecraft:soul_soil"
            | "minecraft:blackstone"
            | "minecraft:polished_blackstone"
            | "minecraft:gilded_blackstone"
            | "minecraft:calcite"
            | "minecraft:tuff"
            | "minecraft:smooth_basalt"
            | "minecraft:cobbled_deepslate"
            | "minecraft:coal_ore"
            | "minecraft:iron_ore"
            | "minecraft:copper_ore"
            | "minecraft:gold_ore"
            | "minecraft:diamond_ore"
            | "minecraft:emerald_ore"
            | "minecraft:lapis_ore"
            | "minecraft:nether_gold_ore"
            | "minecraft:deepslate_coal_ore"
            | "minecraft:deepslate_iron_ore"
            | "minecraft:deepslate_copper_ore"
            | "minecraft:deepslate_gold_ore"
            | "minecraft:deepslate_diamond_ore"
            | "minecraft:deepslate_emerald_ore"
            | "minecraft:deepslate_lapis_ore"
    )
}

fn axial_ground(name: &str) -> bool {
    if matches!(
        name,
        "minecraft:basalt"
            | "minecraft:polished_basalt"
            | "minecraft:deepslate"
            | "minecraft:bamboo_block"
            | "minecraft:stripped_bamboo_block"
    ) {
        return true;
    }
    let Some(local) = name.strip_prefix("minecraft:") else {
        return false;
    };
    let local = local.strip_prefix("stripped_").unwrap_or(local);
    for suffix in ["_log", "_wood"] {
        if let Some(wood) = local.strip_suffix(suffix) {
            return matches!(
                wood,
                "oak"
                    | "spruce"
                    | "birch"
                    | "jungle"
                    | "acacia"
                    | "dark_oak"
                    | "mangrove"
                    | "cherry"
                    | "pale_oak"
            );
        }
    }
    matches!(
        local,
        "crimson_stem" | "crimson_hyphae" | "warped_stem" | "warped_hyphae"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Registry;
    impl HostRegistry for Registry {
        fn resolve(&self, name: &str, properties: &[(&str, &str)]) -> Option<HostBlockId> {
            let id = match (name, properties) {
                ("minecraft:air", []) => 405,
                ("minecraft:stone", []) => 927,
                ("minecraft:oak_log", [("axis", "x")]) => 117,
                ("minecraft:oak_log", [("axis", "y")]) => 118,
                ("minecraft:oak_log", [("axis", "z")]) => 119,
                _ => return None,
            };
            Some(HostBlockId::from_registry_id(id))
        }
    }
    fn state(name: &str, properties: &[(&str, Property)]) -> BlockState {
        BlockState {
            ordinal: 17,
            hash: 0xf000_0001,
            name: name.into(),
            properties: properties
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
            state_version: 18168865,
            light: [0, 0],
        }
    }
    #[test]
    fn native_numbers_are_never_host_ids_and_axis_is_explicit() {
        let mut mapper = TerrainRules::new(&Registry);
        let stone = state("minecraft:stone", &[]);
        let air = state("minecraft:air", &[]);
        assert_eq!(mapper.map_cell(&[&stone, &air]).unwrap().get(), 927);
        for (axis, expected) in [("x", 117), ("y", 118), ("z", 119)] {
            let log = state(
                "minecraft:oak_log",
                &[("pillar_axis", Property::String(axis.into()))],
            );
            assert_eq!(mapper.map_cell(&[&log]).unwrap().get(), expected);
        }
        // Recognized semantic rule with no exact host state still refuses.
        assert!(mapper.map_cell(&[&state("minecraft:clay", &[])]).is_none());
    }
    #[test]
    fn unsupported_or_malformed_semantics_cannot_fall_back() {
        let mut mapper = TerrainRules::new(&Registry);
        let stone = state("minecraft:stone", &[]);
        let water = state("minecraft:water", &[("liquid_depth", Property::Int(0))]);
        assert!(mapper.map_cell(&[]).is_none());
        assert!(mapper.map_cell(&[&stone; 9]).is_none());
        assert!(mapper.map_cell(&[&stone, &water]).is_none());
        assert!(
            mapper
                .map_cell(&[
                    &stone,
                    &state("minecraft:air", &[("extra", Property::Byte(0))])
                ])
                .is_none()
        );
        for name in [
            "minecraft:grass_block",
            "minecraft:oak_leaves",
            "custom:stone",
        ] {
            assert!(mapper.map_cell(&[&state(name, &[])]).is_none());
        }
        for value in [Property::Int(0), Property::String("north".into())] {
            let log = state("minecraft:oak_log", &[("pillar_axis", value)]);
            assert!(mapper.map_cell(&[&log]).is_none());
        }
        let log = state(
            "minecraft:oak_log",
            &[
                ("pillar_axis", Property::String("x".into())),
                ("extra", Property::Byte(0)),
            ],
        );
        assert!(mapper.map_cell(&[&log]).is_none());
    }
}

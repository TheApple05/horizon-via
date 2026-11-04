// SPDX-License-Identifier: MIT

//! Atomic cell-order/identity handoff to a host-owned semantic block mapper.
//! This does not create a world, merge fluids implicitly or supply server light.
use crate::{
    catalog::{BlockState, Catalog, RuntimeIds},
    storage::Subchunk,
};
use std::{collections::HashMap, fmt};

/// Constructed by the host only after resolving a state in its own registry.
/// A Bedrock wire ID has no implicit conversion to this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBlockId(u32);
impl HostBlockId {
    pub fn from_registry_id(id: u32) -> Self {
        Self(id)
    }
    pub fn get(self) -> u32 {
        self.0
    }
}
/// Lookups, including canonical air, must stay stable during one translation.
pub trait Palette {
    fn resolve(&self, wire_id: u32) -> Option<&BlockState>;
    fn air(&self) -> Option<&BlockState>;
}
pub struct VanillaPalette<'a> {
    catalog: &'a Catalog,
    mode: RuntimeIds,
    air: Option<&'a BlockState>,
}
impl<'a> VanillaPalette<'a> {
    pub fn new(catalog: &'a Catalog, mode: RuntimeIds) -> Self {
        Self {
            catalog,
            mode,
            air: catalog.air(),
        }
    }
}
impl Palette for VanillaPalette<'_> {
    fn resolve(&self, wire_id: u32) -> Option<&BlockState> {
        self.catalog.block(self.mode, wire_id)
    }
    fn air(&self) -> Option<&BlockState> {
        self.air
    }
}
pub trait Mapper {
    /// Map every native layer together, including secondary air/water layers.
    /// Implementations only resolve semantics and must remain stable during one
    /// translation; apply world changes only after a complete result. None means
    /// unsupported semantics, not permission to substitute air/default state.
    fn map_cell(&mut self, layers: &[&BlockState]) -> Option<HostBlockId>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPosition {
    pub section_y: i8,
    pub x: u8,
    pub y: u8,
    pub z: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingFailure {
    MissingContext,
    Unsupported,
}
pub trait ContextMapper {
    /// Resolve only against one stable neighborhood snapshot/generation. Never
    /// write the host world here: a later cell can still fail. The caller binds
    /// connection/dimension/chunk ownership and rejects stale results on apply.
    fn map_cell_at(
        &mut self,
        position: CellPosition,
        layers: &[&BlockState],
    ) -> Result<HostBlockId, MappingFailure>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationError {
    ExcessiveLayers,
    MissingAir,
    UnknownRuntime {
        layer: usize,
        wire_id: u32,
        x: u8,
        y: u8,
        z: u8,
    },
    UnsupportedLayers {
        x: u8,
        y: u8,
        z: u8,
    },
    MissingContext {
        x: u8,
        y: u8,
        z: u8,
    },
}
impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExcessiveLayers => write!(f, "too many native storage layers"),
            Self::MissingAir => write!(f, "empty storage requires a canonical native air identity"),
            Self::UnknownRuntime {
                layer,
                wire_id,
                x,
                y,
                z,
            } => write!(
                f,
                "unknown native runtime ID {wire_id} in layer {layer} at {x},{y},{z}"
            ),
            Self::UnsupportedLayers { x, y, z } => {
                write!(f, "unsupported native block/layer semantics at {x},{y},{z}")
            }
            Self::MissingContext { x, y, z } => write!(
                f,
                "native block mapping needs neighborhood context at {x},{y},{z}"
            ),
        }
    }
}
impl std::error::Error for TranslationError {}

/// Host X-fast order `(y << 8) | (z << 4) | x`, distinct from the wire's Y-fast
/// order. Biomes, dimension admission and block entities are separate handoffs.
pub struct MappedSection {
    section_y: i8,
    cells: Box<[HostBlockId; 4096]>,
}
impl MappedSection {
    pub fn section_y(&self) -> i8 {
        self.section_y
    }
    pub fn cells(&self) -> &[HostBlockId; 4096] {
        &self.cells
    }
    /// Receiving terrain is not evidence that it has been lit or presented.
    pub fn requires_local_lighting(&self) -> bool {
        true
    }
}
/// The caller decodes against the negotiated dimension bounds first. Complete
/// output is returned only after all cells/layers resolve. Applying it to the
/// host world belongs to the caller. At most 4096 distinct cell tuples enter
/// the per-call cache.
pub fn translate_subchunk(
    section: &Subchunk,
    palette: &impl Palette,
    mapper: &mut impl Mapper,
) -> Result<MappedSection, TranslationError> {
    translate_with(section, palette, true, |_, layers| {
        mapper.map_cell(layers).ok_or(MappingFailure::Unsupported)
    })
}
/// Position-dependent results are never cached by native identity. Even empty
/// storage calls the mapper for every cell. Local Y becomes absolute only in
/// caller context: `i32::from(section_y)*16 + i32::from(y)`.
pub fn translate_contextual_subchunk(
    section: &Subchunk,
    palette: &impl Palette,
    mapper: &mut impl ContextMapper,
) -> Result<MappedSection, TranslationError> {
    translate_with(section, palette, false, |position, layers| {
        mapper.map_cell_at(position, layers)
    })
}
fn translate_with(
    section: &Subchunk,
    palette: &impl Palette,
    cache_host: bool,
    mut map: impl FnMut(CellPosition, &[&BlockState]) -> Result<HostBlockId, MappingFailure>,
) -> Result<MappedSection, TranslationError> {
    if section.layers.len() > 8 {
        return Err(TranslationError::ExcessiveLayers);
    }
    let mut cells = Box::new([HostBlockId(0); 4096]);
    let mut native = HashMap::<[u32; 8], Vec<&BlockState>>::new();
    if section.layers.is_empty() {
        let air = palette
            .air()
            .filter(|s| s.name == "minecraft:air" && s.properties.is_empty())
            .ok_or(TranslationError::MissingAir)?;
        native.insert([0; 8], vec![air]);
    }
    let mut cache = HashMap::<[u32; 8], HostBlockId>::new();
    for y in 0..16u8 {
        for z in 0..16u8 {
            for x in 0..16u8 {
                let mut key = [0; 8];
                for (i, layer) in section.layers.iter().enumerate() {
                    key[i] = layer.at(x, y, z).expect("bounded storage coordinate");
                }
                let cached = if cache_host { cache.get(&key) } else { None };
                let host = match cached {
                    Some(host) => *host,
                    None => {
                        let layers = match native.entry(key) {
                            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                let mut layers = Vec::with_capacity(section.layers.len());
                                for (i, &wire_id) in
                                    key.iter().take(section.layers.len()).enumerate()
                                {
                                    layers.push(palette.resolve(wire_id).ok_or(
                                        TranslationError::UnknownRuntime {
                                            layer: i,
                                            wire_id,
                                            x,
                                            y,
                                            z,
                                        },
                                    )?);
                                }
                                entry.insert(layers)
                            }
                        };
                        let host = map(
                            CellPosition {
                                section_y: section.section_y,
                                x,
                                y,
                                z,
                            },
                            layers,
                        )
                        .map_err(|error| match error {
                            MappingFailure::MissingContext => {
                                TranslationError::MissingContext { x, y, z }
                            }
                            MappingFailure::Unsupported => {
                                TranslationError::UnsupportedLayers { x, y, z }
                            }
                        })?;
                        if cache_host {
                            cache.insert(key, host);
                        }
                        host
                    }
                };
                cells[(usize::from(y) << 8) | (usize::from(z) << 4) | usize::from(x)] = host;
            }
        }
    }
    Ok(MappedSection {
        section_y: section.section_y,
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::decode_subchunk;
    use std::collections::BTreeMap;
    fn state(name: &str) -> BlockState {
        BlockState {
            ordinal: 0,
            hash: 0,
            name: name.into(),
            properties: BTreeMap::new(),
            state_version: 18168865,
            light: [0, 0],
        }
    }
    struct States(HashMap<u32, BlockState>);
    impl Palette for States {
        fn resolve(&self, id: u32) -> Option<&BlockState> {
            self.0.get(&id)
        }
        fn air(&self) -> Option<&BlockState> {
            self.0.values().find(|s| s.name == "minecraft:air")
        }
    }
    struct Waterlogged {
        calls: usize,
        reject: bool,
    }
    impl Mapper for Waterlogged {
        fn map_cell(&mut self, layers: &[&BlockState]) -> Option<HostBlockId> {
            self.calls += 1;
            if self.reject {
                return None;
            }
            match layers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["minecraft:air", "minecraft:water"] => Some(HostBlockId::from_registry_id(901)),
                ["minecraft:oak_fence", "minecraft:water"] => {
                    Some(HostBlockId::from_registry_id(707))
                }
                ["minecraft:air"] => Some(HostBlockId::from_registry_id(405)),
                _ => None,
            }
        }
    }
    fn signed(out: &mut Vec<u8>, value: u32) {
        let mut zig = (value << 1) ^ (((value as i32) >> 31) as u32);
        while zig >= 128 {
            out.push((zig as u8) | 128);
            zig >>= 7;
        }
        out.push(zig as u8);
    }
    fn two_layers() -> (Subchunk, States) {
        // Original network fixture: only coordinate2,7,11 uses the second
        // palette entry. The expected host offset below is independent of
        // this writer and follows the host's documented X-fast axes.
        let ids = [0x8000_0001, 0xf000_0002, 0xe000_0003];
        let mut wire = vec![9, 2, (-4i8) as u8, 3];
        let slot = (2 << 8) | (11 << 4) | 7;
        for word in 0..128 {
            wire.extend_from_slice(
                &(if word == slot / 32 {
                    1u32 << (slot % 32)
                } else {
                    0
                })
                .to_le_bytes(),
            );
        }
        signed(&mut wire, 2);
        signed(&mut wire, ids[0]);
        signed(&mut wire, ids[1]);
        wire.push(1);
        signed(&mut wire, ids[2]);
        (
            decode_subchunk(&wire, -4..20).unwrap(),
            States(HashMap::from([
                (ids[0], state("minecraft:air")),
                (ids[1], state("minecraft:oak_fence")),
                (ids[2], state("minecraft:water")),
            ])),
        )
    }
    #[test]
    fn signed_y_high_unsigned_ids_layers_and_host_axes_are_preserved() {
        let (section, palette) = two_layers();
        let mut mapper = Waterlogged {
            calls: 0,
            reject: false,
        };
        let mapped = translate_subchunk(&section, &palette, &mut mapper).unwrap();
        assert_eq!(mapped.section_y(), -4);
        assert!(mapped.requires_local_lighting());
        assert_eq!(mapped.cells()[1970].get(), 707); // 7*256 +11*16 +2.
        assert_eq!(mapped.cells().iter().filter(|c| c.get() == 707).count(), 1);
        assert!(mapped.cells().iter().all(|c| matches!(c.get(), 707 | 901)));
        assert_eq!(mapper.calls, 2); // Per unique complete layer tuple, not per cell.
    }
    #[test]
    fn unknown_identity_or_unsupported_layer_semantics_never_produce_a_section() {
        let (section, mut palette) = two_layers();
        let mut mapper = Waterlogged {
            calls: 0,
            reject: true,
        };
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::UnsupportedLayers { .. })
        ));
        mapper.reject = false;
        palette.0.remove(&0xe000_0003);
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::UnknownRuntime {
                layer: 1,
                wire_id: 0xe000_0003,
                ..
            })
        ));
    }
    #[test]
    fn late_unknown_and_all_eight_ordered_layers_are_checked() {
        let (section, mut palette) = two_layers();
        palette.0.remove(&0xf000_0002);
        let mut mapper = Waterlogged {
            calls: 0,
            reject: false,
        };
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::UnknownRuntime {
                layer: 0,
                wire_id: 0xf000_0002,
                x: 2,
                y: 7,
                z: 11
            })
        ));
        assert_eq!(mapper.calls, 1); // Earlier cells resolved, but no section escaped.
        struct AllLayers;
        impl Mapper for AllLayers {
            fn map_cell(&mut self, layers: &[&BlockState]) -> Option<HostBlockId> {
                if layers.len() != 8 || layers[7].name != "minecraft:water" {
                    return None;
                }
                let names = layers[..7]
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>();
                if names.iter().all(|n| *n == "minecraft:air") {
                    Some(HostBlockId::from_registry_id(700))
                } else if names.iter().all(|n| *n == "minecraft:oak_fence") {
                    Some(HostBlockId::from_registry_id(708))
                } else {
                    None
                }
            }
        }
        let (mut section, palette) = two_layers();
        let secondary = section.layers[1].clone();
        section.layers = vec![section.layers[0].clone(); 7];
        section.layers.push(secondary);
        let mapped = translate_subchunk(&section, &palette, &mut AllLayers).unwrap();
        assert_eq!(mapped.cells()[1970].get(), 708);
        assert_eq!(mapped.cells().iter().filter(|c| c.get() == 708).count(), 1);
    }
    #[test]
    fn empty_storage_requires_explicit_air_mapping_and_all_layer_bounds_hold() {
        let (mut section, mut palette) = two_layers();
        section.layers.clear();
        let mut mapper = Waterlogged {
            calls: 0,
            reject: false,
        };
        assert!(
            translate_subchunk(&section, &palette, &mut mapper)
                .unwrap()
                .cells()
                .iter()
                .all(|c| c.get() == 405)
        );
        palette.0.remove(&0x8000_0001);
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::MissingAir)
        ));
        palette.0.insert(0x8000_0001, state("minecraft:air"));
        palette
            .0
            .get_mut(&0x8000_0001)
            .unwrap()
            .properties
            .insert("not_canonical".into(), crate::catalog::Property::Byte(1));
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::MissingAir)
        ));
        let (mut section, palette) = two_layers();
        section.layers = vec![section.layers[0].clone(); 9];
        assert!(matches!(
            translate_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::ExcessiveLayers)
        ));
    }
    struct PerPosition {
        calls: usize,
        missing: bool,
    }
    impl ContextMapper for PerPosition {
        fn map_cell_at(
            &mut self,
            position: CellPosition,
            layers: &[&BlockState],
        ) -> Result<HostBlockId, MappingFailure> {
            self.calls += 1;
            assert_eq!(position.section_y, -4);
            if self.missing && (position.x, position.y, position.z) == (2, 7, 11) {
                return Err(MappingFailure::MissingContext);
            }
            let id = if layers.len() == 1 && layers[0].name == "minecraft:air" {
                405
            } else {
                100 + u32::from(position.x)
            };
            Ok(HostBlockId::from_registry_id(id))
        }
    }
    #[test]
    fn contextual_results_do_not_reuse_identity_only_cache_or_empty_fill() {
        let (mut section, palette) = two_layers();
        let mut mapper = PerPosition {
            calls: 0,
            missing: false,
        };
        let mapped = translate_contextual_subchunk(&section, &palette, &mut mapper).unwrap();
        assert_eq!(mapped.cells()[0].get(), 100);
        assert_eq!(mapped.cells()[1].get(), 101);
        assert_eq!(mapped.cells()[1970].get(), 102);
        assert_eq!(mapper.calls, 4096);
        section.layers.clear();
        mapper.calls = 0;
        let mapped = translate_contextual_subchunk(&section, &palette, &mut mapper).unwrap();
        assert!(mapped.cells().iter().all(|id| id.get() == 405));
        assert_eq!(mapper.calls, 4096);
    }
    #[test]
    fn missing_context_is_retryable_and_late_unknown_still_wins_before_mapping() {
        let (section, mut palette) = two_layers();
        let mut mapper = PerPosition {
            calls: 0,
            missing: true,
        };
        assert!(matches!(
            translate_contextual_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::MissingContext { x: 2, y: 7, z: 11 })
        ));
        assert_eq!(mapper.calls, 1971);
        palette.0.remove(&0xf000_0002);
        mapper.calls = 0;
        assert!(matches!(
            translate_contextual_subchunk(&section, &palette, &mut mapper),
            Err(TranslationError::UnknownRuntime {
                layer: 0,
                x: 2,
                y: 7,
                z: 11,
                ..
            })
        ));
        assert_eq!(mapper.calls, 1970);
    }
}

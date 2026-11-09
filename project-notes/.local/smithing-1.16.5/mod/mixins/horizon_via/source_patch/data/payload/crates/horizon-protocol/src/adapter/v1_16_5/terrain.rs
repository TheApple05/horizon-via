//! 754 chunk palettes and independently delivered light, from vanilla packet writers.
use super::*;
use crate::chunk::{BlockEntityInfo, ChunkData, LightData, Paletted, Section, Strategy};
use horizon_world::datafix::{self, dynamic as d, schema::Ty};
#[path = "state_properties.rs"]
mod properties;

#[derive(Clone)]
struct Column {
    sections: Vec<Section>,
    entities: Vec<BlockEntityInfo>,
}

#[derive(Default)]
pub(super) struct World {
    columns: HashMap<(i32, i32), Column>,
    lights: HashMap<(i32, i32), LightData>,
    biome_count: usize,
    biome_map: Vec<i32>,
}

fn empty_light() -> LightData {
    LightData {
        sky_mask: vec![],
        block_mask: vec![],
        empty_sky_mask: vec![],
        empty_block_mask: vec![],
        sky: vec![],
        block: vec![],
    }
}

fn bitset(value: u32) -> Vec<u8> {
    let mut bytes = value.to_le_bytes().to_vec();
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    bytes
}

impl World {
    pub(super) fn clear(&mut self) {
        self.columns.clear();
        self.lights.clear();
    }
    pub(super) fn set_biomes(&mut self, map: Vec<i32>) -> Result<()> {
        if map.is_empty() || map.len() > 65536 {
            return Err(Error::new("invalid 1.16.5 biome registry size"));
        }
        self.biome_count = map.iter().filter(|id| **id >= 0).count();
        if self.biome_count == 0 {
            return Err(Error::new("empty old biome registry"));
        }
        self.biome_map = map;
        Ok(())
    }
    pub(super) fn forget(&mut self, x: i32, z: i32) {
        self.columns.remove(&(x, z));
        self.lights.remove(&(x, z));
    }

    pub(super) fn apply_updates(&mut self, packets: &[play::Clientbound]) -> Result<()> {
        let mut changes = HashMap::<(i32, i32, usize), Vec<(usize, i32)>>::new();
        let mut add = |pos: [i32; 3], state: i32| -> Result<()> {
            if !(0..256).contains(&pos[1]) || state < 0 {
                return Err(Error::new("invalid old authoritative block update"));
            }
            let index = (((pos[1] & 15) << 8) | ((pos[2] & 15) << 4) | (pos[0] & 15)) as usize;
            changes
                .entry((pos[0] >> 4, pos[2] >> 4, pos[1] as usize / 16))
                .or_default()
                .push((index, state));
            Ok(())
        };
        for packet in packets {
            match packet {
                play::Clientbound::BlockUpdate(p) => add(p.pos, p.state)?,
                play::Clientbound::SectionBlocksUpdate(p) => {
                    for change in &p.changes {
                        let pos = [
                            p.section[0] * 16 + ((change >> 8) & 15) as i32,
                            p.section[1] * 16 + (change & 15) as i32,
                            p.section[2] * 16 + ((change >> 4) & 15) as i32,
                        ];
                        add(
                            pos,
                            i32::try_from(change >> 12)
                                .map_err(|_| Error::new("invalid old mapped block state"))?,
                        )?;
                    }
                }
                _ => {}
            }
        }
        for ((x, z, y), updates) in changes {
            let Some(column) = self.columns.get_mut(&(x, z)) else {
                continue;
            };
            let section = &mut column.sections[y];
            let mut values = (0..4096).map(|i| section.states.get(i)).collect::<Vec<_>>();
            for (index, state) in updates {
                let changed = block(values[index]) != block(state);
                values[index] = state;
                let px = ((index & 15) << 4) | ((index >> 4) & 15);
                let by = (y * 16 + (index >> 8)) as i16;
                if changed {
                    column
                        .entities
                        .retain(|e| e.packed_xz as u8 != px as u8 || e.y != by);
                }
            }
            section.non_empty = values.iter().filter(|id| !air(**id)).count() as i16;
            section.fluids = values.iter().filter(|id| fluid(**id)).count() as i16;
            section.states = Paletted::from_values(&values, Strategy::Blocks, 16);
        }
        for packet in packets {
            if let play::Clientbound::BlockEntityData(p) = packet
                && let Some(column) = self.columns.get_mut(&(p.pos[0] >> 4, p.pos[2] >> 4))
            {
                let xz = (((p.pos[0] & 15) << 4) | (p.pos[2] & 15)) as i8;
                column
                    .entities
                    .retain(|e| e.packed_xz != xz || i32::from(e.y) != p.pos[1]);
                column.entities.push(BlockEntityInfo {
                    packed_xz: xz,
                    y: p.pos[1] as i16,
                    kind: p.kind,
                    tag: p.tag.clone(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn light(&mut self, payload: &[u8]) -> Result<play::Clientbound> {
        let mut r = Reader::new(payload);
        let x = r.varint()?;
        let z = r.varint()?;
        if self.lights.len() >= 4096 && !self.lights.contains_key(&(x, z)) {
            return Err(Error::new("1.16.5 light cache limit exceeded"));
        }
        boolean(&mut r)?; // Trust edges is a light-engine hint, not a plane.
        let mut masks = [0u32; 4];
        for mask in &mut masks {
            let raw = r.varint()?;
            if raw < 0 || raw & !0x3ffff != 0 {
                return Err(Error::new("invalid 1.16.5 light mask"));
            }
            *mask = raw as u32;
        }
        if masks[0] & masks[2] != 0 || masks[1] & masks[3] != 0 {
            return Err(Error::new("overlapping 1.16.5 light masks"));
        }
        let mut layers = Vec::new();
        for mask in &masks[..2] {
            let mut planes = Vec::new();
            for _ in 0..mask.count_ones() {
                let plane = r.byte_array(2048)?;
                if plane.len() != 2048 {
                    return Err(Error::new("invalid 1.16.5 light plane size"));
                }
                planes.push(plane);
            }
            layers.push(planes);
        }
        finish(&r)?;
        let update = LightData {
            sky_mask: bitset(masks[0]),
            block_mask: bitset(masks[1]),
            empty_sky_mask: bitset(masks[2]),
            empty_block_mask: bitset(masks[3]),
            sky: layers.remove(0),
            block: layers.remove(0),
        };
        let existing = self.lights.entry((x, z)).or_insert_with(empty_light);
        let old = existing.layers(18);
        let new = update.layers(18);
        let mut merged = empty_light();
        for (i, ((sky, block), (new_sky, new_block))) in old.into_iter().zip(new).enumerate() {
            for (value, mask, empty, planes) in [
                (
                    new_sky.or(sky),
                    &mut merged.sky_mask,
                    &mut merged.empty_sky_mask,
                    &mut merged.sky,
                ),
                (
                    new_block.or(block),
                    &mut merged.block_mask,
                    &mut merged.empty_block_mask,
                    &mut merged.block,
                ),
            ] {
                if let Some(value) = value {
                    let target = if value.iter().all(|b| *b == 0) {
                        empty
                    } else {
                        mask
                    };
                    if target.len() <= i / 8 {
                        target.resize(i / 8 + 1, 0);
                    }
                    target[i / 8] |= 1 << (i % 8);
                    if value.iter().any(|b| *b != 0) {
                        planes.push(value);
                    }
                }
            }
        }
        *existing = merged;
        Ok(play::Clientbound::LightUpdate(play::LightUpdate {
            x,
            z,
            light: update,
        }))
    }

    pub(super) fn chunk(&mut self, payload: &[u8]) -> Result<play::Clientbound> {
        let mut r = Reader::new(payload);
        let x = r.i32()?;
        let z = r.i32()?;
        if self.columns.len() >= 4096 && !self.columns.contains_key(&(x, z)) {
            return Err(Error::new("1.16.5 chunk cache limit exceeded"));
        }
        let full = boolean(&mut r)?;
        let mask = r.varint()?;
        if mask < 0 || mask & !65535 != 0 {
            return Err(Error::new("invalid 1.16.5 chunk mask"));
        }
        let heights = r.named_nbt()?;
        let biomes = if full {
            let n = r.count(1024)?;
            if n != 1024 {
                return Err(Error::new("invalid 1.16.5 biome array size"));
            }
            Some(
                (0..n)
                    .map(|_| {
                        let raw = r.varint()?;
                        usize::try_from(raw)
                            .ok()
                            .and_then(|id| self.biome_map.get(id))
                            .copied()
                            .filter(|id| *id >= 0)
                            .ok_or_else(|| Error::new("old biome id is not declared"))
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            None
        };
        let buffer = r.byte_array(2 * 1024 * 1024)?;
        let n = r.count(65536)?;
        let mut entities = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let mut tag = r.named_nbt()?;
            let id = d::as_str(tag.get("id"))
                .ok_or_else(|| Error::new("old block entity has no type"))?;
            let raw = tables::BLOCK_ENTITY_NAMES
                .iter()
                .position(|name| *name == id)
                .ok_or_else(|| Error::new("unknown 1.16.5 block entity"))?;
            let bx = d::as_int(tag.get("x"), i32::MIN);
            let y = d::as_int(tag.get("y"), i32::MIN);
            let bz = d::as_int(tag.get("z"), i32::MIN);
            if bx >> 4 != x || bz >> 4 != z || !(0..256).contains(&y) {
                return Err(Error::new("old block entity is outside its chunk"));
            }
            if id == "minecraft:bed" {
                bed_payload(&tag)?;
                continue;
            }
            let kind = up(tables::BLOCK_ENTITY_UP, raw as i32)?;
            datafix::update(Ty::BlockEntity, &mut tag, 2586, datafix::CURRENT);
            entities.push(BlockEntityInfo {
                packed_xz: (((bx & 15) << 4) | (bz & 15)) as i8,
                y: y as i16,
                kind,
                tag,
            });
        }
        finish(&r)?;
        let empty = Section {
            non_empty: 0,
            fluids: 0,
            states: Paletted::from_values(&[0; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 8),
        };
        let mut column = if full {
            Column {
                sections: vec![empty; 16],
                entities: vec![],
            }
        } else {
            self.columns
                .get(&(x, z))
                .cloned()
                .ok_or_else(|| Error::new("partial 1.16.5 chunk precedes full column"))?
        };
        let mut sections = Reader::new(&buffer);
        let mut allocator_padding = 0;
        for y in 0..16 {
            if mask & (1 << y) != 0 {
                let non_empty = sections.i16()?;
                if !(0..=4096).contains(&non_empty) {
                    return Err(Error::new("invalid old section block count"));
                }
                if !matches!(buffer.get(sections.position()), Some(4..=8 | 15)) {
                    return Err(Error::new("invalid 1.16.5 palette width"));
                }
                allocator_padding += usize::from(buffer[sections.position()] == 15);
                let palette = Paletted::read_prefixed(&mut sections, Strategy::Blocks, 15)?;
                let values = (0..4096)
                    .map(|i| up(tables::STATE_UP, palette.get(i)))
                    .collect::<Result<Vec<_>>>()?;
                column.sections[y].states = Paletted::from_values(&values, Strategy::Blocks, 16);
                column.sections[y].non_empty = non_empty;
                column.sections[y].fluids = (0..4096)
                    .filter(|i| properties::FLUID[palette.get(*i) as usize])
                    .count() as i16;
            }
            if let Some(biomes) = &biomes {
                let values = &biomes[y * 64..y * 64 + 64];
                if values
                    .iter()
                    .any(|id| *id < 0 || *id as usize >= self.biome_count)
                {
                    return Err(Error::new("old biome id outside declared registry"));
                }
                let bits = (usize::BITS - self.biome_count.saturating_sub(1).leading_zeros()) as u8;
                column.sections[y].biomes = Paletted::from_values(values, Strategy::Biomes, bits);
            }
        }
        // GlobalPalette.getSerializedSize counts one byte while its writer emits
        // none. The packet allocator leaves exactly one zero per global section.
        if sections.bytes(allocator_padding)?.iter().any(|b| *b != 0) {
            return Err(Error::new("nonzero 1.16.5 chunk allocator padding"));
        }
        finish(&sections)?;
        if full {
            column.entities = entities;
        } else {
            column
                .entities
                .retain(|e| mask & (1 << (e.y as usize / 16)) == 0);
            column.entities.extend(entities);
        }
        let heightmaps = [
            "WORLD_SURFACE_WG",
            "WORLD_SURFACE",
            "OCEAN_FLOOR_WG",
            "OCEAN_FLOOR",
            "MOTION_BLOCKING",
            "MOTION_BLOCKING_NO_LEAVES",
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(i, name)| match heights.get(name) {
            Some(Tag::LongArray(data)) => Some((i as i32, data.clone())),
            _ => None,
        })
        .collect();
        let packet = play::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps,
                buffer: ChunkData::from_sections(&column.sections),
                block_entities: column.entities.clone(),
            },
            light: self
                .lights
                .get(&(x, z))
                .cloned()
                .unwrap_or_else(empty_light),
        };
        self.columns.insert((x, z), column);
        Ok(play::Clientbound::LevelChunkWithLight(packet))
    }
}

fn fluid(state: i32) -> bool {
    static FLUID: OnceLock<std::collections::HashSet<i32>> = OnceLock::new();
    FLUID
        .get_or_init(|| {
            tables::STATE_UP
                .iter()
                .enumerate()
                .filter_map(|(i, id)| properties::FLUID[i].then_some(*id))
                .collect()
        })
        .contains(&state)
}
fn air(state: i32) -> bool {
    properties::AIR
        .iter()
        .any(|id| tables::STATE_UP[*id as usize] == state)
}
fn block(state: i32) -> Option<i32> {
    static BLOCK: OnceLock<HashMap<i32, i32>> = OnceLock::new();
    BLOCK
        .get_or_init(|| {
            tables::STATE_UP
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, properties::BLOCK[i]))
                .collect()
        })
        .get(&state)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    mod goldens {
        include!("chunk_goldens.rs");
    }
    #[test]
    fn actual_vanilla_local_and_global_chunk_writers_preserve_every_state() {
        for (name, count, runs) in goldens::WRITERS {
            let payload = runs
                .iter()
                .flat_map(|(value, count)| std::iter::repeat_n(*value, *count))
                .collect::<Vec<_>>();
            let mut world = World::default();
            world.set_biomes(vec![0]).unwrap();
            let play::Clientbound::LevelChunkWithLight(packet) = world.chunk(&payload).unwrap()
            else {
                panic!("{name}");
            };
            assert_eq!((packet.x, packet.z), (2, -3));
            let mut r = Reader::new(&packet.chunk.buffer);
            for y in 0..16 {
                let section = Section::read(&mut r, 16, 1).unwrap();
                assert_eq!(section.non_empty, if y == 0 { *count as i16 } else { 0 });
                let mut fluids = 0;
                for i in 0..4096 {
                    let raw = if y == 0 && i < *count { i + 1 } else { 0 };
                    assert_eq!(
                        section.states.get(i),
                        tables::STATE_UP[raw],
                        "{name}/{y}/{i}"
                    );
                    fluids += i16::from(properties::FLUID[raw]);
                }
                assert_eq!(section.fluids, fluids);
                assert!((0..64).all(|i| section.biomes.get(i) == 0));
            }
            finish(&r).unwrap();
            for len in [payload.len() - 1, payload.len() - 2] {
                assert!(world.chunk(&payload[..len]).is_err(), "{name}/{len}");
            }

            // Reframe the actual section bytes to test the allocator's exact suffix.
            let mut r = Reader::new(&payload);
            assert_eq!(r.i32().unwrap(), 2);
            assert_eq!(r.i32().unwrap(), -3);
            assert!(boolean(&mut r).unwrap());
            assert_eq!(r.varint().unwrap(), 1);
            r.named_nbt().unwrap();
            assert_eq!(r.count(1024).unwrap(), 1024);
            for _ in 0..1024 {
                assert_eq!(r.varint().unwrap(), 0);
            }
            let prefix = r.position();
            let buffer = r.byte_array(2 * 1024 * 1024).unwrap();
            assert_eq!(r.varint().unwrap(), 0);
            finish(&r).unwrap();
            for suffix in [vec![0], vec![1]] {
                let mut wrong = Writer::new();
                wrong.bytes(&payload[..prefix]);
                let mut sections = buffer.clone();
                sections.extend(suffix);
                wrong.byte_array(&sections);
                wrong.varint(0);
                assert!(world.chunk(&wrong.data).is_err(), "{name} extra suffix");
            }
            if name == &"global15" {
                let mut wrong = Writer::new();
                wrong.bytes(&payload[..prefix]);
                let mut sections = buffer;
                *sections.last_mut().unwrap() = 1;
                wrong.byte_array(&sections);
                wrong.varint(0);
                assert!(world.chunk(&wrong.data).is_err(), "nonzero global padding");
            }
        }
    }
    #[test]
    fn malformed_light_cannot_replace_cached_planes() {
        let mut w = World::default();
        let mut b = Writer::new();
        b.varint(2);
        b.varint(-3);
        b.bool(true);
        for mask in [2, 0, 0, 0] {
            b.varint(mask);
        }
        b.byte_array(&vec![0x31; 2048]);
        w.light(&b.data).unwrap();
        let before = w.lights.get(&(2, -3)).unwrap().clone();
        b.data.pop();
        assert!(w.light(&b.data).is_err());
        assert_eq!(w.lights.get(&(2, -3)), Some(&before));
    }
}

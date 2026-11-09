//! Connection-local terrain and light conversion. The mappings are supplied by the pinned game.
use super::{
    neighbors47,
    terrain::{self, Column, Update},
};
use crate::{
    Error, Result,
    buf::set_bit,
    chunk::{ChunkData, LightData, Paletted, Section, Strategy},
    packets::play::{
        Clientbound, ForgetLevelChunk, LevelChunkWithLight, LightUpdate, SectionBlocksUpdate,
    },
};
use std::collections::HashMap;

const MAX_COLUMNS: usize = 4096;

#[derive(Clone, Copy)]
pub struct Block {
    pub id: i32,
    pub air: bool,
    pub fluid: bool,
    /// The old block's vanilla light opacity, used by Chunk.canSeeSky's heightmap.
    pub sky_opacity: u8,
}

#[derive(Default)]
pub struct World {
    columns: HashMap<(i32, i32), Column>,
}

impl World {
    pub fn reset(&mut self) {
        self.columns.clear();
    }

    pub fn column(&self, x: i32, z: i32) -> Option<&Column> {
        self.columns.get(&(x, z))
    }

    pub fn raw_state(&self, pos: [i32; 3]) -> Option<u16> {
        raw_in(self.column(pos[0] >> 4, pos[2] >> 4)?, pos)
    }

    /// Only neighboring columns are copied; mapping can inspect the full incoming batch before commit.
    pub fn snapshot(&self, updates: &[Update]) -> Result<HashMap<(i32, i32), Column>> {
        let mut out = HashMap::new();
        for update in updates {
            let min_x = update
                .x
                .checked_sub(1)
                .ok_or_else(|| Error::new("legacy chunk coordinate overflow"))?;
            let max_x = update
                .x
                .checked_add(1)
                .ok_or_else(|| Error::new("legacy chunk coordinate overflow"))?;
            let min_z = update
                .z
                .checked_sub(1)
                .ok_or_else(|| Error::new("legacy chunk coordinate overflow"))?;
            let max_z = update
                .z
                .checked_add(1)
                .ok_or_else(|| Error::new("legacy chunk coordinate overflow"))?;
            for x in min_x..=max_x {
                for z in min_z..=max_z {
                    if let Some(column) = self.columns.get(&(x, z)) {
                        out.entry((x, z)).or_insert_with(|| column.clone());
                    }
                }
            }
        }
        for update in updates {
            let key = (update.x, update.z);
            if update.unload {
                out.remove(&key);
            } else if update.full {
                out.insert(key, Column::from_full(update.clone())?);
            } else {
                out.get_mut(&key)
                    .ok_or_else(|| Error::new("legacy partial chunk precedes full column"))?
                    .apply(update.clone())?;
            }
        }
        Ok(out)
    }

    /// Retain individual authoritative block changes for later partial-chunk/context mapping.
    pub fn set_raw(&mut self, pos: [i32; 3], raw: u16) -> Result<()> {
        if !(0..256).contains(&pos[1]) {
            return Err(Error::new("legacy block Y outside its world"));
        }
        let Some(column) = self.columns.get_mut(&(pos[0] >> 4, pos[2] >> 4)) else {
            return Ok(());
        };
        let y = (pos[1] >> 4) as usize;
        if column.sections[y].is_none() {
            // An allocated old section starts empty and is relit by the normal client block update.
            column.sections[y] = Some(terrain::Section {
                states: vec![0; 4096],
                block_light: vec![0; 2048],
                sky_light: column.has_sky.then(|| vec![0xff; 2048]),
            });
        }
        let section = column.sections[y].as_mut().unwrap();
        section.states[(((pos[1] & 15) << 8) | ((pos[2] & 15) << 4) | (pos[0] & 15)) as usize] =
            raw;
        Ok(())
    }

    /// Validate a whole incoming batch before committing any of its terrain state.
    pub fn apply(
        &mut self,
        updates: Vec<Update>,
        block: impl Fn(i32, i32, usize, usize, u16) -> Result<Block>,
        biome: impl Fn(u8) -> Result<i32>,
        block_bits: u8,
        biome_bits: u8,
    ) -> Result<Vec<Clientbound>> {
        let mut staged = HashMap::new();
        let mut packets = Vec::with_capacity(updates.len());
        for update in updates {
            let key = (update.x, update.z);
            if update.unload {
                staged.insert(key, None);
                packets.push(Clientbound::ForgetLevelChunk(ForgetLevelChunk {
                    pos: [key.0, key.1],
                }));
                continue;
            }
            let column = if update.full {
                let column = Column::from_full(update)?;
                let packet = convert(&column, &block, &biome, block_bits, biome_bits)?;
                packets.push(Clientbound::LevelChunkWithLight(packet));
                column
            } else {
                let previous = staged
                    .get(&key)
                    .and_then(Option::as_ref)
                    .or_else(|| {
                        (!staged.contains_key(&key))
                            .then(|| self.columns.get(&key))
                            .flatten()
                    })
                    .cloned()
                    .ok_or_else(|| Error::new("legacy partial chunk precedes its full column"))?;
                let provided =
                    std::array::from_fn(|y| update.sections.get(y).is_some_and(Option::is_some));
                let mut column = previous.clone();
                column.apply(update)?;
                packets.extend(partial(&previous, &column, &provided, &block)?);
                column
            };
            staged.insert(key, Some(column));
        }
        let retained = self
            .columns
            .keys()
            .filter(|key| !staged.contains_key(key))
            .count();
        if retained + staged.values().filter(|v| v.is_some()).count() > MAX_COLUMNS {
            return Err(Error::new("legacy terrain cache exceeds its column limit"));
        }
        for (key, column) in staged {
            if let Some(column) = column {
                self.columns.insert(key, column);
            } else {
                self.columns.remove(&key);
            }
        }
        Ok(packets)
    }
}

/// A partial old S21 keeps all omitted sections, block entities and locally
/// updated light. Replacing it with a full current column would erase that state.
fn partial(
    previous: &Column,
    column: &Column,
    provided: &[bool; terrain::SECTIONS],
    block: &impl Fn(i32, i32, usize, usize, u16) -> Result<Block>,
) -> Result<Vec<Clientbound>> {
    let mut packets = Vec::new();
    let mut light = LightData {
        sky_mask: Vec::new(),
        block_mask: Vec::new(),
        empty_sky_mask: Vec::new(),
        empty_block_mask: Vec::new(),
        sky: Vec::new(),
        block: Vec::new(),
    };
    for y in 0..terrain::SECTIONS {
        let old = previous.sections[y].as_ref();
        let new = column.sections[y].as_ref();
        if old.is_some_and(|s| s.states.len() != 4096)
            || new.is_some_and(|s| s.states.len() != 4096)
        {
            return Err(Error::new("invalid cached legacy block section shape"));
        }
        let below_changed = y > 0 && provided[y - 1];
        let above_changed = y + 1 < terrain::SECTIONS && provided[y + 1];
        let mut changes = Vec::new();
        if provided[y] || below_changed || above_changed {
            for i in 0..4096 {
                let raw = new.map_or(0, |s| s.states[i]);
                let contextual = neighbors47::needs_context(raw);
                // Raw-cache equality does not prove equality with the native
                // world's predictions. Reconcile every authoritative received
                // cell; the native World::set skips dirty work for equal states
                // and sync_block_entity retains same-kind block-entity NBT.
                let inside = provided[y];
                // A door/double plant/snowy grass just outside the received
                // section may depend on its newly received vertical neighbor.
                let border = contextual
                    && ((below_changed && i >> 8 == 0) || (above_changed && i >> 8 == 15));
                if !inside && !border {
                    continue;
                }
                let state = block(column.x, column.z, y, i, raw)?.id;
                if state < 0 {
                    return Err(Error::new("negative mapped legacy block state"));
                }
                let local = ((i & 15) << 8) | (i & 0xf0) | (i >> 8);
                changes.push((i64::from(state) << 12) | local as i64);
            }
        }
        if !changes.is_empty() {
            packets.push(Clientbound::SectionBlocksUpdate(SectionBlocksUpdate {
                section: [column.x, y as i32, column.z],
                changes,
            }));
        }
        if !provided[y] {
            continue;
        }
        let new = new.ok_or_else(|| Error::new("received legacy section is absent"))?;
        // Bit0 is the light-only section below y0. Omitted real planes stay put.
        layer(
            &new.block_light,
            y + 1,
            &mut light.block_mask,
            &mut light.empty_block_mask,
            &mut light.block,
        )?;
        match (&new.sky_light, column.has_sky) {
            (Some(sky), true) => layer(
                sky,
                y + 1,
                &mut light.sky_mask,
                &mut light.empty_sky_mask,
                &mut light.sky,
            )?,
            (None, false) => {}
            _ => {
                return Err(Error::new(
                    "legacy chunk sky layer disagrees with its dimension",
                ));
            }
        }
    }
    if provided[0] {
        // Old World.getLightFor clamps negative Y to zero. Refresh the derived
        // lower padding whenever the authoritative bottom section is received.
        // The regular layers above already validated both source plane lengths.
        let first = column.sections[0].as_ref().unwrap();
        let count = light.block.len();
        layer(
            &repeat_bottom(Some(&first.block_light), |_| 0),
            0,
            &mut light.block_mask,
            &mut light.empty_block_mask,
            &mut light.block,
        )?;
        if light.block.len() != count {
            light.block.rotate_right(1);
        }
        if column.has_sky {
            let count = light.sky.len();
            layer(
                &repeat_bottom(first.sky_light.as_deref(), |_| 0),
                0,
                &mut light.sky_mask,
                &mut light.empty_sky_mask,
                &mut light.sky,
            )?;
            if light.sky.len() != count {
                light.sky.rotate_right(1);
            }
        }
    }
    if provided.iter().any(|v| *v) {
        packets.push(Clientbound::LightUpdate(LightUpdate {
            x: column.x,
            z: column.z,
            light,
        }));
    }
    Ok(packets)
}

pub fn raw_in(column: &Column, pos: [i32; 3]) -> Option<u16> {
    if !(0..256).contains(&pos[1]) || (pos[0] >> 4, pos[2] >> 4) != (column.x, column.z) {
        return None;
    }
    Some(
        column.sections[(pos[1] >> 4) as usize]
            .as_ref()
            .map_or(0, |s| {
                s.states[(((pos[1] & 15) << 8) | ((pos[2] & 15) << 4) | (pos[0] & 15)) as usize]
            }),
    )
}

fn convert(
    column: &Column,
    block: &impl Fn(i32, i32, usize, usize, u16) -> Result<Block>,
    biome: &impl Fn(u8) -> Result<i32>,
    block_bits: u8,
    biome_bits: u8,
) -> Result<LevelChunkWithLight> {
    if column.sections.len() != terrain::SECTIONS || column.biomes.len() != 256 {
        return Err(Error::new("invalid cached legacy column shape"));
    }
    let mut biome_values = Vec::with_capacity(64);
    for _y in 0..4 {
        for z in 0..4 {
            for x in 0..4 {
                let id = biome(column.biomes[(z * 4 + 2) * 16 + x * 4 + 2])?;
                if id < 0 {
                    return Err(Error::new("negative mapped legacy biome"));
                }
                biome_values.push(id);
            }
        }
    }
    let biomes = Paletted::from_values(&biome_values, Strategy::Biomes, biome_bits);
    let mut sections = Vec::with_capacity(terrain::SECTIONS);
    let mut heights = [0usize; 256];
    let mut light = LightData {
        sky_mask: Vec::new(),
        block_mask: Vec::new(),
        empty_sky_mask: Vec::new(),
        empty_block_mask: Vec::new(),
        sky: Vec::new(),
        block: Vec::new(),
    };
    for (y, old) in column.sections.iter().enumerate() {
        let mut values = Vec::with_capacity(4096);
        let mut non_empty = 0;
        let mut fluids = 0;
        if old.as_ref().is_some_and(|s| s.states.len() != 4096) {
            return Err(Error::new("invalid cached legacy block section shape"));
        }
        for i in 0..4096 {
            let raw = old.as_ref().map_or(0, |s| s.states[i]);
            let mapped = block(column.x, column.z, y, i, raw)?;
            if mapped.id < 0 {
                return Err(Error::new("negative mapped legacy block state"));
            }
            values.push(mapped.id);
            non_empty += i16::from(!mapped.air);
            fluids += i16::from(mapped.fluid);
        }
        sections.push(Section {
            non_empty,
            fluids,
            states: Paletted::from_values(&values, Strategy::Blocks, block_bits),
            biomes: biomes.clone(),
        });
        // Old Chunk's height map is the first Y above a block with nonzero light opacity.
        // Nonempty is different: glass and plants must not cast an opaque sky column.
        for i in 0..4096 {
            let raw = old.as_ref().map_or(0, |s| s.states[i]);
            if block(column.x, column.z, y, i, raw)?.sky_opacity != 0 {
                heights[i & 255] = y * 16 + (i >> 8) + 1;
            }
        }
    }
    // Bit0 is the light-only section below y0. Actual section y maps to bit y+1.
    for (y, old) in column.sections.iter().enumerate() {
        let index = y + 1;
        if let Some(old) = old {
            layer(
                &old.block_light,
                index,
                &mut light.block_mask,
                &mut light.empty_block_mask,
                &mut light.block,
            )?;
            match (&old.sky_light, column.has_sky) {
                (Some(sky), true) => layer(
                    sky,
                    index,
                    &mut light.sky_mask,
                    &mut light.empty_sky_mask,
                    &mut light.sky,
                )?,
                (None, false) => set_bit(&mut light.empty_sky_mask, index),
                _ => {
                    return Err(Error::new(
                        "legacy chunk sky layer disagrees with its dimension",
                    ));
                }
            }
        } else {
            set_bit(&mut light.empty_block_mask, index);
            if column.has_sky {
                layer(
                    &missing_sky(y * 16, &heights),
                    index,
                    &mut light.sky_mask,
                    &mut light.empty_sky_mask,
                    &mut light.sky,
                )?;
            } else {
                set_bit(&mut light.empty_sky_mask, index);
            }
        }
    }
    // World.getLightFor clamps every negative Y to zero, including explicit transmitted layers.
    let first = column.sections[0].as_ref();
    let lower_block = repeat_bottom(first.map(|s| s.block_light.as_slice()), |_| 0);
    let count = light.block.len();
    layer(
        &lower_block,
        0,
        &mut light.block_mask,
        &mut light.empty_block_mask,
        &mut light.block,
    )?;
    if light.block.len() != count {
        light.block.rotate_right(1);
    }
    set_bit(&mut light.empty_block_mask, terrain::SECTIONS + 1);
    if column.has_sky {
        let lower_sky = repeat_bottom(first.and_then(|s| s.sky_light.as_deref()), |xz| {
            if heights[xz] == 0 { 15 } else { 0 }
        });
        let count = light.sky.len();
        layer(
            &lower_sky,
            0,
            &mut light.sky_mask,
            &mut light.empty_sky_mask,
            &mut light.sky,
        )?;
        if light.sky.len() != count {
            light.sky.rotate_right(1);
        }
        set_bit(&mut light.sky_mask, terrain::SECTIONS + 1);
        light.sky.push(vec![0xff; 2048]);
    } else {
        set_bit(&mut light.empty_sky_mask, 0);
        set_bit(&mut light.empty_sky_mask, terrain::SECTIONS + 1);
    }
    Ok(LevelChunkWithLight {
        x: column.x,
        z: column.z,
        chunk: ChunkData {
            // The native client rebuilds its local height information from these sections.
            heightmaps: Vec::new(),
            buffer: ChunkData::from_sections(&sections),
            block_entities: Vec::new(),
        },
        light,
    })
}

fn repeat_bottom(source: Option<&[u8]>, absent: impl Fn(usize) -> u8) -> Vec<u8> {
    (0..2048)
        .map(|i| {
            let low = (i * 2) & 255;
            let high = (i * 2 + 1) & 255;
            let sample = |xz| source.map_or_else(|| absent(xz), |s| crate::chunk::nibble(s, xz));
            sample(low) | (sample(high) << 4)
        })
        .collect()
}

fn missing_sky(base_y: usize, heights: &[usize; 256]) -> Vec<u8> {
    let mut layer = vec![0; 2048];
    for i in 0..4096 {
        if base_y + (i >> 8) >= heights[i & 255] {
            layer[i >> 1] |= 15 << ((i & 1) * 4);
        }
    }
    layer
}

fn layer(
    data: &[u8],
    index: usize,
    mask: &mut Vec<u8>,
    empty: &mut Vec<u8>,
    layers: &mut Vec<Vec<u8>>,
) -> Result<()> {
    if data.len() != 2048 {
        return Err(Error::new("legacy light layer is not 2048 bytes"));
    }
    if data.iter().all(|&v| v == 0) {
        set_bit(empty, index);
    } else {
        set_bit(mask, index);
        layers.push(data.to_vec());
    }
    Ok(())
}

#[cfg(test)]
#[path = "world_partial_tests.rs"]
mod partial_tests;

//! Protocol-47 terrain payloads, derived from the actual vanilla 1.8.9 go/gp writers.
//! This is an isolated integration candidate; the caller owns per-connection column lifetime.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
    chunk::{Paletted, Strategy},
};

pub const SECTIONS: usize = 16;
const BLOCKS: usize = 4096;
const LIGHT_BYTES: usize = 2048;
const BIOME_BYTES: usize = 256;
const MAX_BULK_COLUMNS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// (y * 16 + z) * 16 + x; each value is block_id << 4 | metadata.
    /// It is NOT Block.getStateId's particle encoding (block_id | metadata << 12).
    pub states: Vec<u16>,
    pub block_light: Vec<u8>,
    pub sky_light: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub x: i32,
    pub z: i32,
    pub full: bool,
    /// Only a single ground-up, zero-mask S21 means unload; bulk zero-mask means loaded air.
    pub unload: bool,
    pub has_sky: bool,
    pub mask: u16,
    /// Missing means unchanged for partial updates, known air for full updates.
    pub sections: Vec<Option<Section>>,
    pub biomes: Option<Vec<u8>>,
}

fn bool_byte(r: &mut Reader) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid legacy chunk boolean")),
    }
}

fn body_len(mask: u16, sky: bool, full: bool) -> usize {
    mask.count_ones() as usize * (8192 + LIGHT_BYTES + usize::from(sky) * LIGHT_BYTES)
        + usize::from(full) * BIOME_BYTES
}

/// Decode old S21, whose byte-array length includes states, lighting and optional 2D biomes.
pub fn single(payload: &[u8], has_sky: bool) -> Result<Update> {
    let mut r = Reader::new(payload);
    let (x, z) = (r.i32()?, r.i32()?);
    check_position(x, z)?;
    let full = bool_byte(&mut r)?;
    let mask = r.u16()?;
    let size = r.count(body_len(u16::MAX, has_sky, true))?;
    let body = r.bytes(size)?;
    if r.remaining() != 0 {
        return Err(Error::new("trailing legacy chunk bytes"));
    }
    if full && mask == 0 {
        // NetHandlerPlayClient handles this before reading sections. A server's unload uses an
        // empty array; a zero-mask full writer can carry 256 biome bytes and still means unload.
        if !body.is_empty() && body.len() != BIOME_BYTES {
            return Err(Error::new("invalid legacy unload body"));
        }
        return Ok(Update {
            x,
            z,
            full,
            unload: true,
            has_sky,
            mask,
            sections: vec![None; SECTIONS],
            biomes: None,
        });
    }
    decode_body(x, z, full, mask, has_sky, body)
}

/// S26 bulk: one sky-light boolean and count, all (x,z,mask) headers, then all raw bodies.
/// There are no per-column lengths and no inner compression stream in protocol 47.
pub fn bulk(payload: &[u8]) -> Result<Vec<Update>> {
    let mut r = Reader::new(payload);
    let sky = bool_byte(&mut r)?;
    let count = r.count(MAX_BULK_COLUMNS)?;
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        let x = r.i32()?;
        let z = r.i32()?;
        check_position(x, z)?;
        headers.push((x, z, r.u16()?));
    }
    let expected = headers.iter().try_fold(0usize, |sum, &(_, _, mask)| {
        sum.checked_add(body_len(mask, sky, true))
            .ok_or_else(|| Error::new("legacy bulk chunk length overflow"))
    })?;
    if expected != r.remaining() {
        return Err(Error::new(
            "legacy bulk bodies disagree with their section masks",
        ));
    }
    let mut columns = Vec::with_capacity(count);
    for (x, z, mask) in headers {
        let body = r.bytes(body_len(mask, sky, true))?;
        // Unlike S21, this does not turn zero-mask columns into unloads.
        columns.push(decode_body(x, z, true, mask, sky, body)?);
    }
    Ok(columns)
}

fn check_position(x: i32, z: i32) -> Result<()> {
    if !(-2_097_152..2_097_152).contains(&x) || !(-2_097_152..2_097_152).contains(&z) {
        return Err(Error::new(
            "legacy chunk is outside packed block position range",
        ));
    }
    Ok(())
}

fn decode_body(x: i32, z: i32, full: bool, mask: u16, sky: bool, body: &[u8]) -> Result<Update> {
    if body.len() != body_len(mask, sky, full) {
        return Err(Error::new(
            "legacy chunk body disagrees with its section mask",
        ));
    }
    let mut r = Reader::new(body);
    let mut sections: Vec<Option<Section>> = vec![None; SECTIONS];
    // Vanilla stores each plane for all sections before moving to the next plane.
    for (y, section) in sections.iter_mut().enumerate() {
        if mask & (1 << y) == 0 {
            continue;
        }
        let states = r
            .bytes(BLOCKS * 2)?
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        *section = Some(Section {
            states,
            block_light: Vec::new(),
            sky_light: None,
        });
    }
    for section in sections.iter_mut().flatten() {
        section.block_light = r.bytes(LIGHT_BYTES)?.to_vec();
    }
    if sky {
        for section in sections.iter_mut().flatten() {
            section.sky_light = Some(r.bytes(LIGHT_BYTES)?.to_vec());
        }
    }
    let biomes = if full {
        Some(r.bytes(BIOME_BYTES)?.to_vec())
    } else {
        None
    };
    debug_assert_eq!(r.remaining(), 0);
    Ok(Update {
        x,
        z,
        full,
        unload: false,
        has_sky: sky,
        mask,
        sections,
        biomes,
    })
}

/// A complete column. Keep these per connection and clear them on dimension change/disconnect.
/// A partial update cannot be translated into a modern full column without its previous content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub x: i32,
    pub z: i32,
    pub has_sky: bool,
    pub sections: Vec<Option<Section>>,
    /// Retain the whole per-block 2D grid even though a modern packet's quart biomes are coarser.
    pub biomes: Vec<u8>,
}

impl Column {
    pub fn from_full(update: Update) -> Result<Self> {
        if !update.full || update.unload || update.sections.len() != SECTIONS {
            return Err(Error::new(
                "a legacy column needs a complete initial update",
            ));
        }
        let biomes = update
            .biomes
            .filter(|biomes| biomes.len() == BIOME_BYTES)
            .ok_or_else(|| Error::new("a legacy full column needs 256 biomes"))?;
        Ok(Self {
            x: update.x,
            z: update.z,
            has_sky: update.has_sky,
            sections: update.sections,
            biomes,
        })
    }

    pub fn apply(&mut self, update: Update) -> Result<()> {
        if (update.x, update.z, update.has_sky) != (self.x, self.z, self.has_sky)
            || update.unload
            || update.sections.len() != SECTIONS
        {
            return Err(Error::new("legacy column update identity disagrees"));
        }
        if update.full {
            *self = Self::from_full(update)?;
        } else {
            for (existing, changed) in self.sections.iter_mut().zip(update.sections) {
                if changed.is_some() {
                    *existing = changed;
                }
            }
        }
        Ok(())
    }

    /// Produce the pre-1.21.5 section framing used by protocol 763/1.20.1: non-empty count,
    /// block palette with a prefixed long count, biome palette with a prefixed long count.
    /// The block callback gets (section_y, local_block_index, raw_state), allowing position-aware
    /// leaf/skull/neighbor reconstruction. Retained raw states must not silently become air.
    /// Supply pinned/current mapped block and biome ids, then feed through the existing section
    /// framing step exactly once. There is no second (fluids) short in this older framing.
    pub fn sections_1_20_1(
        &self,
        block: impl Fn(usize, usize, u16) -> Result<i32>,
        biome: impl Fn(u8) -> Result<i32>,
        block_bits: u8,
        biome_bits: u8,
    ) -> Result<Vec<u8>> {
        if self.sections.len() != SECTIONS || self.biomes.len() != BIOME_BYTES {
            return Err(Error::new("invalid cached legacy column shape"));
        }
        let mut mapped_biomes = Vec::with_capacity(64);
        // 1.8 has a biome per XZ block; modern quart biomes cannot encode all 256 values.
        // Sample each quart's center and repeat vertically, while retaining the full grid above.
        for _y in 0..4 {
            for z in 0..4 {
                for x in 0..4 {
                    mapped_biomes.push(biome(self.biomes[(z * 4 + 2) * 16 + x * 4 + 2])?);
                }
            }
        }
        let biomes = Paletted::from_values(&mapped_biomes, Strategy::Biomes, biome_bits);
        let mut w = Writer::new();
        for (section_y, section) in self.sections.iter().enumerate() {
            let (values, non_empty) = if let Some(section) = section {
                if section.states.len() != BLOCKS {
                    return Err(Error::new("invalid cached legacy block section shape"));
                }
                (
                    section
                        .states
                        .iter()
                        .enumerate()
                        .map(|(index, &id)| block(section_y, index, id))
                        .collect::<Result<Vec<_>>>()?,
                    section.states.iter().filter(|&&id| id >> 4 != 0).count(),
                )
            } else {
                (vec![block(section_y, 0, 0)?; BLOCKS], 0)
            };
            w.i16(non_empty as i16);
            write_prefixed(
                &Paletted::from_values(&values, Strategy::Blocks, block_bits),
                &mut w,
            );
            write_prefixed(&biomes, &mut w);
        }
        Ok(w.data)
    }
}

fn write_prefixed(palette: &Paletted, w: &mut Writer) {
    w.u8(palette.bits);
    if !palette.palette.is_empty() {
        if palette.bits != 0 {
            w.varint(palette.palette.len() as i32);
        }
        for &id in &palette.palette {
            w.varint(id);
        }
    }
    w.varint(palette.data.len() as i32);
    for &word in &palette.data {
        w.i64(word);
    }
}

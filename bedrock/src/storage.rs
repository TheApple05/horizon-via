// SPDX-License-Identifier: MIT
// Adapted storage format from Dragonfly, Copyright (c) 2019 Dragonfly Tech,
// and Gophertunnel's varint format, Copyright (c) 2019 Sandertv.
// Full MIT notices and exact reference pins are retained in notices/ and README.md.
// Changes: bounded Rust decoding, checked indices, explicit tails and shared biomes.

//! Bounded network paletted storage. No assets, NBT disk palettes or world mutation.
//! IDs remain unsigned opaque values until the session selects hashes or ordinals.

use std::{fmt, ops::Range, sync::Arc};

const CELLS: usize = 4096;
const MAX_LAYERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub offset: usize,
    pub reason: &'static str,
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.offset)
    }
}
impl std::error::Error for DecodeError {}

#[derive(Debug, Clone)]
pub struct Storage(Arc<[u32]>);

impl Storage {
    /// Bedrock's network cell order has Y fastest, then Z, then X.
    pub fn at(&self, x: u8, y: u8, z: u8) -> Option<u32> {
        if x >= 16 || y >= 16 || z >= 16 {
            return None;
        }
        Some(self.0[(usize::from(x) << 8) | (usize::from(z) << 4) | usize::from(y)])
    }
}

#[derive(Debug)]
pub struct Subchunk {
    pub section_y: i8,
    /// Kept separate: the application must merge water/extra layers semantically.
    pub layers: Vec<Storage>,
    /// Bytes consumed, leaving any following biomes/block-entity payload untouched.
    pub consumed: usize,
}

#[derive(Debug)]
pub struct Biomes {
    pub sections: Vec<Storage>,
    pub consumed: usize,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn fail<T>(&self, reason: &'static str) -> Result<T, DecodeError> {
        Err(DecodeError {
            offset: self.at,
            reason,
        })
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if len > self.bytes.len() - self.at {
            return self.fail("truncated storage");
        }
        let start = self.at;
        self.at += len;
        Ok(&self.bytes[start..self.at])
    }
    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn signed(&mut self) -> Result<i32, DecodeError> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            if shift == 28 && byte >= 16 {
                return self.fail("varint32 overflow");
            }
            value |= u32::from(byte & 127) << shift;
            if byte < 128 {
                return Ok(((value >> 1) as i32) ^ -((value & 1) as i32));
            }
        }
        self.fail("varint32 overflow")
    }

    fn storage(&mut self, biome: bool) -> Result<Option<Storage>, DecodeError> {
        let header = self.byte()?;
        if header & 1 == 0 {
            return self.fail("disk palette is not a network palette");
        }
        let bits = header >> 1;
        if bits == 127 && biome {
            return Ok(None);
        }
        if !matches!(bits, 0 | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 16) {
            return self.fail("unsupported network palette width");
        }
        let per_word = if bits == 0 { 0 } else { 32 / usize::from(bits) };
        let words = if bits == 0 {
            &[][..]
        } else {
            self.take(CELLS.div_ceil(per_word) * 4)?
        };
        let count = if bits == 0 { 1 } else { self.signed()? };
        // A bounded column decoder deliberately rejects unused oversized palettes.
        if count <= 0 || count as usize > CELLS || count as usize > (1usize << bits) {
            return self.fail("invalid or excessive palette count");
        }
        let mut palette = Vec::with_capacity(count as usize);
        for _ in 0..count {
            palette.push(self.signed()? as u32);
        }
        if bits == 0 {
            return Ok(Some(Storage(vec![palette[0]; CELLS].into())));
        }
        let mask = (1u32 << bits) - 1;
        let mut cells = Vec::with_capacity(CELLS);
        for index in 0..CELLS {
            let offset = (index / per_word) * 4;
            let word =
                u32::from_le_bytes(words[offset..offset + 4].try_into().expect("bounded word"));
            let slot = ((word >> ((index % per_word) * usize::from(bits))) & mask) as usize;
            let Some(&id) = palette.get(slot) else {
                return self.fail("palette index out of range");
            };
            cells.push(id);
        }
        Ok(Some(Storage(cells.into())))
    }
}

/// Decodes one current version-9 subchunk within explicit negotiated section bounds.
/// At most eight layers and 64 sections are admitted by this backend policy.
/// Older storage versions and disk palettes are explicitly unsupported.
pub fn decode_subchunk(bytes: &[u8], section_bounds: Range<i16>) -> Result<Subchunk, DecodeError> {
    let mut r = Reader { bytes, at: 0 };
    if section_bounds.start < -128
        || section_bounds.end > 128
        || section_bounds.is_empty()
        || section_bounds.end - section_bounds.start > 64
    {
        return r.fail("invalid section bounds");
    }
    if r.byte()? != 9 {
        return r.fail("unsupported subchunk version");
    }
    let count = usize::from(r.byte()?);
    if count > MAX_LAYERS {
        return r.fail("excessive storage layers");
    }
    let section_y = r.byte()? as i8;
    if !section_bounds.contains(&i16::from(section_y)) {
        return r.fail("section outside dimension");
    }
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        layers.push(r.storage(false)?.expect("block storage cannot repeat"));
    }
    Ok(Subchunk {
        section_y,
        layers,
        consumed: r.at,
    })
}

/// Decodes full-height biome storages. This count comes from dimension height,
/// independently of the number of block subchunks carried by a LevelChunk.
pub fn decode_biomes(bytes: &[u8], section_count: usize) -> Result<Biomes, DecodeError> {
    let mut r = Reader { bytes, at: 0 };
    if !(1..=64).contains(&section_count) {
        return r.fail("invalid biome section count");
    }
    let mut sections: Vec<Storage> = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        match r.storage(true)? {
            Some(storage) => sections.push(storage),
            None => {
                let Some(last) = sections.last() else {
                    return r.fail("first biome storage cannot repeat");
                };
                sections.push(last.clone());
            }
        }
    }
    Ok(Biomes {
        sections,
        consumed: r.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_signed_hash_negative_y_and_tail_are_preserved() {
        // version9, one layer, section-4, network singleton, ZigZag MIN_I32.
        let bytes = [9, 1, 252, 1, 255, 255, 255, 255, 15, 0xaa];
        let decoded = decode_subchunk(&bytes, -4..20).unwrap();
        assert_eq!(decoded.section_y, -4);
        assert_eq!(decoded.layers[0].at(15, 15, 15), Some(0x80000000));
        assert_eq!(decoded.consumed, 9);
        for end in 0..9 {
            assert!(decode_subchunk(&bytes[..end], -4..20).is_err());
        }
        assert!(decode_subchunk(&bytes, 0..8).is_err());
    }

    #[test]
    fn padded_three_bit_words_do_not_straddle_and_use_xzy_order() {
        let mut bytes = vec![9, 1, 0, 7];
        // 410 LE words. Slots9,10 and291 contain palette entries1,2 and1.
        let mut words = vec![0u8; 1640];
        words[3] = 8;
        words[4] = 2;
        words[116] = 8;
        bytes.extend(words);
        bytes.extend([6, 0, 2, 4]); // three entries: IDs0,1,2.
        let decoded = decode_subchunk(&bytes, 0..16).unwrap();
        let layer = &decoded.layers[0];
        assert_eq!(layer.at(0, 9, 0), Some(1));
        assert_eq!(layer.at(0, 10, 0), Some(2));
        assert_eq!(layer.at(1, 3, 2), Some(1));
        assert_eq!(layer.at(1, 2, 3), Some(0));
        assert_eq!(layer.at(16, 0, 0), None);
        assert_eq!(decoded.consumed, bytes.len());
        bytes[4] = 7; // index7 exceeds this three-entry palette.
        assert!(decode_subchunk(&bytes, 0..16).is_err());
    }

    #[test]
    fn biome_repeat_retains_prior_cells_and_leaves_border_tail() {
        let bytes = [1, 2, 255, 255, 0]; // singleton biome1, repeat, repeat, border.
        let decoded = decode_biomes(&bytes, 3).unwrap();
        assert_eq!(decoded.consumed, 4);
        assert!(decoded.sections.iter().all(|s| s.at(5, 6, 7) == Some(1)));
        assert!(Arc::ptr_eq(&decoded.sections[0].0, &decoded.sections[2].0));
        assert!(decode_biomes(&[255], 1).is_err());
        assert!(decode_biomes(&bytes, 65).is_err());
        assert!(decode_subchunk(&[9, 1, 0, 255], 0..16).is_err());
    }

    #[test]
    fn malformed_headers_counts_and_varints_are_rejected() {
        for bytes in [
            &[8, 0, 0][..],
            &[9, 9, 0],
            &[9, 1, 0, 0],
            &[9, 1, 0, 15],
            &[9, 1, 0, 1, 255, 255, 255, 255, 16],
            &[9, 1, 0, 1, 128],
        ] {
            assert!(decode_subchunk(bytes, 0..16).is_err(), "{bytes:?}");
        }
        let mut bytes = vec![9, 1, 0, 3];
        bytes.extend(vec![0; 512]); // one bit: 128 LE words.
        bytes.push(0); // no palette entries.
        assert!(decode_subchunk(&bytes, 0..16).is_err());
        *bytes.last_mut().unwrap() = 6; // three entries exceed one-bit capacity.
        assert!(decode_subchunk(&bytes, 0..16).is_err());
        assert!(decode_subchunk(&[9, 0, 0], -128..128).is_err());
    }
}

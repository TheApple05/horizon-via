//! Palette sections contain a continuous bit stream in 1.12; values may cross long boundaries.
use super::{boolean, finish};
use crate::adapter::v1_8_9::terrain::{Section, Update};
use crate::{Error, Result, buf::Reader};
use horizon_world::nbt::Tag;

pub struct Chunk {
    pub update: Update,
    pub block_entities: Vec<Tag>,
}

pub fn read(payload: &[u8], has_sky: bool) -> Result<Chunk> {
    let mut r = Reader::new(payload);
    let x = r.i32()?;
    let z = r.i32()?;
    if !(-2_097_152..2_097_152).contains(&x) || !(-2_097_152..2_097_152).contains(&z) {
        return Err(Error::new(
            "protocol340 chunk coordinate outside packed position range",
        ));
    }
    let full = boolean(&mut r)?;
    let mask = u16::try_from(r.varint()?)
        .map_err(|_| Error::new("invalid protocol340 chunk section mask"))?;
    let size = r.count(2 * 1024 * 1024)?;
    let mut body = Reader::new(r.bytes(size)?);
    let mut sections = vec![None; 16];
    for (y, section) in sections.iter_mut().enumerate() {
        if mask & (1 << y) == 0 {
            continue;
        }
        let bits = body.u8()?;
        if !(4..=13).contains(&bits) {
            return Err(Error::new("invalid protocol340 block palette bits"));
        }
        let palette_len = body.count(256)?;
        if bits > 8 && palette_len != 0
            || bits <= 8 && (palette_len == 0 || palette_len > 1 << bits)
        {
            return Err(Error::new("invalid protocol340 block palette length"));
        }
        let mut palette = Vec::with_capacity(palette_len);
        for _ in 0..palette_len {
            palette.push(
                u16::try_from(body.varint()?)
                    .map_err(|_| Error::new("invalid protocol340 palette state"))?,
            );
        }
        let longs = body.count(832)?;
        if longs != (4096 * usize::from(bits)).div_ceil(64) {
            return Err(Error::new(
                "protocol340 palette word count disagrees with bits",
            ));
        }
        let mut data = Vec::with_capacity(longs);
        for _ in 0..longs {
            data.push(body.i64()? as u64);
        }
        let states = unpack(&data, bits, &palette)?;
        let block_light = body.bytes(2048)?.to_vec();
        let sky_light = if has_sky {
            Some(body.bytes(2048)?.to_vec())
        } else {
            None
        };
        *section = Some(Section {
            states,
            block_light,
            sky_light,
        });
    }
    let biomes = if full {
        Some(body.bytes(256)?.to_vec())
    } else {
        None
    };
    finish(&body)?;
    let count = r.count(65536)?;
    let mut block_entities = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let tag = r.named_nbt()?;
        if !matches!(tag, Tag::Compound(_)) {
            return Err(Error::new("protocol340 chunk block entity is not compound"));
        }
        block_entities.push(tag);
    }
    finish(&r)?;
    Ok(Chunk {
        update: Update {
            x,
            z,
            full,
            unload: false,
            has_sky,
            mask,
            sections,
            biomes,
        },
        block_entities,
    })
}

fn unpack(data: &[u64], bits: u8, palette: &[u16]) -> Result<Vec<u16>> {
    let bits = usize::from(bits);
    if data.len() != (4096 * bits).div_ceil(64) {
        return Err(Error::new("invalid protocol340 packed data length"));
    }
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(4096);
    for i in 0..4096 {
        let bit = i * bits;
        let offset = bit & 63;
        let mut value = data[bit / 64] >> offset;
        if offset + bits > 64 {
            value |= data[bit / 64 + 1] << (64 - offset);
        }
        let value = (value & mask) as usize;
        let raw = if palette.is_empty() {
            value as u16
        } else {
            *palette
                .get(value)
                .ok_or_else(|| Error::new("protocol340 palette index out of range"))?
        };
        out.push(raw);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_palette_crosses_word_boundary() {
        let bits = 5usize;
        let values: Vec<u16> = (0..4096).map(|i| (i % 31) as u16).collect();
        let mut data = vec![0u64; (4096 * bits).div_ceil(64)];
        for (i, &v) in values.iter().enumerate() {
            let bit = i * bits;
            let offset = bit & 63;
            data[bit / 64] |= u64::from(v) << offset;
            if offset + bits > 64 {
                data[bit / 64 + 1] |= u64::from(v) >> (64 - offset);
            }
        }
        let palette: Vec<u16> = (0..31).map(|i| 100 + i).collect();
        assert_eq!(
            unpack(&data, 5, &palette).unwrap(),
            values.into_iter().map(|i| i + 100).collect::<Vec<_>>()
        );
        assert!(unpack(&data, 5, &[100]).is_err());
        assert!(unpack(&data[..data.len() - 1], 5, &palette).is_err());
    }
}

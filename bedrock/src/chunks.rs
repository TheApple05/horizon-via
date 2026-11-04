// SPDX-License-Identifier: MIT
// Format reference: Dragonfly, Copyright (c) 2019 Dragonfly Tech, and
// Gophertunnel, Copyright (c) 2019 Sandertv. Full MIT notices are retained.

//! Bounded cache-disabled LevelChunk envelopes. Sparse omissions remain unknown;
//! border/block-entity bytes are preserved, not validated or applied to a world.
use crate::storage::{self, Biomes, Subchunk};
use bedrock_protocol::packets::LevelChunk;
use std::{fmt, ops::Range};

const MAX_PAYLOAD: usize = 8 * 1024 * 1024;

/// The active dimension's independently negotiated bounds, never inferred from
/// a packet's subchunk count or a familiar numeric dimension ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimension {
    id: i32,
    start: i16,
    end: i16,
}
impl Dimension {
    pub fn new(id: i32, sections: Range<i16>) -> Result<Self, ChunkError> {
        if sections.start < -128
            || sections.end > 128
            || sections.is_empty()
            || i32::from(sections.end) - i32::from(sections.start) > 64
        {
            return Err(ChunkError::InvalidBounds);
        }
        Ok(Self {
            id,
            start: sections.start,
            end: sections.end,
        })
    }
    pub fn id(self) -> i32 {
        self.id
    }
    pub fn sections(self) -> Range<i16> {
        self.start..self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    InvalidBounds,
    WrongDimension,
    UnsupportedCache,
    UnsupportedRequests,
    ExcessivePayload,
    ExcessiveSubchunks,
    DuplicateSection(i8),
    Storage(storage::DecodeError),
}
impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => f.write_str("invalid native dimension section bounds"),
            Self::WrongDimension => f.write_str("native chunk belongs to another dimension"),
            Self::UnsupportedCache => f.write_str("native chunk blob caching is unsupported"),
            Self::UnsupportedRequests => f.write_str("native subchunk request mode is unsupported"),
            Self::ExcessivePayload => f.write_str("native chunk payload exceeds receive budget"),
            Self::ExcessiveSubchunks => f.write_str("native chunk count exceeds dimension bounds"),
            Self::DuplicateSection(y) => write!(f, "duplicate native section {y}"),
            Self::Storage(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ChunkError {}

#[derive(Debug)]
pub struct DecodedChunk<'a> {
    dimension: Dimension,
    position: (i32, i32),
    sections: Vec<Option<Subchunk>>,
    biomes: Biomes,
    tail: &'a [u8],
    client_biome_update: bool,
}
impl DecodedChunk<'_> {
    pub fn dimension(&self) -> Dimension {
        self.dimension
    }
    pub fn position(&self) -> (i32, i32) {
        self.position
    }
    /// In negotiated Y order. None means untransmitted, without an air or
    /// completeness assertion. Version9's explicit Y determines each slot.
    pub fn sections(&self) -> &[Option<Subchunk>] {
        &self.sections
    }
    pub fn biomes(&self) -> &Biomes {
        &self.biomes
    }
    /// Unvalidated border/block-entity bytes. A caller must handle them before
    /// claiming complete world state, and bind session/generation on apply.
    pub fn tail(&self) -> &[u8] {
        self.tail
    }
    pub fn is_client_biome_update(&self) -> bool {
        self.client_biome_update
    }
}

pub fn decode_level_chunk(
    packet: &LevelChunk,
    dimension: Dimension,
) -> Result<DecodedChunk<'_>, ChunkError> {
    if packet.dimension_id.0 != dimension.id {
        return Err(ChunkError::WrongDimension);
    }
    if packet.cache_enabled || !packet.cache_metadata.is_empty() {
        return Err(ChunkError::UnsupportedCache);
    }
    if packet.client_request_sub_chunk_limit.is_some() {
        return Err(ChunkError::UnsupportedRequests);
    }
    let raw: &[u8] = packet.serialized_chunk_data.as_ref();
    if raw.len() > MAX_PAYLOAD {
        return Err(ChunkError::ExcessivePayload);
    }
    let count = (dimension.end - dimension.start) as usize;
    if packet.sub_chunks_count.0 > count as u32 {
        return Err(ChunkError::ExcessiveSubchunks);
    }
    let mut sections: Vec<Option<Subchunk>> = (0..count).map(|_| None).collect();
    let mut cursor = 0;
    let absolute = |mut error: storage::DecodeError, offset| {
        error.offset += offset;
        ChunkError::Storage(error)
    };
    for _ in 0..packet.sub_chunks_count.0 {
        let subchunk = storage::decode_subchunk(&raw[cursor..], dimension.sections())
            .map_err(|error| absolute(error, cursor))?;
        let index = (i16::from(subchunk.section_y) - dimension.start) as usize;
        if sections[index].is_some() {
            return Err(ChunkError::DuplicateSection(subchunk.section_y));
        }
        cursor += subchunk.consumed;
        sections[index] = Some(subchunk);
    }
    // Full dimension height, independently of the transmitted block count.
    let biomes =
        storage::decode_biomes(&raw[cursor..], count).map_err(|error| absolute(error, cursor))?;
    cursor += biomes.consumed;
    Ok(DecodedChunk {
        dimension,
        position: (packet.chunk_position.x.0, packet.chunk_position.z.0),
        sections,
        biomes,
        tail: &raw[cursor..],
        client_biome_update: packet.is_client_biome_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bedrock_protocol::{types, wire};
    fn packet(count: u32, raw: Vec<u8>) -> LevelChunk {
        LevelChunk {
            chunk_position: types::ChunkPos {
                x: wire::ZigZag32(-7),
                z: wire::ZigZag32(11),
            },
            dimension_id: types::DimensionType(2000),
            sub_chunks_count: wire::VarUInt(count),
            serialized_chunk_data: raw.into(),
            ..Default::default()
        }
    }
    #[test]
    fn sparse_absolute_sections_biome_height_and_opaque_tail_are_preserved() {
        let mut packet = packet(
            2,
            vec![
                9, 0, 0, 9, 1, 252, 1, 2, 1, 4, 255, 255, 255, 255, 0xaa, 0xbb,
            ],
        );
        packet.is_client_biome_update = true;
        let dimension = Dimension::new(2000, -4..1).unwrap();
        let decoded = decode_level_chunk(&packet, dimension).unwrap();
        assert_eq!(decoded.dimension(), dimension);
        assert_eq!(decoded.position(), (-7, 11));
        assert!(decoded.is_client_biome_update());
        assert_eq!(decoded.sections().len(), 5);
        let low = decoded.sections()[0].as_ref().unwrap();
        assert_eq!(low.section_y, -4);
        assert_eq!(low.layers[0].at(2, 7, 11), Some(1));
        assert!(decoded.sections()[1..4].iter().all(Option::is_none));
        assert!(decoded.sections()[4].as_ref().unwrap().layers.is_empty());
        assert_eq!(decoded.biomes().sections.len(), 5);
        assert!(
            decoded
                .biomes()
                .sections
                .iter()
                .all(|s| s.at(15, 15, 15) == Some(2))
        );
        assert_eq!(decoded.tail(), &[0xaa, 0xbb]);
    }
    #[test]
    fn zero_block_sections_do_not_erase_dimension_biomes_or_assert_air() {
        let packet = packet(0, vec![1, 6, 255, 0]);
        let decoded = decode_level_chunk(&packet, Dimension::new(2000, -1..1).unwrap()).unwrap();
        assert!(decoded.sections().iter().all(Option::is_none));
        assert_eq!(decoded.biomes().sections.len(), 2);
        assert_eq!(decoded.biomes().sections[1].at(0, 0, 0), Some(3));
        assert_eq!(decoded.tail(), &[0]);
        assert!(!decoded.is_client_biome_update());
    }
    #[test]
    fn metadata_requests_cache_and_ambiguous_sections_fail_before_admission() {
        let dimension = Dimension::new(2000, 0..2).unwrap();
        let mut p = packet(2, vec![9, 0, 0, 9, 0, 0]);
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::DuplicateSection(0))
        ));
        p.sub_chunks_count.0 = 3;
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::ExcessiveSubchunks)
        ));
        p.sub_chunks_count.0 = 0;
        p.cache_enabled = true;
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::UnsupportedCache)
        ));
        p.cache_enabled = false;
        p.cache_metadata.push(types::SubChunkMetadata(1));
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::UnsupportedCache)
        ));
        p.cache_metadata.clear();
        p.client_request_sub_chunk_limit = Some(wire::ZigZag32(-1));
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::UnsupportedRequests)
        ));
        p.client_request_sub_chunk_limit = None;
        p.dimension_id.0 = 0;
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::WrongDimension)
        ));
    }
    #[test]
    fn truncation_bounds_and_payload_budgets_cannot_produce_a_column() {
        for (start, end) in [(-129, -128), (-128, -63), (0, 129), (1, 1), (2, 1)] {
            assert!(Dimension::new(0, start..end).is_err());
        }
        let dimension = Dimension::new(2000, -4..-3).unwrap();
        let bytes = [9, 1, 252, 1, 2, 1, 6];
        for end in 0..bytes.len() {
            assert!(decode_level_chunk(&packet(1, bytes[..end].to_vec()), dimension).is_err());
        }
        assert!(decode_level_chunk(&packet(1, bytes.to_vec()), dimension).is_ok());
        let p = packet(1, vec![9, 0, 253, 1, 6]);
        assert!(matches!(
            decode_level_chunk(&p, dimension),
            Err(ChunkError::Storage(_))
        ));
        assert!(matches!(
            decode_level_chunk(&packet(0, vec![0; MAX_PAYLOAD + 1]), dimension),
            Err(ChunkError::ExcessivePayload)
        ));
    }
    #[test]
    fn maximum_section_counts_cover_both_signed_y_extremes() {
        for start in [-128i16, 64] {
            let dimension = Dimension::new(2000, start..start + 64).unwrap();
            let mut raw = Vec::new();
            for y in dimension.sections().rev() {
                raw.extend([9, 0, (y as i8) as u8]);
            }
            raw.extend([1, 0]);
            raw.extend([255; 63]);
            let p = packet(64, raw);
            let decoded = decode_level_chunk(&p, dimension).unwrap();
            assert_eq!(decoded.sections().len(), 64);
            for (index, section) in decoded.sections().iter().enumerate() {
                assert_eq!(
                    i16::from(section.as_ref().unwrap().section_y),
                    start + index as i16
                );
            }
            assert_eq!(decoded.biomes().sections.len(), 64);
            assert!(decoded.tail().is_empty());
            let excessive = packet(u32::MAX, Vec::new());
            assert!(matches!(
                decode_level_chunk(&excessive, dimension),
                Err(ChunkError::ExcessiveSubchunks)
            ));
        }
    }
}

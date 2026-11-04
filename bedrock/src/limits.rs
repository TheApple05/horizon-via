// SPDX-License-Identifier: MIT

//! Application-owned bounds also cover expanded typed collections, not just batches.
use bedrock_protocol::wire::{Limits, Peer};

/// Initial backend policy. Servers exceeding these bounds remain unsupported
/// until a reviewed per-field budget and exact fixture justify an exception.
pub fn server_receive_limits() -> Limits {
    Limits::receiving_from(Peer::Server)
        .with_elements(4096)
        .with_bytes(8 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bedrock_protocol::{
        fields,
        types::BiomeStringList,
        wire::{Decode, DecodeError, Reader},
    };

    #[test]
    fn generated_codec_rejects_expanded_collection_before_allocation() {
        let limits = server_receive_limits();
        // A complete, representable4097-element list of empty strings fits in4KiB.
        // Its declared count must fail the application cap, not merely hit EOF.
        let mut excessive = vec![0x81, 0x20];
        excessive.extend(vec![0; 4097]);
        let mut reader = Reader::new(&excessive).with_limits(&limits);
        assert_eq!(
            BiomeStringList::decode(&mut reader).unwrap_err(),
            DecodeError::LengthLimitExceeded {
                field: fields::BIOME_STRING_LIST_STRINGS,
                limit: 4096,
                actual: 4097,
            }
        );
        let mut reader = Reader::new(&[2, 1, b'a', 0]).with_limits(&limits);
        let valid = BiomeStringList::decode(&mut reader).unwrap();
        assert_eq!(valid.strings.len(), 2);
        assert_eq!(valid.strings[0].as_bytes(), b"a");
        assert_eq!(valid.strings[1].as_bytes(), b"");
    }
}

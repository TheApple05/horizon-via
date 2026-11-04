// SPDX-License-Identifier: MIT

//! Developer-only comparison against separately generated synthetic Go vectors.
use horizon_bedrock::storage::decode_subchunk;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vector {
    bits: u8,
    bytes: Vec<u8>,
    expected: Vec<u32>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("supply synthetic vectors JSON")?;
    let bytes = std::fs::read(path)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("oversized synthetic vectors".into());
    }
    let vectors: Vec<Vector> = serde_json::from_slice(&bytes)?;
    let widths = vectors.iter().map(|v| v.bits).collect::<Vec<_>>();
    assert_eq!(widths, [0, 1, 2, 3, 4, 5, 6, 8, 16]);
    for v in vectors {
        assert_eq!(v.expected.len(), 4096);
        let decoded = decode_subchunk(&v.bytes, -4..20)?;
        assert_eq!(decoded.section_y, -4);
        assert_eq!(decoded.layers.len(), 1);
        assert_eq!(decoded.consumed, v.bytes.len());
        for x in 0..16u8 {
            for z in 0..16u8 {
                for y in 0..16u8 {
                    let expected =
                        v.expected[(usize::from(x) << 8) | (usize::from(z) << 4) | usize::from(y)];
                    assert_eq!(
                        decoded.layers[0].at(x, y, z),
                        Some(expected),
                        "width{} at({x},{y},{z})",
                        v.bits
                    );
                }
            }
        }
    }
    println!("All9reference widths and36,864synthetic cells matched.");
    Ok(())
}

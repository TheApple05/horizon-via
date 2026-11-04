// SPDX-License-Identifier: MIT

//! Developer-only smoke check against explicitly supplied extracted CC0 facts.
use horizon_bedrock::catalog::{Catalog, RuntimeIds};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("supply an extracted native-facts.json path")?;
    let bytes = std::fs::read(path)?;
    let catalog = Catalog::load(&bytes)?;
    for (ordinal, hash, name) in [
        (18894, 3690217760, "minecraft:air"),
        (3317, 2150698529, "minecraft:stone"),
        (14825, 2186211206, "minecraft:dirt"),
        (16313, 3727763636, "minecraft:grass_block"),
    ] {
        assert_eq!(
            catalog.block(RuntimeIds::Ordinal, ordinal).unwrap().name,
            name
        );
        assert_eq!(catalog.block(RuntimeIds::Hashed, hash).unwrap().name, name);
    }
    let mut tampered = bytes;
    tampered.push(b' ');
    assert!(Catalog::load(&tampered).is_err());
    println!(
        "Pinned native catalog loaded; independent ordinal/hash identities and tamper rejection passed."
    );
    Ok(())
}

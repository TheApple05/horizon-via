use super::*;
use crate::buf::Reader;

fn section(sky: bool) -> terrain::Section {
    terrain::Section {
        states: vec![0; 4096],
        block_light: vec![0; 2048],
        sky_light: sky.then(|| vec![0xff; 2048]),
    }
}
fn update(full: bool, sky: bool) -> Update {
    Update {
        x: 2,
        z: -3,
        full,
        unload: false,
        has_sky: sky,
        mask: 0,
        sections: vec![None; 16],
        biomes: full.then(|| vec![1; 256]),
    }
}
fn put(update: &mut Update, y: usize, section: terrain::Section) {
    update.mask |= 1 << y;
    update.sections[y] = Some(section);
}
fn index(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}
fn mapped(_: i32, _: i32, _: usize, _: usize, raw: u16) -> Result<Block> {
    Ok(Block {
        id: i32::from(raw),
        air: raw == 0,
        fluid: false,
        sky_opacity: u8::from(raw != 0),
    })
}
fn apply(world: &mut World, updates: Vec<Update>) -> Result<Vec<Clientbound>> {
    world.apply(updates, mapped, |_| Ok(1), 16, 6)
}
fn named(mask: &[u8], bit: usize) -> bool {
    mask.get(bit >> 3)
        .is_some_and(|v| v & (1 << (bit & 7)) != 0)
}
fn light(packets: &[Clientbound]) -> &LightData {
    &packets
        .iter()
        .find_map(|p| match p {
            Clientbound::LightUpdate(p) => Some(p),
            _ => None,
        })
        .unwrap()
        .light
}
fn changes(packets: &[Clientbound], section: i32) -> Vec<(usize, i32)> {
    packets
        .iter()
        .filter_map(|p| match p {
            Clientbound::SectionBlocksUpdate(p) if p.section[1] == section => Some(&p.changes),
            _ => None,
        })
        .flatten()
        .map(|&v| ((v & 4095) as usize, (v >> 12) as i32))
        .collect()
}
fn decode(packets: &[Clientbound]) {
    for packet in packets {
        let bytes = packet.encode();
        let mut r = Reader::new(&bytes);
        let id = r.varint().unwrap();
        Clientbound::decode(id, r.rest()).unwrap();
    }
}

#[test]
fn partial_reconciles_received_blocks_and_explicit_light_without_replacing_tile_entities() {
    let mut world = World::default();
    let mut full = update(true, true);
    let mut lower = section(true);
    lower.states[index(1, 2, 3)] = 63 << 4; // Unchanged sign with native NBT.
    lower.block_light.fill(0xff);
    put(&mut full, 0, lower);
    put(&mut full, 1, section(true));
    assert!(matches!(
        apply(&mut world, vec![full]).unwrap().as_slice(),
        [Clientbound::LevelChunkWithLight(_)]
    ));
    let mut partial = update(false, true);
    let mut upper = section(true);
    upper.states[index(4, 5, 6)] = 1 << 4;
    upper.block_light.fill(0x21);
    upper.sky_light.as_mut().unwrap().fill(0x43);
    put(&mut partial, 1, upper);
    let packets = apply(&mut world, vec![partial]).unwrap();
    decode(&packets);
    assert!(packets.iter().all(|p| matches!(
        p,
        Clientbound::SectionBlocksUpdate(_) | Clientbound::LightUpdate(_)
    )));
    assert_eq!(changes(&packets, 1).len(), 4096);
    assert!(changes(&packets, 1).contains(&(0x465, 16)));
    assert!(changes(&packets, 0).is_empty());
    let light = light(&packets);
    for bit in 0..18 {
        assert_eq!(named(&light.block_mask, bit), bit == 2);
        assert_eq!(named(&light.sky_mask, bit), bit == 2);
    }
    assert!(light.empty_block_mask.is_empty() && light.empty_sky_mask.is_empty());
    assert_eq!(light.block, vec![vec![0x21; 2048]]);
    assert_eq!(light.sky, vec![vec![0x43; 2048]]);
    assert_eq!(world.raw_state([33, 2, -45]), Some(63 << 4));
}

#[test]
fn partial_light_only_update_does_not_replay_stale_light_from_locally_updated_sections() {
    let mut world = World::default();
    let mut full = update(true, true);
    let mut lower = section(true);
    lower.states[0] = 50 << 4;
    lower.block_light.fill(0xff);
    put(&mut full, 0, lower);
    put(&mut full, 1, section(true));
    apply(&mut world, vec![full]).unwrap();
    world.set_raw([32, 0, -48], 0).unwrap(); // A prior block packet/explosion already removed the native light source.
    let mut partial = update(false, true);
    put(&mut partial, 1, section(true));
    let packets = apply(&mut world, vec![partial]).unwrap();
    decode(&packets);
    assert_eq!(packets.len(), 2);
    // Even raw-equal received cells must settle possible native predictions.
    let reconciled = changes(&packets, 1);
    assert_eq!(reconciled.len(), 4096);
    assert!(reconciled.iter().all(|(_, state)| *state == 0));
    assert_eq!(
        reconciled
            .iter()
            .map(|(local, _)| *local)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4096
    );
    let light = light(&packets);
    assert!(named(&light.empty_block_mask, 2));
    assert!(!named(&light.block_mask, 1) && !named(&light.empty_block_mask, 1));
    assert!(!named(&light.sky_mask, 1) && !named(&light.empty_sky_mask, 1));
    // The old server cache still has the previous network nibbles; they must not be resent.
    assert_eq!(
        world.column(2, -3).unwrap().sections[0]
            .as_ref()
            .unwrap()
            .block_light[0],
        0xff
    );
}

#[test]
fn all_contextual_cells_in_received_sections_are_refreshed_even_when_raw_state_is_unchanged() {
    let mut world = World::default();
    let mut full = update(true, false);
    let mut s = section(false);
    s.states[index(1, 5, 2)] = 54 << 4;
    s.states[index(2, 5, 2)] = 1 << 4;
    put(&mut full, 3, s.clone());
    apply(&mut world, vec![full]).unwrap();
    let mut partial = update(false, false);
    put(&mut partial, 3, s);
    let packets = world
        .apply(
            vec![partial],
            |_, _, _, _, raw| {
                Ok(Block {
                    id: if raw >> 4 == 54 { 1234 } else { i32::from(raw) },
                    ..mapped(0, 0, 0, 0, raw)?
                })
            },
            |_| Ok(1),
            16,
            6,
        )
        .unwrap();
    assert_eq!(changes(&packets, 3).len(), 4096);
    assert!(changes(&packets, 3).contains(&(0x125, 1234)));
    assert!(changes(&packets, 3).contains(&(0x225, 16)));
    decode(&packets);
    assert!(light(&packets).sky_mask.is_empty() && light(&packets).empty_sky_mask.is_empty());
}

#[test]
fn partial_refreshes_contextual_cells_across_both_vertical_section_borders() {
    let mut world = World::default();
    let mut full = update(true, true);
    let mut low = section(true);
    low.states[index(3, 15, 4)] = 64 << 4;
    low.states[index(5, 15, 6)] = 2 << 4;
    let mut high = section(true);
    high.states[index(3, 0, 4)] = (64 << 4) | 8;
    put(&mut full, 0, low);
    put(&mut full, 1, high);
    apply(&mut world, vec![full]).unwrap();
    let mut upper = world.column(2, -3).unwrap().sections[1].clone().unwrap();
    upper.states[index(5, 0, 6)] = 78 << 4;
    let mut partial = update(false, true);
    put(&mut partial, 1, upper);
    let snapshot = world.snapshot(&[partial.clone()]).unwrap();
    let packets = world
        .apply(
            vec![partial],
            |x, z, y, i, raw| {
                let mut value = mapped(x, z, y, i, raw)?;
                if raw >> 4 == 2 {
                    value.id = if raw_in(&snapshot[&(x, z)], [x * 16 + 5, 16, z * 16 + 6])
                        == Some(78 << 4)
                    {
                        2001
                    } else {
                        2000
                    };
                }
                Ok(value)
            },
            |_| Ok(1),
            16,
            6,
        )
        .unwrap();
    assert!(changes(&packets, 0).contains(&(0x56f, 2001))); // Snowy grass outside the changed section.
    assert!(!named(&light(&packets).sky_mask, 1));
    let mut lower = world.column(2, -3).unwrap().sections[0].clone().unwrap();
    lower.states[index(3, 15, 4)] = (64 << 4) | 1;
    let mut partial = update(false, true);
    put(&mut partial, 0, lower);
    let snapshot = world.snapshot(&[partial.clone()]).unwrap();
    let packets = world
        .apply(
            vec![partial],
            |x, z, y, i, raw| {
                let mut value = mapped(x, z, y, i, raw)?;
                if raw == (64 << 4) | 8 {
                    value.id = 4000
                        + i32::from(
                            raw_in(&snapshot[&(x, z)], [x * 16 + 3, 15, z * 16 + 4]).unwrap() & 3,
                        );
                }
                Ok(value)
            },
            |_| Ok(1),
            16,
            6,
        )
        .unwrap();
    assert!(changes(&packets, 1).contains(&(0x340, 4001)));
    decode(&packets);
    assert!(!named(&light(&packets).block_mask, 2) && !named(&light(&packets).empty_block_mask, 2));
}

#[test]
fn partial_failure_rolls_back_the_entire_batch_and_does_not_emit_invalid_light() {
    let mut world = World::default();
    let mut full = update(true, true);
    put(&mut full, 0, section(true));
    apply(&mut world, vec![full]).unwrap();
    let before = world.column(2, -3).unwrap().clone();
    let mut added = update(true, true);
    added.x = 3;
    let mut partial = update(false, true);
    let mut s = section(true);
    s.states[0] = u16::MAX;
    put(&mut partial, 0, s);
    assert!(
        world
            .apply(
                vec![added, partial],
                |x, z, y, i, raw| if raw == u16::MAX {
                    Err(Error::new("fixture mapping failure"))
                } else {
                    mapped(x, z, y, i, raw)
                },
                |_| Ok(1),
                16,
                6
            )
            .is_err()
    );
    assert_eq!(world.column(2, -3), Some(&before));
    assert!(world.column(3, -3).is_none());
    let mut partial = update(false, true);
    put(&mut partial, 0, section(false));
    assert!(apply(&mut world, vec![partial]).is_err());
    assert_eq!(world.column(2, -3), Some(&before));
}

#[test]
fn empty_partial_is_a_noop_full_and_unload_packets_keep_their_existing_semantics() {
    let mut world = World::default();
    let packets = apply(&mut world, vec![update(true, true)]).unwrap();
    decode(&packets);
    assert!(matches!(
        packets.as_slice(),
        [Clientbound::LevelChunkWithLight(_)]
    ));
    let before = world.column(2, -3).unwrap().clone();
    assert!(
        apply(&mut world, vec![update(false, true)])
            .unwrap()
            .is_empty()
    );
    assert_eq!(world.column(2, -3), Some(&before));
    let mut gone = update(true, true);
    gone.unload = true;
    assert!(matches!(
        apply(&mut world, vec![gone]).unwrap().as_slice(),
        [Clientbound::ForgetLevelChunk(_)]
    ));
    assert!(world.column(2, -3).is_none());
}

#[test]
fn received_bottom_light_refreshes_negative_y_padding_in_mask_order() {
    let mut world = World::default();
    let mut full = update(true, true);
    put(&mut full, 0, section(true));
    apply(&mut world, vec![full]).unwrap();
    let mut received = update(false, true);
    let mut bottom = section(true);
    bottom.block_light.fill(0xa5);
    bottom.sky_light.as_mut().unwrap().fill(0xb6);
    for i in 0..128 {
        bottom.block_light[i] = i as u8;
        bottom.sky_light.as_mut().unwrap()[i] = 255 - i as u8;
    }
    let expected_bottom = bottom.clone();
    put(&mut received, 0, bottom);
    let mut high = section(true);
    high.block_light.fill(0xc7);
    high.sky_light.as_mut().unwrap().fill(0xd8);
    put(&mut received, 5, high);
    let packets = apply(&mut world, vec![received]).unwrap();
    decode(&packets);
    let light = light(&packets);
    for bit in 0..18 {
        assert_eq!(named(&light.block_mask, bit), matches!(bit, 0 | 1 | 6));
        assert_eq!(named(&light.sky_mask, bit), matches!(bit, 0 | 1 | 6));
    }
    assert_eq!(light.block.len(), 3);
    assert_eq!(light.sky.len(), 3);
    for i in 0..2048 {
        assert_eq!(light.block[0][i], expected_bottom.block_light[i % 128]);
        assert_eq!(
            light.sky[0][i],
            expected_bottom.sky_light.as_ref().unwrap()[i % 128]
        );
    }
    assert_eq!(light.block[1], expected_bottom.block_light);
    assert_eq!(light.sky[1], expected_bottom.sky_light.unwrap());
    assert_eq!(light.block[2], vec![0xc7; 2048]);
    assert_eq!(light.sky[2], vec![0xd8; 2048]);
}

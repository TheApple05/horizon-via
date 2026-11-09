use super::*;

fn column(x: i32, z: i32, blocks: &[([usize; 3], u16)]) -> Vec<u8> {
    column_kind(x, z, blocks, true)
}
fn column_kind(x: i32, z: i32, blocks: &[([usize; 3], u16)], full: bool) -> Vec<u8> {
    let mut states = vec![0u16; 4096];
    for &([x, y, z], raw) in blocks {
        states[(y * 16 + z) * 16 + x] = raw;
    }
    let mut body = Vec::new();
    for raw in states {
        body.extend(raw.to_le_bytes());
    }
    body.extend(vec![0; 2048]); // No skylight in this synthetic dimension.
    if full {
        body.extend(vec![1; 256]);
    }
    let mut w = Writer::new();
    w.i32(x);
    w.i32(z);
    w.bool(full);
    w.u16(1);
    w.varint(body.len() as i32);
    w.bytes(&body);
    w.data
}

fn state(bridge: &Bridge, pos: [i32; 3]) -> i32 {
    Context(bridge)
        .block_state_checked(i32::from(bridge.terrain.raw_state(pos).unwrap()), pos)
        .unwrap()
}

#[test]
fn door_upper_update_refreshes_lower_state_using_new_atomic_neighbor_context() {
    let mut bridge = Bridge {
        common: Some(common::State::new(1)),
        ..Default::default()
    };
    let lower = [1, 4, 1];
    let upper = [1, 5, 1];
    bridge
        .play(
            0x21,
            &column(0, 0, &[([1, 4, 1], 64 << 4), ([1, 5, 1], (64 << 4) | 8)]),
        )
        .unwrap();
    let before = state(&bridge, lower);
    let packets = bridge.apply_raw_changes(&[(upper, (64 << 4) | 9)]).unwrap();
    let after = state(&bridge, lower);
    assert_ne!(before, after, "upper hinge changes lower rendering");
    assert!(
        packets.iter().any(
            |p| matches!(p,play::Clientbound::BlockUpdate(p) if p.pos==lower && p.state==after)
        )
    );
    assert_eq!(bridge.terrain.raw_state(upper), Some((64 << 4) | 9));
}

#[test]
fn chunk_arrival_refreshes_existing_fence_border_without_erasing_cached_tile_data() {
    let mut bridge = Bridge::default();
    let fence = [15, 4, 1];
    bridge
        .play(
            0x21,
            &column(
                0,
                0,
                &[
                    ([15, 4, 1], 85 << 4),
                    ([2, 4, 2], 140 << 4),
                    ([3, 4, 3], 144 << 4),
                ],
            ),
        )
        .unwrap();
    let before = state(&bridge, fence);
    bridge.pots.borrow_mut().insert([2, 4, 2], 123);
    bridge.skulls.borrow_mut().insert([3, 4, 3], (1, 7));
    let packets = bridge
        .play(0x21, &column(1, 0, &[([0, 4, 1], 1 << 4)]))
        .unwrap();
    let after = state(&bridge, fence);
    assert_ne!(before, after);
    assert!(
        packets.iter().any(
            |p| matches!(p,play::Clientbound::BlockUpdate(p) if p.pos==fence && p.state==after)
        )
    );
    assert_eq!(bridge.pots.borrow().get(&[2, 4, 2]), Some(&123));
    assert_eq!(bridge.skulls.borrow().get(&[3, 4, 3]), Some(&(1, 7)));
    assert_eq!(
        packets
            .iter()
            .filter(|p| matches!(p, play::Clientbound::LevelChunkWithLight(_)))
            .count(),
        1
    );
}

#[test]
fn rejected_block_batch_and_failed_replacement_do_not_mutate_prior_terrain() {
    let mut bridge = Bridge::default();
    bridge
        .play(0x21, &column(0, 0, &[([1, 4, 1], 16)]))
        .unwrap();
    assert!(
        bridge
            .apply_raw_changes(&[([1, 4, 1], 0), ([2, 4, 1], u16::MAX)])
            .is_err()
    );
    assert_eq!(bridge.terrain.raw_state([1, 4, 1]), Some(16));
    bridge.pots.borrow_mut().insert([2, 4, 2], 123);
    assert!(
        bridge
            .play(0x21, &column(0, 0, &[([1, 4, 1], u16::MAX)]))
            .is_err()
    );
    assert_eq!(bridge.terrain.raw_state([1, 4, 1]), Some(16));
    assert_eq!(bridge.pots.borrow().get(&[2, 4, 2]), Some(&123));
}

#[test]
fn actual_explosion_dispatch_updates_old_cache_and_unknown_packets_fail_explicitly() {
    let mut bridge = Bridge::default();
    bridge
        .play(0x21, &column(0, 0, &[([1, 4, 1], 16)]))
        .unwrap();
    let mut w = Writer::new();
    for f in [1., 4., 1., 2.] {
        w.f32(f);
    }
    w.i32(1);
    for _ in 0..3 {
        w.i8(0);
    }
    for _ in 0..3 {
        w.f32(0.);
    }
    let packets = bridge.play(0x27, &w.data).unwrap();
    assert_eq!(bridge.terrain.raw_state([1, 4, 1]), Some(0));
    assert!(
        packets
            .iter()
            .any(|p| matches!(p, play::Clientbound::Explode(_)))
    );
    assert!(bridge.play(0x7f, &[]).is_err());
}

#[test]
fn partial_removing_a_tile_block_forgets_only_its_contextual_cache() {
    let mut bridge = Bridge::default();
    bridge
        .play(
            0x21,
            &column(
                0,
                0,
                &[
                    ([1, 4, 1], 144 << 4),
                    ([2, 4, 1], 140 << 4),
                    ([3, 4, 1], 144 << 4),
                ],
            ),
        )
        .unwrap();
    bridge
        .skulls
        .borrow_mut()
        .extend([([1, 4, 1], (1, 3)), ([3, 4, 1], (2, 7))]);
    bridge.pots.borrow_mut().insert([2, 4, 1], 123);
    let packets = bridge
        .play(
            0x21,
            &column_kind(0, 0, &[([1, 4, 1], 16), ([3, 4, 1], 144 << 4)], false),
        )
        .unwrap();
    assert!(
        !packets
            .iter()
            .any(|p| matches!(p, play::Clientbound::LevelChunkWithLight(_)))
    );
    assert!(!bridge.skulls.borrow().contains_key(&[1, 4, 1]));
    assert!(bridge.pots.borrow().is_empty());
    assert_eq!(bridge.skulls.borrow().get(&[3, 4, 1]), Some(&(2, 7)));
}

//! Java 1.20/1.20.1 packet fields before the 1.20.2 configuration split.
use super::*;
use horizon_world::nbt::Tag;

pub(super) fn clientbound(
    _session: &Session,
    state: State,
    id: i32,
    payload: &[u8],
) -> Result<Option<Vec<u8>>> {
    if state != State::Play {
        return Ok(None);
    }
    use packet_ids::play::clientbound as cb;
    let mut r = Reader::new(payload);
    let mut w = Writer::new();
    match id {
        cb::LOGIN => return Ok(Some(read_login(payload)?.1)),
        cb::RESPAWN => {
            let spawn = Spawn::read(&mut r)?;
            let keep = r.u8()?;
            let tail = SpawnTail::read(&mut r)?;
            spawn.write(&mut w);
            tail.write(&mut w);
            w.u8(keep);
        }
        cb::FORGET_LEVEL_CHUNK => {
            let x = r.i32()?;
            let z = r.i32()?;
            w.i64(((u64::from(z as u32) << 32) | u64::from(x as u32)) as i64);
        }
        cb::SET_DISPLAY_OBJECTIVE => {
            // The old packet reads a signed byte; preserve its value before the later enum codec.
            w.varint(i32::from(r.i8()?));
            w.string(&r.string(32767)?);
        }
        cb::LEVEL_CHUNK_WITH_LIGHT => {
            w.i32(r.i32()?);
            w.i32(r.i32()?);
            let heightmaps = compound(&mut r, false)?;
            w.nbt(&heightmaps);
            let sections = r.byte_array(crate::buf::NBT_QUOTA)?;
            w.byte_array(without_section_padding(&sections)?);
            // Each entry needs packed XZ, Y, at least one type byte and an NBT type byte.
            let count = r.count(r.remaining() / 5)?;
            w.varint(count as i32);
            for _ in 0..count {
                w.u8(r.u8()?);
                w.i16(r.i16()?);
                w.varint(r.varint()?);
                w.nbt(&compound(&mut r, true)?);
            }
            // Light masks and arrays have the same layout. Their normal decoder validates them.
            w.bytes(r.rest());
        }
        cb::BLOCK_ENTITY_DATA => {
            w.i64(r.i64()?);
            w.varint(r.varint()?);
            w.nbt(&compound(&mut r, true)?);
        }
        cb::TAG_QUERY => {
            w.varint(r.varint()?);
            w.nbt(&compound(&mut r, true)?);
        }
        cb::UPDATE_MOB_EFFECT => {
            w.varint(r.varint()?);
            w.varint(r.varint()?);
            w.u8(r.u8()?);
            w.varint(r.varint()?);
            w.u8(r.u8()?);
            let factor = r.bool()?;
            w.bool(factor);
            if factor {
                w.nbt(&compound(&mut r, false)?);
            }
        }
        _ => return Ok(None),
    }
    finish(&r)?;
    Ok(Some(w.data))
}

/// Old Data.getSerializedSize counts the VarInt width of the entry count,
/// whereas Data.write sends the storage's long count. The column allocates the
/// estimated size and transmits its unused zero-filled suffix. Validate both
/// palettes to identify the exact section boundary and discard only that suffix.
fn without_section_padding(buffer: &[u8]) -> Result<&[u8]> {
    let mut r = Reader::new(buffer);
    let mut padding = 0;
    while r.remaining() > padding {
        r.i16()?;
        for (strategy, entries) in [(Strategy::Blocks, 4096), (Strategy::Biomes, 64)] {
            let palette = Paletted::read_prefixed(&mut r, strategy, 0)?;
            let extra = varint_bytes(entries)
                .checked_sub(varint_bytes(palette.data.len()))
                .ok_or_else(|| crate::Error::new("invalid legacy section allocation"))?;
            padding += extra;
        }
    }
    let end = r.position();
    if r.remaining() != padding || r.rest().iter().any(|byte| *byte != 0) {
        return Err(crate::Error::new("invalid 1.20/1.20.1 chunk padding"));
    }
    Ok(&buffer[..end])
}

fn varint_bytes(mut value: usize) -> usize {
    let mut bytes = 1;
    while value >= 128 {
        value >>= 7;
        bytes += 1;
    }
    bytes
}

/// The full old Login's embedded network registry compound, before any virtual configuration.
/// This parses named NBT explicitly; its result is independent of active codec/TLS guards.
pub(crate) fn login_registry(payload: &[u8]) -> Result<Tag> {
    Ok(read_login(payload)?.0)
}

fn read_login(payload: &[u8]) -> Result<(Tag, Vec<u8>)> {
    let mut r = Reader::new(payload);
    let player = r.i32()?;
    let hardcore = r.bool()?;
    let mode = r.u8()?;
    let previous = r.i8()?;
    let count = r.count(r.remaining())?;
    let mut levels = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        levels.push(r.string(32767)?);
    }
    let registry = compound(&mut r, false)?;
    let dimension_type = r.string(32767)?;
    let dimension = r.string(32767)?;
    let seed = r.i64()?;
    let max_players = r.varint()?;
    let view_distance = r.varint()?;
    let simulation_distance = r.varint()?;
    let reduced_debug = r.bool()?;
    let death_screen = r.bool()?;
    let debug = r.bool()?;
    let flat = r.bool()?;
    let tail = SpawnTail::read(&mut r)?;
    finish(&r)?;
    let mut w = Writer::new();
    w.i32(player);
    w.bool(hardcore);
    w.varint(levels.len() as i32);
    for level in levels {
        w.string(&level);
    }
    w.varint(max_players);
    w.varint(view_distance);
    w.varint(simulation_distance);
    w.bool(reduced_debug);
    w.bool(death_screen);
    w.bool(false); // The old client has no doLimitedCrafting login option.
    Spawn {
        dimension_type,
        dimension,
        seed,
        mode,
        previous,
        debug,
        flat,
    }
    .write(&mut w);
    tail.write(&mut w);
    Ok((registry, w.data))
}

struct Spawn {
    dimension_type: String,
    dimension: String,
    seed: i64,
    mode: u8,
    previous: i8,
    debug: bool,
    flat: bool,
}

impl Spawn {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            dimension_type: r.string(32767)?,
            dimension: r.string(32767)?,
            seed: r.i64()?,
            mode: r.u8()?,
            previous: r.i8()?,
            debug: r.bool()?,
            flat: r.bool()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.string(&self.dimension_type);
        w.string(&self.dimension);
        w.i64(self.seed);
        w.u8(self.mode);
        w.i8(self.previous);
        w.bool(self.debug);
        w.bool(self.flat);
    }
}

struct SpawnTail {
    death: Option<(String, i64)>,
    portal_cooldown: i32,
}

impl SpawnTail {
    fn read(r: &mut Reader) -> Result<Self> {
        let death = if r.bool()? {
            Some((r.string(32767)?, r.i64()?))
        } else {
            None
        };
        Ok(Self {
            death,
            portal_cooldown: r.varint()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.bool(self.death.is_some());
        if let Some((dimension, position)) = &self.death {
            w.string(dimension);
            w.i64(*position);
        }
        w.varint(self.portal_cooldown);
    }
}

fn compound(r: &mut Reader, optional: bool) -> Result<Tag> {
    let tag = r.named_nbt()?;
    if matches!(tag, Tag::Compound(_)) || optional && matches!(tag, Tag::End) {
        Ok(tag)
    } else {
        Err(crate::Error::new(
            "legacy packet requires a compound NBT root",
        ))
    }
}

fn finish(r: &Reader) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(crate::Error::new("trailing 1.20/1.20.1 packet bytes"))
    }
}

/// The removed AddPlayer packet, encoded directly as a current AddEntity body.
/// The tuple also supplies the entity id and UUID for the connection's entity bookkeeping.
pub(crate) fn old_add_player(session: &Session, payload: &[u8]) -> Result<(Vec<u8>, i32, u128)> {
    let mut r = Reader::new(payload);
    let id = r.varint()?;
    let uuid = r.uuid()?;
    let position = crate::buf::Vec3d::read(&mut r)?;
    let yaw = r.i8()?;
    let pitch = r.i8()?;
    finish(&r)?;
    // Both official763 registries place minecraft:player at122.
    let kind = session
        .version
        .ids()
        .and_then(|ids| ids.entity_type.up(122))
        .ok_or_else(|| crate::Error::new("missing legacy player entity type"))?;
    let entity = play::AddEntity {
        id,
        uuid,
        kind,
        position,
        velocity: [0.0; 3],
        pitch,
        yaw,
        head_yaw: yaw,
        data: 0,
    };
    let mut w = Writer::new();
    play::AddEntity::write(&entity, &mut w);
    Ok((w.data, id, uuid))
}

/// 1.20.2 inserts Display's position/rotation interpolation duration at10.
/// Root calls this only for positively identified block/item/text display entities.
pub(crate) fn display_metadata_index(index: u8) -> Result<u8> {
    if index < 10 {
        return Ok(index);
    }
    index
        .checked_add(1)
        .filter(|i| *i != u8::MAX)
        .ok_or_else(|| crate::Error::new("legacy display metadata index exceeds the wire range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(hex: &str) -> Vec<u8> {
        hex.as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect()
    }

    fn converted(id: i32, input: &str) -> Vec<u8> {
        clientbound(
            &Session::new(ServerVersion::V1_20_1),
            State::Play,
            id,
            &wire(input),
        )
        .unwrap()
        .unwrap()
    }

    // Paired official763/764 readers and writers produced these fixtures; both763 releases agree.
    // The empty registry tests framing/reordering, not full synchronized-world registry contents.
    const LOGIN_763: &str = "0000002a0103ff02136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d0a000000136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d1020304050607080140c0a0001010001146d696e6563726166743a7468655f6e6574686572000001ffffff7040ac02";
    const LOGIN_764: &str = "0000002a0102136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d140c0a000100136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d102030405060708003ff010001146d696e6563726166743a7468655f6e6574686572000001ffffff7040ac02";

    #[test]
    fn login_extracts_registry_and_reorders_spawn_fields() {
        assert_eq!(
            converted(packet_ids::play::clientbound::LOGIN, LOGIN_763),
            wire(LOGIN_764)
        );
        let Tag::Compound(registry) = login_registry(&wire(LOGIN_763)).unwrap() else {
            panic!("registry must be a compound");
        };
        assert!(registry.is_empty());
        let mut truncated = wire(LOGIN_763);
        truncated.pop();
        assert!(login_registry(&truncated).is_err());
        let mut trailing = wire(LOGIN_763);
        trailing.push(0);
        assert!(login_registry(&trailing).is_err());
    }

    #[test]
    fn respawn_moves_keep_data_after_death_location_and_cooldown() {
        assert_eq!(
            converted(
                packet_ids::play::clientbound::RESPAWN,
                "136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d102030405060708003ff01000301146d696e6563726166743a7468655f6e6574686572000001ffffff7040ac02"
            ),
            wire(
                "136d696e6563726166743a6f766572776f726c640e6578616d706c653a637573746f6d102030405060708003ff010001146d696e6563726166743a7468655f6e6574686572000001ffffff7040ac0203"
            )
        );
    }

    #[test]
    fn packed_chunk_position_keeps_signed_coordinates() {
        assert_eq!(
            converted(
                packet_ids::play::clientbound::FORGET_LEVEL_CHUNK,
                "00000001fffffffd"
            ),
            wire("fffffffd00000001")
        );
        assert_eq!(
            converted(
                packet_ids::play::clientbound::FORGET_LEVEL_CHUNK,
                "800000007fffffff"
            ),
            wire("7fffffff80000000")
        );
        assert_eq!(
            converted(
                packet_ids::play::clientbound::SET_DISPLAY_OBJECTIVE,
                "12096f626a656374697665"
            ),
            wire("12096f626a656374697665")
        );
    }

    #[test]
    fn named_query_and_block_entity_tags_become_network_tags() {
        assert_eq!(
            converted(
                packet_ids::play::clientbound::TAG_QUERY,
                "ac020a00000800056c6162656c0005717565727900"
            ),
            wire("ac020a0800056c6162656c0005717565727900")
        );
        // NbtIo.read accepts a nonempty root name; it is not a component/property name.
        assert_eq!(
            converted(
                packet_ids::play::clientbound::TAG_QUERY,
                "ac020a0004776972650800056c6162656c0005717565727900"
            ),
            wire("ac020a0800056c6162656c0005717565727900")
        );
        assert_eq!(
            converted(
                packet_ids::play::clientbound::BLOCK_ENTITY_DATA,
                "0000007fffffdfc0000a00000800056c6162656c0005626c6f636b00"
            ),
            wire("0000007fffffdfc0000a0800056c6162656c0005626c6f636b00")
        );
        assert_eq!(
            converted(packet_ids::play::clientbound::TAG_QUERY, "ac0200"),
            wire("ac0200")
        );
    }

    // Actual vanilla LevelChunkSection constructors/writers produced these four
    // palettes: singleton, mixed blocks, mixed biomes, then both mixed. The old
    // getSerializedSize totals 4156, despite writing exactly 4154 bytes; 764's
    // corrected calculation totals 4154. Both 763 releases produced identical
    // section bytes and full packet hashes, including three 2048-byte light layers.
    fn official_sections() -> Vec<Vec<u8>> {
        let single = wire("0000000000000000");
        let mut blocks = wire("00010402000180020000000000000001");
        blocks.resize(2056, 0);
        let mut states_mixed = blocks.clone();
        states_mixed.extend_from_slice(&[0, 0, 0]);
        let biomes_mixed = wire("000000000001020001010000000000000001");
        let mut both_mixed = blocks;
        both_mixed.extend_from_slice(&wire("01020001010000000000000001"));
        vec![single, states_mixed, biomes_mixed, both_mixed]
    }

    fn official_chunk(old: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.i32(1);
        w.i32(-3);
        let heights = Tag::compound([
            ("MOTION_BLOCKING", Tag::LongArray(vec![0; 29])),
            ("WORLD_SURFACE", Tag::LongArray(vec![0; 29])),
        ]);
        if old {
            w.named_nbt(&heights);
        } else {
            w.nbt(&heights);
        }
        let mut sections = official_sections().concat();
        if old {
            sections.extend_from_slice(&[0, 0]);
        }
        w.byte_array(&sections);
        w.varint(1);
        w.u8(0x2f);
        w.i16(-64);
        w.varint(0);
        let block = Tag::compound([("label", Tag::String("light_fixture".into()))]);
        if old {
            w.named_nbt(&block);
        } else {
            w.nbt(&block);
        }
        for mask in [5, 2, 58, 61] {
            w.varint(1);
            w.i64(mask);
        }
        w.varint(2);
        w.byte_array(&vec![0x11; 2048]);
        w.byte_array(&vec![0x22; 2048]);
        w.varint(1);
        w.byte_array(&vec![0x33; 2048]);
        w.data
    }

    #[test]
    fn chunk_strips_only_the_exact_vanilla_allocated_padding() {
        for (section, padding) in official_sections().iter().zip([1, 0, 1, 0]) {
            let mut allocated = section.clone();
            allocated.resize(allocated.len() + padding, 0);
            assert_eq!(without_section_padding(&allocated).unwrap(), section);
        }
        let sections = official_sections().concat();
        assert_eq!(sections.len(), 4154);
        let mut allocated = sections.clone();
        allocated.extend_from_slice(&[0, 0]);
        assert_eq!(without_section_padding(&allocated).unwrap(), sections);
        let mut nonzero = allocated.clone();
        *nonzero.last_mut().unwrap() = 1;
        assert!(without_section_padding(&nonzero).is_err());
        allocated.pop();
        assert!(without_section_padding(&allocated).is_err());
        allocated.extend_from_slice(&[0, 0]);
        assert!(without_section_padding(&allocated).is_err());
    }

    #[test]
    fn actual_constructed_chunk_reaches_current_decoder_with_nonempty_light() {
        use sha2::Digest;
        let old = official_chunk(true);
        let new = official_chunk(false);
        // Whole-packet independent Java writer goldens bind these compact
        // fixtures to the actual producer, not just a DTO's opaque byte array.
        assert_eq!(old.len(), 10896);
        assert_eq!(new.len(), 10890);
        assert_eq!(
            &sha2::Sha256::digest(&old)[..],
            wire("16ae826e24113c42a8473f025a9bb48add17e3e587279274860133b82f799530")
        );
        assert_eq!(
            &sha2::Sha256::digest(&new)[..],
            wire("94d086597ce5672b516d458099941b88bbc828c5887c5ef8d8f38e9babbcdc41")
        );
        let session = Session::new(ServerVersion::V1_20_1);
        assert_eq!(
            clientbound(
                &session,
                State::Play,
                packet_ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT,
                &old
            )
            .unwrap()
            .unwrap(),
            new
        );
        let normalize = |version, payload: &[u8]| {
            let session = Session::new(version);
            let (id, body) = super::super::clientbound(
                &session,
                State::Play,
                packet_ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT,
                payload,
            )
            .unwrap()
            .unwrap();
            (id, body.unwrap_or_else(|| payload.to_vec()))
        };
        let canonical = normalize(ServerVersion::V1_20_1, &old);
        assert_eq!(canonical, normalize(ServerVersion::V1_20_2, &new));
        let play::Clientbound::LevelChunkWithLight(packet) =
            play::Clientbound::decode(canonical.0, &canonical.1).unwrap()
        else {
            panic!("expected current LevelChunkWithLight");
        };
        assert_eq!((packet.x, packet.z), (1, -3));
        assert_eq!(packet.chunk.heightmaps.len(), 2);
        assert!(
            packet
                .chunk
                .heightmaps
                .iter()
                .all(|(_, data)| data == &vec![0; 29])
        );
        assert_eq!(packet.chunk.block_entities.len(), 1);
        assert_eq!(packet.chunk.block_entities[0].y, -64);
        assert_eq!(
            packet.chunk.block_entities[0].tag.get("label"),
            Some(&Tag::String("light_fixture".into()))
        );
        let mut data = Reader::new(&packet.chunk.buffer);
        let mut sections = Vec::new();
        for _ in 0..4 {
            sections.push(Section::read(&mut data, 16, 1).unwrap());
        }
        finish(&data).unwrap(); // No padding or phantom section reaches the renderer.
        assert_eq!(
            sections.iter().map(|s| s.non_empty).collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
        assert!(sections.iter().all(|s| s.fluids == 0));
        let stone = session.version.ids().unwrap().block_state.up(1).unwrap();
        for (section, mixed) in sections.iter().zip([false, true, false, true]) {
            assert_eq!(section.states.get(0), if mixed { stone } else { 0 });
            assert_eq!(section.states.get(1), 0);
        }
        for (section, mixed) in sections.iter().zip([false, false, true, true]) {
            assert_eq!(section.biomes.get(0), i32::from(mixed));
            assert_eq!(section.biomes.get(1), 0);
        }
        assert_eq!(packet.light.sky_mask, [5]);
        assert_eq!(packet.light.block_mask, [2]);
        assert_eq!(packet.light.empty_sky_mask, [58]);
        assert_eq!(packet.light.empty_block_mask, [61]);
        assert_eq!(packet.light.sky, [vec![0x11; 2048], vec![0x22; 2048]]);
        assert_eq!(packet.light.block, [vec![0x33; 2048]]);
        assert!(
            clientbound(
                &session,
                State::Play,
                packet_ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT,
                &wire("00000001fffffffd000000")
            )
            .is_err()
        );
    }

    #[test]
    fn effect_factor_named_tag_keeps_values_and_the_unmapped_effect_id() {
        let body = converted(
            packet_ids::play::clientbound::UPDATE_MOB_EFFECT,
            "ac0201022805010a0000050015666163746f725f70726576696f75735f6672616d653e80000005000c666163746f725f73746172740000000003000c7469636b735f6163746976650000000305000d666163746f725f7461726765743f80000005000e666163746f725f63757272656e743f0000000100146861645f6566666563745f6c6173745f7469636b0103001070616464696e675f6475726174696f6e0000000100",
        );
        let mut r = Reader::new(&body);
        assert_eq!(r.varint().unwrap(), 300);
        assert_eq!(r.varint().unwrap(), 1); //763 speed; the shared registry step remaps it once.
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(r.varint().unwrap(), 40);
        assert_eq!(r.u8().unwrap(), 5);
        assert!(r.bool().unwrap());
        assert_eq!(
            r.nbt().unwrap(),
            Tag::compound([
                ("padding_duration", Tag::Int(1)),
                ("factor_start", Tag::Float(0.0f32.to_bits())),
                ("factor_target", Tag::Float(1.0f32.to_bits())),
                ("factor_current", Tag::Float(0.5f32.to_bits())),
                ("ticks_active", Tag::Int(3)),
                ("factor_previous_frame", Tag::Float(0.25f32.to_bits())),
                ("had_effect_last_tick", Tag::Byte(1)),
            ])
        );
        finish(&r).unwrap();
    }

    #[test]
    fn add_player_keeps_wire_orientation_and_supplies_absent_fields() {
        let session = Session::new(ServerVersion::V1_20_1);
        let (body, id, uuid) = old_add_player(&session,
            &wire("ac0200112233445566778899aabbccddeeff3ff40000000000004050000000000000c00600000000000040e0")).unwrap();
        let mut r = Reader::new(&body);
        let entity = play::AddEntity::read(&mut r).unwrap();
        finish(&r).unwrap();
        assert_eq!(id, 300);
        assert_eq!(uuid, 0x00112233445566778899aabbccddeeff);
        assert_eq!((entity.id, entity.uuid, entity.kind), (300, uuid, 159));
        assert_eq!(entity.position, [1.25, 64.0, -2.75]);
        assert_eq!((entity.yaw, entity.pitch), (64, -32));
        assert_eq!(entity.velocity, [0.0; 3]);
        assert_eq!(entity.data, 0);
        assert_eq!(entity.head_yaw, 64); // Deterministic fallback; old packet has no head rotation.
    }

    #[test]
    fn display_accessor_shift_preserves_interpolation_and_subclass_indices() {
        // Actual static accessor oracle: 8/9 stay, translation10->11, glow21->22,
        // subclass text/item/block22->23, final text style flags26->27.
        for (old, new) in [
            (8, 8),
            (9, 9),
            (10, 11),
            (11, 12),
            (12, 13),
            (13, 14),
            (14, 15),
            (15, 16),
            (16, 17),
            (17, 18),
            (18, 19),
            (19, 20),
            (20, 21),
            (21, 22),
            (22, 23),
            (23, 24),
            (24, 25),
            (25, 26),
            (26, 27),
        ] {
            assert_eq!(display_metadata_index(old).unwrap(), new);
        }
        assert!(display_metadata_index(254).is_err());
    }
}

//! Native protocol754 boundary. Outputs are already pinned consumer packets.
use super::{
    ServerVersion, ids as registry_ids,
    v1_8_9::bridge::{Incoming, Outgoing},
};
use crate::{
    Error, Result, State,
    buf::{Codec, Reader, Writer},
    ids,
    packets::{configuration as cfg, login, play},
};
use horizon_world::nbt::Tag;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, OnceLock},
};

mod bootstrap;
mod commands;
mod entity;
mod inventory;
pub mod tables;
const CAT_VARIANTS: &[&str] = &[
    "minecraft:tabby",
    "minecraft:black",
    "minecraft:red",
    "minecraft:siamese",
    "minecraft:british_shorthair",
    "minecraft:calico",
    "minecraft:persian",
    "minecraft:ragdoll",
    "minecraft:white",
    "minecraft:jellie",
    "minecraft:all_black",
];
mod terrain;
mod worldfx;

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing 1.16.5 packet bytes"))
    }
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid 1.16.5 boolean")),
    }
}
fn up(table: &[i32], id: i32) -> Result<i32> {
    usize::try_from(id)
        .ok()
        .and_then(|id| table.get(id))
        .copied()
        .filter(|id| *id >= 0)
        .ok_or_else(|| Error::new("unmapped 1.16.5 registry id"))
}
fn packet(bytes: Vec<u8>) -> (i32, Vec<u8>) {
    let mut r = Reader::new(&bytes);
    let id = r.varint().expect("encoded packet id");
    (id, r.rest().to_vec())
}
fn wire(id: i32, w: Writer) -> Vec<u8> {
    let mut out = Writer::new();
    out.varint(id);
    out.bytes(&w.data);
    out.data
}
fn text(r: &mut Reader<'_>) -> Result<Tag> {
    super::read_legacy_component(r)
}
fn bed_payload(tag: &Tag) -> Result<()> {
    // Vanilla754 BedBlockEntity adds no saved fields; current beds render from block state.
    match tag {
        Tag::End => Ok(()),
        Tag::Compound(fields)
            if fields
                .keys()
                .all(|key| matches!(key.to_string_lossy().as_str(), "id" | "x" | "y" | "z")) =>
        {
            Ok(())
        }
        _ => Err(Error::new("old bed has unsupported block-entity data")),
    }
}

fn item_ids() -> &'static registry_ids::Ids {
    static IDS: OnceLock<registry_ids::Ids> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut ids = *ServerVersion::V1_20_1.ids().unwrap();
        ids.item = registry_ids::Remap::Runs {
            up: tables::ITEM_UP_RUNS,
            down: tables::ITEM_DOWN_RUNS,
        };
        ids.block_state = registry_ids::Remap::Runs {
            up: tables::STATE_UP_RUNS,
            down: tables::STATE_DOWN_RUNS,
        };
        ids.quirks.legacy_item_data_version = 2586;
        ids
    })
}

pub fn names() -> Arc<registry_ids::LegacyRegistries> {
    static NAMES: OnceLock<Arc<registry_ids::LegacyRegistries>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut result = (*super::v1_8_9::bootstrap::names()).clone();
            for (registry, names, map) in [
                ("minecraft:item", tables::ITEM_NAMES, tables::ITEM_UP),
                ("minecraft:block", tables::BLOCK_NAMES, tables::BLOCK_UP),
                (
                    "minecraft:entity_type",
                    tables::ENTITY_NAMES,
                    tables::ENTITY_UP,
                ),
                (
                    "minecraft:mob_effect",
                    tables::MOB_EFFECT_NAMES,
                    tables::MOB_EFFECT_UP,
                ),
                (
                    "minecraft:attribute",
                    tables::ATTRIBUTE_NAMES,
                    tables::ATTRIBUTE_UP,
                ),
                ("minecraft:potion", tables::POTION_NAMES, tables::POTION_UP),
                (
                    "minecraft:enchantment",
                    tables::ENCHANTMENT_NAMES,
                    tables::ENCHANTMENT_UP,
                ),
            ] {
                let target = result.entry(registry.into()).or_default();
                for (raw, name) in names.iter().enumerate() {
                    if name.is_empty() {
                        continue;
                    }
                    if let Ok(id) = up(map, raw as i32) {
                        target
                            .names
                            .resize(target.names.len().max(id as usize + 1), String::new());
                        if target.names[id as usize].is_empty() {
                            target.names[id as usize] = (*name).into();
                        }
                        target.ids.insert((*name).into(), id);
                    }
                }
            }
            let songs = result.entry("minecraft:jukebox_song".into()).or_default();
            if songs.id("minecraft:pigstep").is_none() {
                let id = songs.names.len() as i32;
                songs.names.push("minecraft:pigstep".into());
                songs.ids.insert("minecraft:pigstep".into(), id);
            }
            result.insert(
                "minecraft:cat_variant".into(),
                crate::registry::Ids::from_names(
                    CAT_VARIANTS.iter().map(|v| (*v).into()).collect(),
                ),
            );
            Arc::new(result)
        })
        .clone()
}

fn read_slot(r: &mut Reader<'_>) -> Result<Option<crate::item::Stack>> {
    let _up = registry_ids::renumbering(item_ids(), registry_ids::Way::Up);
    crate::item::Slot::read(r)
}
fn write_slot(value: &Option<crate::item::Stack>, w: &mut Writer) -> Result<()> {
    let mut encoded = Writer::new();
    {
        let _down = registry_ids::renumbering(item_ids(), registry_ids::Way::Down);
        crate::item::Slot::write(value, &mut encoded);
    }
    if value.is_some() && encoded.data.first() != Some(&1) {
        return Err(Error::new("unrepresentable 1.16.5 item edit"));
    }
    w.bytes(&encoded.data);
    Ok(())
}

#[derive(Default)]
pub struct Bridge {
    online: bool,
    profile: bool,
    profile_name: String,
    acknowledged: bool,
    finish_queued: bool,
    finish_seen: bool,
    finished: bool,
    deferred: Vec<Vec<u8>>,
    registries: Vec<cfg::RegistryData>,
    raw_dimensions: Vec<cfg::RegistryEntry>,
    dimension: String,
    local_player: Option<i32>,
    shifted: bool,
    terrain: terrain::World,
    inventory: inventory::Inventory,
    entities: entity::Entities,
    predictions: Vec<(i32, Vec<u8>)>,
    rejected: bool,
    resource: Option<u128>,
    worldfx: worldfx::State,
}

impl Bridge {
    pub fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        self.entities.take_projectile_acceleration(id)
    }
    pub fn online_mode(&self) -> bool {
        self.online
    }
    pub fn finish_delivered(&mut self) {
        if self.finish_queued {
            self.finish_seen = true;
        }
    }
    pub fn take_predictions(&mut self) -> Vec<(i32, Vec<u8>)> {
        std::mem::take(&mut self.predictions)
    }
    pub fn rejected_locally(&self) -> bool {
        self.rejected
    }

    pub fn clientbound(&mut self, state: State, id: i32, payload: Vec<u8>) -> Result<Incoming> {
        let mut out = Incoming::default();
        match state {
            State::Status => {
                crate::packets::status::Clientbound::decode(id, &payload)?;
                out.packets.push((id, payload));
            }
            State::Login => {
                let mut r = Reader::new(&payload);
                match id {
                    0 => {
                        let reason_json = r.string(262144)?;
                        finish(&r)?;
                        out.packets.push(packet(
                            login::Clientbound::Disconnect(login::Disconnect { reason_json })
                                .encode(),
                        ));
                    }
                    1 => {
                        let hello = login::Hello {
                            server_id: r.string(20)?,
                            public_key: r.byte_array(1 << 20)?,
                            challenge: r.byte_array(1 << 20)?,
                            should_authenticate: true,
                        };
                        finish(&r)?;
                        self.online = true;
                        out.packets
                            .push(packet(login::Clientbound::Hello(hello).encode()));
                    }
                    2 => {
                        if self.profile {
                            return Err(Error::new("duplicate 1.16.5 profile"));
                        }
                        let profile = login::GameProfile {
                            uuid: r.uuid()?,
                            name: r.string(16)?,
                            properties: vec![],
                        };
                        finish(&r)?;
                        self.profile = true;
                        self.profile_name = profile.name.clone();
                        out.packets.push(packet(
                            login::Clientbound::LoginFinished(login::LoginFinished {
                                profile,
                                session: 0,
                            })
                            .encode(),
                        ));
                        out.phase = Some(State::Configuration);
                    }
                    3 | 4 => out.packets.push((id, payload)),
                    _ => return Err(Error::new("unknown 1.16.5 login packet")),
                }
            }
            State::Configuration => {
                if id == 31 {
                    let mut r = Reader::new(&payload);
                    let id = r.i64()?;
                    finish(&r)?;
                    out.packets.push(packet(
                        cfg::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode(),
                    ));
                    return Ok(out);
                }
                if id == 25 {
                    let mut r = Reader::new(&payload);
                    let reason = text(&mut r)?;
                    finish(&r)?;
                    out.packets.push(packet(
                        cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode(),
                    ));
                    return Ok(out);
                }
                if id != 36 || !self.profile || self.finish_queued {
                    return Err(Error::new("1.16.5 play packet precedes login"));
                }
                let join = bootstrap::join(&payload, self.online)?;
                self.raw_dimensions = join.dimensions;
                self.registries = join.registries;
                self.terrain.set_biomes(join.biomes)?;
                self.dimension = join.login.spawn.dimension.clone();
                self.local_player = Some(join.login.player_id);
                self.entities.register_player(join.login.player_id);
                for registry in &self.registries {
                    out.packets.push(packet(
                        cfg::Clientbound::RegistryData(registry.clone()).encode(),
                    ));
                }
                out.packets.push(packet(
                    cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
                        features: vec!["minecraft:vanilla".into()],
                    })
                    .encode(),
                ));
                out.packets.push((
                    ids::configuration::clientbound::FINISH_CONFIGURATION,
                    vec![],
                ));
                out.packets
                    .push(packet(play::Clientbound::Login(join.login).encode()));
                out.packets
                    .push(packet(super::v1_8_9::bootstrap::commands().encode()));
                out.packets.push(packet(
                    play::Clientbound::GameEvent(play::GameEvent {
                        event: 13,
                        param: 0.0,
                    })
                    .encode(),
                ));
                self.finish_queued = true;
                out.phase = Some(State::Play);
            }
            State::Play => {
                let (packets, extra) = self.play(id, &payload)?;
                self.terrain.apply_updates(&packets)?;
                out.packets
                    .extend(packets.into_iter().map(|v| packet(v.encode())));
                out.outgoing.extend(extra);
            }
            State::Handshake => return Err(Error::new("1.16.5 packet precedes handshake")),
        }
        Ok(out)
    }

    fn play(&mut self, id: i32, payload: &[u8]) -> Result<(Vec<play::Clientbound>, Vec<Vec<u8>>)> {
        use play::Clientbound as C;
        if let Some(result) = self.inventory.clientbound(id, payload)? {
            return Ok(result);
        }
        if let Some(packets) = self.entities.clientbound(id, payload)? {
            return Ok((packets, vec![]));
        }
        if let Some(packets) = self.worldfx.clientbound(id, payload)? {
            return Ok((packets, vec![]));
        }
        let mut r = Reader::new(payload);
        let mut out = vec![];
        match id {
            8 => out.push(C::BlockDestruction(play::BlockDestruction::read(&mut r)?)),
            9 => {
                let pos = crate::buf::Pos::read(&mut r)?;
                let action = r.u8()?;
                let mut tag = r.named_nbt()?;
                if action == 11 {
                    bed_payload(&tag)?;
                    finish(&r)?;
                    return Ok((vec![], vec![]));
                }
                let name = match action {
                    1 => "mob_spawner",
                    2 => "command_block",
                    3 => "beacon",
                    4 => "skull",
                    5 => "conduit",
                    6 => "banner",
                    7 => "structure_block",
                    8 => "end_gateway",
                    9 => "sign",
                    10 => "shulker_box",
                    11 => "bed",
                    12 => "jigsaw",
                    13 => "campfire",
                    14 => "beehive",
                    _ => return Err(Error::new("unknown 1.16.5 block entity action")),
                };
                let raw = tables::BLOCK_ENTITY_NAMES
                    .iter()
                    .position(|v| v.strip_prefix("minecraft:") == Some(name))
                    .ok_or_else(|| Error::new("missing old block entity type"))?;
                let kind = up(tables::BLOCK_ENTITY_UP, raw as i32)?;
                horizon_world::datafix::update(
                    horizon_world::datafix::schema::Ty::BlockEntity,
                    &mut tag,
                    2586,
                    horizon_world::datafix::CURRENT,
                );
                out.push(C::BlockEntityData(play::BlockEntityData { pos, kind, tag }));
            }
            10 => {
                let pos = crate::buf::Pos::read(&mut r)?;
                let b0 = r.u8()?;
                let b1 = r.u8()?;
                let block = up(tables::BLOCK_UP, r.varint()?)?;
                out.push(C::BlockEvent(play::BlockEvent { pos, b0, b1, block }));
            }
            11 => {
                let pos = crate::buf::Pos::read(&mut r)?;
                let state = up(tables::STATE_UP, r.varint()?)?;
                out.push(C::BlockUpdate(play::BlockUpdate { pos, state }));
            }
            13 => {
                out.push(C::ChangeDifficulty(play::ChangeDifficulty {
                    difficulty: i32::from(r.u8()?),
                    locked: boolean(&mut r)?,
                }));
            }
            14 => {
                let content = text(&mut r)?;
                let kind = r.u8()?;
                r.uuid()?;
                if kind > 2 {
                    return Err(Error::new("invalid old chat position"));
                }
                out.push(C::SystemChat(play::SystemChat {
                    content,
                    overlay: kind == 2,
                }));
            }
            16 => out.push(C::Commands(commands::read(&mut r)?)),
            22 => {
                let raw = r.varint()?;
                let duration = r.varint()?;
                let group = tables::ITEM_NAMES
                    .get(usize::try_from(raw).map_err(|_| Error::new("negative cooldown item"))?)
                    .ok_or_else(|| Error::new("unknown cooldown item"))?
                    .to_string();
                out.push(C::Cooldown(play::Cooldown { group, duration }));
            }
            23 => out.push(C::CustomPayload(cfg::CustomPayload::read(&mut r)?)),
            25 => out.push(C::Disconnect(cfg::Disconnect {
                reason: text(&mut r)?,
            })),
            26 => out.push(C::EntityEvent(play::EntityEvent::read(&mut r)?)),
            28 => {
                let x = r.i32()?;
                let z = r.i32()?;
                self.terrain.forget(x, z);
                out.push(C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
            }
            29 => out.push(C::GameEvent(play::GameEvent::read(&mut r)?)),
            31 => out.push(C::KeepAlive(cfg::KeepAlive::read(&mut r)?)),
            32 => return self.terrain.chunk(payload).map(|p| (vec![p], vec![])),
            33 => out.push(C::LevelEvent(play::LevelEvent::read(&mut r)?)),
            35 => return self.terrain.light(payload).map(|p| (vec![p], vec![])),
            43 => {
                let position = crate::buf::Vec3d::read(&mut r)?;
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                out.push(C::MoveVehicle(play::MoveVehicle {
                    position,
                    yaw,
                    pitch,
                }));
            }
            44 => out.push(C::OpenBook(play::OpenBook::read(&mut r)?)),
            46 => {
                let pos = crate::buf::Pos::read(&mut r)?;
                out.push(C::OpenSignEditor(play::OpenSignEditor { pos, front: true }));
            }
            48 => out.push(C::PlayerAbilities(play::PlayerAbilities::read(&mut r)?)),
            50 => out.extend(player_info(&mut r)?),
            51 => out.push(C::PlayerLookAt(play::PlayerLookAt::read(&mut r)?)),
            52 => {
                let position = crate::buf::Vec3d::read(&mut r)?;
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let relatives = i32::from(r.u8()?);
                let teleport = r.varint()?;
                out.push(C::PlayerPosition(play::PlayerPosition {
                    teleport,
                    position,
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives,
                }));
            }
            56 => {
                let url = r.string(32767)?;
                let hash = r.string(40)?;
                let id = 0x754;
                self.resource = Some(id);
                out.push(C::ResourcePackPush(cfg::ResourcePackPush {
                    id,
                    url,
                    hash,
                    required: false,
                    prompt: None,
                }));
            }
            57 => {
                let dimension = r.named_nbt()?;
                let dimension_type = self
                    .raw_dimensions
                    .iter()
                    .position(|e| e.data.as_ref() == Some(&dimension))
                    .ok_or_else(|| Error::new("unknown old respawn dimension"))?
                    as i32;
                let world = r.string(32767)?;
                let seed = r.i64()?;
                let mode = r.u8()?;
                let previous = r.i8()?;
                let debug = boolean(&mut r)?;
                let flat = boolean(&mut r)?;
                let keep = boolean(&mut r)?;
                if mode > 3 || !(-1..=3).contains(&previous) {
                    return Err(Error::new("invalid old respawn game mode"));
                }
                self.dimension = world.clone();
                self.terrain.clear();
                self.entities.clear();
                self.inventory.respawn();
                if let Some(id) = self.local_player {
                    self.entities.register_player(id);
                }
                out.push(C::Respawn(play::Respawn {
                    spawn: play::SpawnInfo {
                        dimension_type,
                        dimension: world,
                        seed,
                        game_type: i32::from(mode),
                        previous_game_type: (previous >= 0).then_some(i32::from(previous)),
                        debug,
                        flat,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    keep: if keep { 3 } else { 0 },
                }));
            }
            59 => {
                let section = crate::buf::SectionPos::read(&mut r)?;
                boolean(&mut r)?;
                let n = r.count(4096)?;
                let mut changes = Vec::with_capacity(n);
                for _ in 0..n {
                    let raw = r.varlong()?;
                    let state = up(
                        tables::STATE_UP,
                        i32::try_from(raw >> 12)
                            .map_err(|_| Error::new("invalid old block update"))?,
                    )?;
                    changes.push((i64::from(state) << 12) | (raw & 4095));
                }
                out.push(C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                    section,
                    changes,
                }));
            }
            60 => out.push(C::SelectAdvancementsTab(play::SelectAdvancementsTab::read(
                &mut r,
            )?)),
            62 => out.push(C::SetCamera(play::SetCamera::read(&mut r)?)),
            63 => {
                let slot = r.u8()?;
                out.push(C::SetHeldSlot(play::SetHeldSlot {
                    slot: i32::from(slot),
                }));
            }
            64 => out.push(C::SetChunkCacheCenter(play::SetChunkCacheCenter::read(
                &mut r,
            )?)),
            65 => out.push(C::SetChunkCacheRadius(play::Distance::read(&mut r)?)),
            66 => {
                let pos = crate::buf::Pos::read(&mut r)?;
                out.push(C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
                    pos: play::GlobalPos {
                        dimension: self.dimension.clone(),
                        pos,
                    },
                    yaw: 0.0,
                    pitch: 0.0,
                }));
            }
            69 => out.push(C::SetEntityLink(play::SetEntityLink::read(&mut r)?)),
            72 => out.push(C::SetExperience(play::SetExperience::read(&mut r)?)),
            73 => out.push(C::SetHealth(play::SetHealth::read(&mut r)?)),
            75 => out.push(C::SetPassengers(play::SetPassengers::read(&mut r)?)),
            78 => {
                let game_time = r.i64()?;
                let time = r.i64()?;
                out.push(C::SetTime(play::SetTime {
                    game_time,
                    clocks: vec![play::ClockState {
                        clock: 0,
                        total_ticks: time.saturating_abs(),
                        partial_tick: 0.0,
                        rate: if time < 0 { 0.0 } else { 1.0 },
                    }],
                }));
            }
            83 => out.push(C::TabList(play::TabList {
                header: text(&mut r)?,
                footer: text(&mut r)?,
            })),
            84 => out.push(C::TagQuery(play::TagQuery {
                transaction: r.varint()?,
                tag: r.named_nbt()?,
            })),
            85 => out.push(C::TakeItemEntity(play::TakeItemEntity::read(&mut r)?)),
            91 => out.push(C::UpdateTags(tags(&mut r)?)),
            // These old payload families need their own modern recipe/advancement/map semantics.
            6 | 7 | 12 | 15 | 24 | 27 | 34 | 37 | 38 | 47 | 49 | 53 | 61 | 67 | 74 | 76 | 77
            | 79 | 80 | 81 | 82 | 87 | 88 | 89 | 90 => {
                return Err(Error::new(format!(
                    "unimplemented 1.16.5 packet family {id}"
                )));
            }
            _ => return Err(Error::new(format!("unknown 1.16.5 play packet {id}"))),
        }
        finish(&r)?;
        Ok((out, vec![]))
    }

    pub fn serverbound(&mut self, state: State, id: i32, payload: &[u8]) -> Result<Outgoing> {
        self.rejected = false;
        let mut out = Outgoing::default();
        let mut r = Reader::new(payload);
        let mut w = Writer::new();
        match state {
            State::Status => {
                crate::packets::status::Serverbound::decode(id, payload)?;
                w.bytes(payload);
                out.packet = Some(wire(id, w));
            }
            State::Login => match id {
                ids::login::serverbound::HELLO => {
                    let hello = login::ServerHello::read(&mut r)?;
                    w.string(&hello.name);
                    out.packet = Some(wire(0, w));
                }
                ids::login::serverbound::KEY => {
                    w.bytes(payload);
                    out.packet = Some(wire(1, w));
                }
                ids::login::serverbound::CUSTOM_QUERY_ANSWER => {
                    w.bytes(payload);
                    out.packet = Some(wire(2, w));
                }
                ids::login::serverbound::LOGIN_ACKNOWLEDGED => {
                    if !self.profile || self.acknowledged {
                        return Err(Error::new("unexpected old login acknowledgement"));
                    }
                    self.acknowledged = true;
                    out.phase = Some(State::Configuration);
                }
                _ => return Err(Error::new("unsupported 1.16.5 login action")),
            },
            State::Configuration => match id {
                ids::configuration::serverbound::FINISH_CONFIGURATION => {
                    if !self.acknowledged || !self.finish_seen || self.finished {
                        return Err(Error::new("unexpected old configuration finish"));
                    }
                    self.finished = true;
                    out.phase = Some(State::Play);
                    out.extra.append(&mut self.deferred);
                }
                ids::configuration::serverbound::CLIENT_INFORMATION => {
                    let info = cfg::ClientInformation::read(&mut r)?;
                    finish(&r)?;
                    if self.deferred.len() >= 128 {
                        return Err(Error::new("too many deferred old configuration packets"));
                    }
                    self.deferred.push(client_info(info)?);
                }
                ids::configuration::serverbound::KEEP_ALIVE => {
                    w.i64(r.i64()?);
                    out.packet = Some(wire(16, w));
                }
                ids::configuration::serverbound::CUSTOM_PAYLOAD => {
                    let custom = cfg::CustomPayload::read(&mut r)?;
                    finish(&r)?;
                    if custom.data.len() > 32767 || self.deferred.len() >= 128 {
                        return Err(Error::new("deferred old custom payload budget exceeded"));
                    }
                    cfg::CustomPayload::write(&custom, &mut w);
                    self.deferred.push(wire(11, w));
                }
                _ => {}
            },
            State::Play => {
                if id == ids::play::serverbound::COMMAND_SUGGESTION {
                    let request = r.varint()?;
                    let command = r.string(32500)?;
                    finish(&r)?;
                    w.varint(request);
                    w.string(&command);
                    out.packet = Some(wire(6, w));
                    return Ok(out);
                }
                if let Some((translated, corrections, rejected)) =
                    self.inventory.serverbound(id, payload)?
                {
                    out.packet = translated;
                    self.predictions
                        .extend(corrections.into_iter().map(|p| packet(p.encode())));
                    self.rejected = rejected;
                    return Ok(out);
                }
                if !self.finished {
                    return Err(Error::new(
                        "old play action precedes configuration acknowledgement",
                    ));
                }
                use play::Serverbound as S;
                let p = play::Serverbound::decode(id, payload)?;
                let mapped = match p {
                    S::AcceptTeleportation(p) => {
                        w.varint(p.id);
                        Some(0)
                    }
                    S::Attack(p) => {
                        w.varint(p.id);
                        w.varint(1);
                        w.bool(self.shifted);
                        Some(14)
                    }
                    S::Interact(p) => {
                        if !(0..=1).contains(&p.hand)
                            || p.location
                                .iter()
                                .any(|x| !x.is_finite() || (*x as f32).is_infinite())
                        {
                            return Err(Error::new("invalid old entity interaction"));
                        }
                        w.varint(p.id);
                        w.varint(2);
                        for x in p.location {
                            w.f32(x as f32);
                        }
                        w.varint(p.hand);
                        w.bool(p.secondary);
                        Some(14)
                    }
                    S::Chat(p) => {
                        w.string(&p.message);
                        Some(3)
                    }
                    S::ChatCommand(p) => {
                        w.string(&format!("/{}", p.command));
                        Some(3)
                    }
                    S::ChatCommandSigned(p) => {
                        w.string(&format!("/{}", p.command));
                        Some(3)
                    }
                    S::ClientCommand(p) => {
                        if p.action > 1 {
                            None
                        } else {
                            play::ClientCommand::write(&p, &mut w);
                            Some(4)
                        }
                    }
                    S::ClientInformation(p) => {
                        out.packet = Some(client_info(p)?);
                        return Ok(out);
                    }
                    S::CustomPayload(p) => {
                        cfg::CustomPayload::write(&p, &mut w);
                        Some(11)
                    }
                    S::EditBook(p) => {
                        let book = self
                            .inventory
                            .book(p.slot)?
                            .ok_or_else(|| Error::new("book edit targets empty hand"))?;
                        let raw = tables::ITEM_NAMES
                            .iter()
                            .position(|v| *v == "minecraft:writable_book")
                            .ok_or_else(|| Error::new("missing old writable book"))?;
                        if book.item != up(tables::ITEM_UP, raw as i32)? {
                            return Err(Error::new("book edit targets another item"));
                        }
                        let mut old = Writer::new();
                        write_slot(&Some(book), &mut old)?;
                        let mut old = Reader::new(&old.data);
                        boolean(&mut old)?;
                        old.varint()?;
                        old.i8()?;
                        let mut tag = old.named_nbt()?;
                        finish(&old)?;
                        use horizon_world::datafix::dynamic as d;
                        d::set(
                            &mut tag,
                            "pages",
                            Tag::List(p.pages.iter().map(|page| d::string(page)).collect()),
                        );
                        if let Some(title) = &p.title {
                            d::set(&mut tag, "title", d::string(title));
                            d::set(&mut tag, "author", d::string(&self.profile_name));
                        }
                        w.bool(true);
                        w.varint(raw as i32);
                        w.i8(1);
                        w.named_nbt(&tag);
                        w.bool(p.title.is_some());
                        w.varint(p.slot);
                        Some(12)
                    }
                    S::KeepAlive(p) => {
                        cfg::KeepAlive::write(&p, &mut w);
                        Some(16)
                    }
                    S::MovePlayerPos(p) => {
                        w.f64(p.x);
                        w.f64(p.y);
                        w.f64(p.z);
                        w.bool(p.flags & 1 != 0);
                        Some(18)
                    }
                    S::MovePlayerPosRot(p) => {
                        w.f64(p.x);
                        w.f64(p.y);
                        w.f64(p.z);
                        w.f32(p.yaw);
                        w.f32(p.pitch);
                        w.bool(p.flags & 1 != 0);
                        Some(19)
                    }
                    S::MovePlayerRot(p) => {
                        w.f32(p.yaw);
                        w.f32(p.pitch);
                        w.bool(p.flags & 1 != 0);
                        Some(20)
                    }
                    S::MovePlayerStatus(p) => {
                        w.bool(p.flags & 1 != 0);
                        Some(21)
                    }
                    S::MoveVehicle(p) => {
                        crate::buf::Vec3d::write(&p.position, &mut w);
                        w.f32(p.yaw);
                        w.f32(p.pitch);
                        Some(22)
                    }
                    S::PaddleBoat(p) => {
                        play::PaddleBoat::write(&p, &mut w);
                        Some(23)
                    }
                    S::PlayerAbilities(p) => {
                        w.i8(p.flags & 2);
                        Some(26)
                    }
                    S::PlayerAction(p) => {
                        let action = match p.action {
                            0 => Some(0),
                            1 => None,
                            2..=7 => Some(p.action - 1),
                            _ => return Err(Error::new("unsupported old player action")),
                        };
                        if let Some(action) = action {
                            w.varint(action);
                            crate::buf::Pos::write(&p.pos, &mut w);
                            w.u8(p.direction);
                            Some(27)
                        } else {
                            None
                        }
                    }
                    S::PlayerCommand(p) => {
                        w.varint(p.id);
                        w.varint(match p.action {
                            0 => 2,
                            1 => 3,
                            2 => 4,
                            3 => 5,
                            4 => 6,
                            5 => 7,
                            6 => 8,
                            _ => return Err(Error::new("invalid native player command")),
                        });
                        w.varint(p.data);
                        Some(28)
                    }
                    S::PlayerInput(p) => {
                        let shifted = p.flags & 32 != 0;
                        if shifted != self.shifted {
                            let mut command = Writer::new();
                            command.varint(
                                self.local_player
                                    .ok_or_else(|| Error::new("input before player join"))?,
                            );
                            command.varint(i32::from(!shifted));
                            command.varint(0);
                            out.extra.push(wire(28, command));
                            self.shifted = shifted;
                        }
                        w.f32(
                            f32::from(u8::from(p.flags & 4 != 0))
                                - f32::from(u8::from(p.flags & 8 != 0)),
                        );
                        w.f32(
                            f32::from(u8::from(p.flags & 1 != 0))
                                - f32::from(u8::from(p.flags & 2 != 0)),
                        );
                        w.u8(((p.flags >> 4) & 1) | ((p.flags >> 4) & 2));
                        Some(29)
                    }
                    S::RenameItem(p) => {
                        play::RenameItem::write(&p, &mut w);
                        Some(32)
                    }
                    S::ResourcePack(p) => {
                        if self.resource == Some(p.id) {
                            let action = match p.action {
                                0..=3 => Some(p.action),
                                4 => None,
                                5..=7 => Some(2),
                                _ => {
                                    return Err(Error::new(
                                        "invalid native resource-pack response",
                                    ));
                                }
                            };
                            if let Some(action) = action {
                                w.varint(action);
                                Some(33)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    S::SeenAdvancements(p) => {
                        play::SeenAdvancements::write(&p, &mut w);
                        Some(34)
                    }
                    S::SelectTrade(p) => {
                        play::SelectTrade::write(&p, &mut w);
                        Some(35)
                    }
                    S::SetBeacon(p) => {
                        for effect in [p.primary, p.secondary] {
                            w.varint(if let Some(effect) = effect {
                                tables::MOB_EFFECT_UP
                                    .iter()
                                    .position(|id| *id == effect)
                                    .ok_or_else(|| Error::new("effect has no old beacon id"))?
                                    as i32
                            } else {
                                0
                            });
                        }
                        Some(36)
                    }
                    S::SetCarriedItem(p) => {
                        play::SetCarriedItem::write(&p, &mut w);
                        Some(37)
                    }
                    S::PlaceRecipe(p) => {
                        w.u8(u8::try_from(p.container)
                            .map_err(|_| Error::new("old recipe container out of range"))?);
                        w.string(self.worldfx.recipe_key(p.recipe)?);
                        w.bool(p.use_max);
                        Some(25)
                    }
                    S::RecipeBookSeenRecipe(p) => {
                        w.string(self.worldfx.recipe_key(p.recipe)?);
                        Some(30)
                    }
                    S::RecipeBookChangeSettings(p) => {
                        crate::packets::recipes::RecipeBookChangeSettings::write(&p, &mut w);
                        Some(31)
                    }
                    S::UseItemOn(p) => {
                        w.varint(p.hand);
                        crate::buf::Pos::write(&p.pos, &mut w);
                        w.varint(p.face);
                        for x in p.cursor {
                            w.f32(x);
                        }
                        w.bool(p.inside);
                        Some(46)
                    }
                    S::UseItem(p) => {
                        w.varint(p.hand);
                        Some(47)
                    }
                    S::SignUpdate(p) => {
                        if !p.front {
                            self.rejected = true;
                            None
                        } else {
                            crate::buf::Pos::write(&p.pos, &mut w);
                            for line in p.lines {
                                w.string(&line);
                            }
                            Some(43)
                        }
                    }
                    S::Punch(_) => {
                        w.varint(0);
                        Some(44)
                    }
                    S::TeleportToEntity(p) => {
                        play::TeleportToEntity::write(&p, &mut w);
                        Some(45)
                    }
                    S::BlockEntityTagQuery(p) => {
                        play::BlockEntityTagQuery::write(&p, &mut w);
                        Some(1)
                    }
                    S::EntityTagQuery(p) => {
                        play::EntityTagQuery::write(&p, &mut w);
                        Some(13)
                    }
                    S::ChangeDifficulty(p) => {
                        w.u8(u8::try_from(p.difficulty)
                            .ok()
                            .filter(|v| *v <= 3)
                            .ok_or_else(|| Error::new("invalid old difficulty"))?);
                        Some(2)
                    }
                    S::LockDifficulty(p) => {
                        play::LockDifficulty::write(&p, &mut w);
                        Some(17)
                    }
                    S::ChunkBatchReceived(_)
                    | S::PlayerLoaded(_)
                    | S::ClientTickEnd(_)
                    | S::ChatAck(_)
                    | S::ChatSessionUpdate(_) => None,
                    other => {
                        return Err(Error::new(format!(
                            "unsupported native action for 1.16.5: {other:?}"
                        )));
                    }
                };
                if let Some(id) = mapped {
                    out.packet = Some(wire(id, w));
                }
                return Ok(out);
            }
            _ => return Err(Error::new("invalid 1.16.5 sender state")),
        }
        Ok(out)
    }
}

fn client_info(info: cfg::ClientInformation) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    w.string(&info.language);
    w.i8(info.view_distance);
    w.varint(info.chat_visibility);
    w.bool(info.chat_colors);
    w.u8(info.model_customisation);
    w.varint(info.main_hand);
    Ok(wire(5, w))
}

fn player_info(r: &mut Reader<'_>) -> Result<Vec<play::Clientbound>> {
    let action = r.varint()?;
    if !(0..=4).contains(&action) {
        return Err(Error::new("invalid old player info action"));
    }
    let n = r.count(4096)?;
    let mut entries = vec![];
    let mut removed = vec![];
    for _ in 0..n {
        let uuid = r.uuid()?;
        if action == 4 {
            removed.push(uuid);
            continue;
        }
        let mut entry = play::PlayerInfo {
            uuid,
            ..Default::default()
        };
        match action {
            0 => {
                entry.name = Some(r.string(16)?);
                let n = r.count(16)?;
                for _ in 0..n {
                    entry.properties.push(login::Property::read(r)?);
                }
                entry.game_mode = Some(r.varint()?);
                entry.latency = Some(r.varint()?);
                entry.display_name = Some(if boolean(r)? { Some(text(r)?) } else { None });
                entry.listed = Some(true);
                entry.show_hat = Some(true);
            }
            1 => entry.game_mode = Some(r.varint()?),
            2 => entry.latency = Some(r.varint()?),
            3 => entry.display_name = Some(if boolean(r)? { Some(text(r)?) } else { None }),
            _ => unreachable!(),
        }
        entries.push(entry);
    }
    if action == 4 {
        Ok(vec![play::Clientbound::PlayerInfoRemove(
            play::PlayerInfoRemove { uuids: removed },
        )])
    } else {
        Ok(vec![play::Clientbound::PlayerInfoUpdate(
            play::PlayerInfoUpdate {
                actions: match action {
                    0 => 1 | 4 | 8 | 16 | 32 | 128,
                    1 => 4,
                    2 => 16,
                    3 => 32,
                    _ => unreachable!(),
                },
                entries,
            },
        )])
    }
}

fn tags(r: &mut Reader<'_>) -> Result<cfg::UpdateTags> {
    let mut registries = vec![];
    for (registry, table) in [
        ("minecraft:block", tables::BLOCK_UP),
        ("minecraft:item", tables::ITEM_UP),
        ("minecraft:fluid", &[0, 1, 2, 3, 4][..]),
        ("minecraft:entity_type", tables::ENTITY_UP),
    ] {
        let n = r.count(65536)?;
        let mut tags = vec![];
        for _ in 0..n {
            let name = r.string(32767)?;
            let count = r.count(65536)?;
            let mut ids = vec![];
            for _ in 0..count {
                ids.push(up(table, r.varint()?)?);
            }
            tags.push(cfg::TagEntry { name, ids });
        }
        registries.push(cfg::RegistryTags {
            registry: registry.into(),
            tags,
        });
    }
    Ok(cfg::UpdateTags { registries })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> (i32, Vec<u8>) {
        mod goldens {
            include!("fixtures.rs");
        }
        let row = goldens::WRITERS.iter().find(|v| v.0 == name).unwrap();
        let hex = row.2;
        (
            row.1,
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect(),
        )
    }
    #[test]
    fn actual_vanilla_login_and_teleport_preserve_identity_and_id() {
        let (id, payload) = fixture("login_profile");
        let mut bridge = Bridge::default();
        let out = bridge.clientbound(State::Login, id, payload).unwrap();
        let login::Clientbound::LoginFinished(p) =
            login::Clientbound::decode(out.packets[0].0, &out.packets[0].1).unwrap()
        else {
            panic!()
        };
        assert_eq!(p.profile.uuid, 0x112233445566778899aabbccddeeff00);
        assert_eq!(p.profile.name, "ProtocolProbe");
        let (id, payload) = fixture("teleport_absolute");
        let out = bridge.clientbound(State::Play, id, payload).unwrap();
        let play::Clientbound::PlayerPosition(p) =
            play::Clientbound::decode(out.packets[0].0, &out.packets[0].1).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (p.teleport, p.position, p.yaw, p.pitch),
            (37, [1.25, 64.5, -2.75], 90.0, -15.0)
        );
    }
    #[test]
    fn actual_old_potion_and_netherite_tags_roundtrip_without_reconstructing_nbt() {
        let _names = registry_ids::legacy_registries(names());
        let _cache =
            registry_ids::legacy_items(Arc::new(std::sync::Mutex::new(Default::default())));
        for name in ["set_slot_potion", "set_slot_netherite_pickaxe"] {
            let (_, payload) = fixture(name);
            let bytes = &payload[3..];
            let mut r = Reader::new(bytes);
            let value = read_slot(&mut r).unwrap();
            finish(&r).unwrap();
            let mut w = Writer::new();
            write_slot(&value, &mut w).unwrap();
            let mut original = Reader::new(bytes);
            let mut encoded = Reader::new(&w.data);
            assert_eq!(
                boolean(&mut original).unwrap(),
                boolean(&mut encoded).unwrap()
            );
            assert_eq!(original.varint().unwrap(), encoded.varint().unwrap());
            assert_eq!(original.i8().unwrap(), encoded.i8().unwrap());
            // Compound ordering is not part of NBT identity; retain exact slot framing.
            assert_eq!(
                &bytes[..original.position()],
                &w.data[..encoded.position()],
                "{name} slot framing"
            );
            assert_eq!(
                original.named_nbt().unwrap(),
                encoded.named_nbt().unwrap(),
                "{name} full source tag"
            );
            finish(&original).unwrap();
            finish(&encoded).unwrap();
            for end in 0..bytes.len() {
                assert!(
                    read_slot(&mut Reader::new(&bytes[..end])).is_err(),
                    "{name}: {end}"
                );
            }
        }
    }
    #[test]
    fn actual_health_metadata_uses_measured_living_indices() {
        let (id, payload) = fixture("metadata_health_flags");
        let mut bridge = Bridge::default();
        bridge.entities.register_player(123);
        let out = bridge.clientbound(State::Play, id, payload).unwrap();
        let play::Clientbound::SetEntityData(p) =
            play::Clientbound::decode(out.packets[0].0, &out.packets[0].1).unwrap()
        else {
            panic!()
        };
        assert!(p.values.contains(&(
            9,
            crate::entity_data::serializer::FLOAT,
            crate::entity_data::Value::Float(18.5)
        )));
        assert!(p.values.contains(&(
            0,
            crate::entity_data::serializer::BYTE,
            crate::entity_data::Value::Byte(8)
        )));
    }
    #[test]
    fn client_information_waits_for_join_and_virtual_finish_delivery() {
        let mut bridge = Bridge {
            profile: true,
            acknowledged: true,
            ..Default::default()
        };
        let body = cfg::Serverbound::ClientInformation(cfg::ClientInformation::new(8)).encode();
        let (id, payload) = packet(body);
        let result = bridge
            .serverbound(State::Configuration, id, &payload)
            .unwrap();
        assert!(result.packet.is_none() && result.extra.is_empty());
        assert_eq!(bridge.deferred.len(), 1);
        assert!(
            bridge
                .serverbound(
                    State::Configuration,
                    ids::configuration::serverbound::FINISH_CONFIGURATION,
                    &[]
                )
                .is_err()
        );
        assert_eq!(bridge.deferred.len(), 1);
        bridge.finish_queued = true;
        bridge.finish_delivered();
        let result = bridge
            .serverbound(
                State::Configuration,
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[],
            )
            .unwrap();
        assert_eq!(result.phase, Some(State::Play));
        assert_eq!(
            result.extra,
            vec![vec![5, 5, b'e', b'n', b'_', b'u', b's', 8, 0, 1, 127, 1]]
        );
        assert!(bridge.deferred.is_empty());
    }
    #[test]
    fn public_session_orders_configuration_and_quiet_inventory_corrections() {
        use horizon_world::datafix::dynamic as d;
        let session = super::super::Session::new(ServerVersion::V1_16_5);
        let mut handshake = Writer::new();
        handshake.varint(0);
        handshake.varint(777);
        handshake.string("localhost");
        handshake.u16(25565);
        handshake.varint(2);
        session.serverbound(&handshake.data).unwrap();
        session.take_outgoing();
        let (id, profile) = fixture("login_profile");
        session.clientbound(id, profile).unwrap();
        session
            .serverbound(&login::Serverbound::LoginAcknowledged(crate::packets::Empty).encode())
            .unwrap();
        session
            .serverbound(
                &cfg::Serverbound::ClientInformation(cfg::ClientInformation::new(8)).encode(),
            )
            .unwrap();
        assert!(session.take_outgoing().is_empty());
        let dimension =
            Tag::compound([("natural", Tag::Byte(1)), ("logical_height", Tag::Int(256))]);
        let registry_entry = |name: &str, element: Tag| {
            Tag::compound([
                ("name", d::string(name)),
                ("id", Tag::Int(0)),
                ("element", element),
            ])
        };
        let registry = Tag::compound([
            (
                "minecraft:dimension_type",
                Tag::compound([(
                    "value",
                    Tag::List(vec![registry_entry(
                        "minecraft:overworld",
                        dimension.clone(),
                    )]),
                )]),
            ),
            (
                "minecraft:worldgen/biome",
                Tag::compound([(
                    "value",
                    Tag::List(vec![registry_entry(
                        "minecraft:plains",
                        Tag::compound([
                            ("temperature", Tag::Float(0.8f32.to_bits())),
                            ("downfall", Tag::Float(0.4f32.to_bits())),
                        ]),
                    )]),
                )]),
            ),
        ]);
        let mut join = Writer::new();
        join.i32(7);
        join.bool(false);
        join.u8(1);
        join.i8(-1);
        join.varint(1);
        join.string("minecraft:overworld");
        join.named_nbt(&registry);
        join.named_nbt(&dimension);
        join.string("minecraft:overworld");
        join.i64(0);
        join.varint(20);
        join.varint(8);
        join.bool(false);
        join.bool(true);
        join.bool(false);
        join.bool(true);
        let first = session.clientbound(36, join.data).unwrap();
        assert!(session.take_outgoing().is_empty());
        assert!(
            session
                .serverbound(&cfg::Serverbound::FinishConfiguration(crate::packets::Empty).encode())
                .is_err()
        );
        let mut enchantment_entries = None;
        let mut next = first;
        while let Some((id, payload)) = next {
            if id == ids::configuration::clientbound::REGISTRY_DATA
                && let cfg::Clientbound::RegistryData(registry) =
                    cfg::Clientbound::decode(id, &payload).unwrap()
                && registry.registry == "minecraft:enchantment"
            {
                enchantment_entries = Some(registry.entries);
            }
            next = session.next_incoming();
        }
        session
            .serverbound(&cfg::Serverbound::FinishConfiguration(crate::packets::Empty).encode())
            .unwrap();
        assert_eq!(
            session.take_outgoing(),
            vec![vec![5, 5, b'e', b'n', b'_', b'u', b's', 8, 0, 1, 127, 1]]
        );
        let enchantment_entries = enchantment_entries.expect("consumer enchantment RegistryData");
        assert_eq!(
            enchantment_entries
                .iter()
                .map(|v| &v.id)
                .collect::<Vec<_>>(),
            names()["minecraft:enchantment"]
                .names
                .iter()
                .collect::<Vec<_>>()
        );
        let (id, mut pickaxe) = fixture("set_slot_netherite_pickaxe");
        pickaxe[0] = 0; // Deliver the actual item bytes to the player's known window.
        let translated = session.clientbound(id, pickaxe).unwrap().unwrap();
        let play::Clientbound::ContainerSetSlot(slot) =
            play::Clientbound::decode(translated.0, &translated.1).unwrap()
        else {
            panic!()
        };
        let stack = slot.item.unwrap();
        let Some(crate::item::Component::Enchantments(levels)) =
            stack.component(crate::item::component::ENCHANTMENTS)
        else {
            panic!("actual netherite fixture lost its enchantments")
        };
        assert_eq!(levels.len(), 1);
        let (id, level) = levels[0];
        assert_eq!(level, 3);
        assert_eq!(
            enchantment_entries[id as usize].id, "minecraft:unbreaking",
            "the translated component must resolve in the consumer's emitted registry"
        );
        let mut items = Writer::new();
        items.u8(0);
        items.i16(46);
        for _ in 0..46 {
            items.bool(false);
        }
        let initial = session.clientbound(19, items.data).unwrap().unwrap();
        let play::Clientbound::ContainerSetContent(initial) =
            play::Clientbound::decode(initial.0, &initial.1).unwrap()
        else {
            panic!()
        };
        let edit = play::Serverbound::ContainerClick(play::ContainerClick {
            container: 0,
            state: initial.state,
            slot: 10,
            button: 0,
            input: 0,
            changed: vec![(
                10,
                Some(crate::item::Hashed {
                    item: i32::MAX,
                    count: 1,
                    added: vec![],
                    removed: vec![],
                }),
            )],
            carried: None,
        })
        .encode();
        assert_eq!(
            session.serverbound_checked(&edit).unwrap().1,
            super::super::SendDisposition::LocallyRejected
        );
        assert!(session.take_outgoing().is_empty());
        let mut update = Writer::new();
        update.u8(0);
        update.i16(36);
        update.bool(false);
        let correction = session.clientbound(21, update.data).unwrap().unwrap();
        assert!(matches!(
            play::Clientbound::decode(correction.0, &correction.1).unwrap(),
            play::Clientbound::ContainerSetContent(_)
        ));
        let authoritative = session.next_incoming().unwrap();
        assert!(matches!(
            play::Clientbound::decode(authoritative.0, &authoritative.1).unwrap(),
            play::Clientbound::ContainerSetSlot(_)
        ));
    }
    #[test]
    fn native_dig_actions_use_the_source_enum_and_direction_changes_are_consumed() {
        let mut bridge = Bridge {
            finished: true,
            ..Default::default()
        };
        for (native, old) in [
            (0, Some(0)),
            (1, None),
            (2, Some(1)),
            (3, Some(2)),
            (4, Some(3)),
            (5, Some(4)),
            (6, Some(5)),
            (7, Some(6)),
        ] {
            let (id, payload) = packet(
                play::Serverbound::PlayerAction(play::PlayerAction {
                    action: native,
                    pos: [1, 64, -2],
                    direction: 3,
                    sequence: 91,
                })
                .encode(),
            );
            let result = bridge.serverbound(State::Play, id, &payload).unwrap();
            if let Some(old) = old {
                let mut r = Reader::new(result.packet.as_ref().unwrap());
                assert_eq!(r.varint().unwrap(), 27);
                assert_eq!(r.varint().unwrap(), old);
                assert_eq!(crate::buf::Pos::read(&mut r).unwrap(), [1, 64, -2]);
                assert_eq!(r.u8().unwrap(), 3);
                finish(&r).unwrap();
            } else {
                assert!(result.packet.is_none());
            }
            assert!(bridge.take_predictions().is_empty());
        }
    }
    #[test]
    fn teleport_confirmation_has_only_the_old_id_and_does_not_leak_native_coordinates() {
        let mut bridge = Bridge {
            finished: true,
            ..Default::default()
        };
        let (id, payload) = packet(
            play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                id: 37,
                x: 1.25,
                y: 64.5,
                z: -2.75,
                yaw: 90.0,
                pitch: -15.0,
            })
            .encode(),
        );
        assert_eq!(
            bridge
                .serverbound(State::Play, id, &payload)
                .unwrap()
                .packet,
            Some(vec![0, 37])
        );
    }
}

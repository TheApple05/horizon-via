//! Protocol340 connection lifecycle and native777 packet emission.
use super::{boolean, entities, facts, finish, inventory, terrain};
use crate::adapter::v1_8_9::{self as old, bootstrap, common, presentation, world};
use crate::{
    Error, Result, State,
    buf::{Codec, Reader, Writer},
    ids,
    item::Stack,
    packets::{configuration as cfg, login, play},
};
use horizon_world::{
    datafix::{self, dynamic as d, schema::Ty},
    nbt::Tag,
};
use std::collections::HashMap;

pub use old::bridge::{Incoming, Outgoing};

#[derive(Default)]
pub struct Bridge {
    online_mode: bool,
    profile_finished: bool,
    acknowledged: bool,
    finish_queued: bool,
    delivered: bool,
    finished: bool,
    rejected: bool,
    deferred: Vec<Vec<u8>>,
    join: Option<old::Join>,
    common: Option<common::State>,
    presentation: presentation::State,
    inventory: inventory::State,
    terrain: world::World,
    entities: entities::State,
    predictions: Vec<(i32, Vec<u8>)>,
    teleports: std::collections::BTreeSet<i32>,
    pending_outgoing: Vec<Vec<u8>>,
    skulls: HashMap<[i32; 3], (u8, u8)>,
    pots: HashMap<[i32; 3], i32>,
    beds: HashMap<[i32; 3], u8>,
    resource_pack: old::LegacyPackSlot,
    recipes: super::recipes::State,
}

fn wire(id: i32, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    w.bytes(body);
    w.data
}
fn emit(packet: play::Clientbound) -> (i32, Vec<u8>) {
    bootstrap::packet(packet.encode())
}

impl Bridge {
    pub fn online_mode(&self) -> bool {
        self.online_mode
    }
    pub fn finish_delivered(&mut self) {
        if self.finish_queued {
            self.delivered = true;
        }
    }
    pub fn take_predictions(&mut self) -> Vec<(i32, Vec<u8>)> {
        std::mem::take(&mut self.predictions)
    }
    pub fn rejected_locally(&self) -> bool {
        self.rejected
    }
    pub fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        self.entities.take_projectile_acceleration(id)
    }

    pub fn clientbound(&mut self, state: State, id: i32, payload: Vec<u8>) -> Result<Incoming> {
        let mut out = Incoming::default();
        match state {
            State::Status => out.packets.push((id, payload)),
            State::Login => match id {
                0 | 3 => out.packets.push((id, payload)),
                1 => {
                    let mut r = Reader::new(&payload);
                    let p = login::Hello {
                        server_id: r.string(20)?,
                        public_key: r.byte_array(1 << 20)?,
                        challenge: r.byte_array(1 << 20)?,
                        should_authenticate: true,
                    };
                    finish(&r)?;
                    self.online_mode = true;
                    out.packets
                        .push(bootstrap::packet(login::Clientbound::Hello(p).encode()));
                }
                2 => {
                    if self.profile_finished {
                        return Err(Error::new("duplicate protocol340 login profile"));
                    }
                    let profile = old::login_finished(&payload)?;
                    let profile = login::GameProfile::read(&mut Reader::new(&profile))?;
                    out.packets.push(bootstrap::packet(
                        login::Clientbound::LoginFinished(login::LoginFinished {
                            profile,
                            session: 0,
                        })
                        .encode(),
                    ));
                    self.profile_finished = true;
                    out.phase = Some(State::Configuration);
                }
                _ => return Err(Error::new(format!("unknown protocol340 login packet {id}"))),
            },
            State::Configuration => {
                if !self.profile_finished || self.finish_queued {
                    return Err(Error::new("unexpected protocol340 configuration state"));
                }
                if id == 0x1f {
                    let mut r = Reader::new(&payload);
                    let p = cfg::KeepAlive { id: r.i64()? };
                    finish(&r)?;
                    out.packets
                        .push(bootstrap::packet(cfg::Clientbound::KeepAlive(p).encode()));
                    return Ok(out);
                }
                if id == 0x1a {
                    let mut r = Reader::new(&payload);
                    let reason = crate::adapter::read_legacy_component(&mut r)?;
                    finish(&r)?;
                    out.packets.push(bootstrap::packet(
                        cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode(),
                    ));
                    return Ok(out);
                }
                if id != 0x23 {
                    return Err(Error::new("protocol340 play packet precedes JoinGame"));
                }
                let join = join(&payload)?;
                for mut registry in bootstrap::registries() {
                    if registry.registry == "minecraft:worldgen/biome" {
                        registry.entries.push(cfg::RegistryEntry {
                            id: "horizon_via:legacy_biome_127".into(),
                            data: Some(Tag::compound([
                                ("temperature", Tag::Float(0.5f32.to_bits())),
                                ("downfall", Tag::Float(0.5f32.to_bits())),
                                ("has_precipitation", Tag::Byte(0)),
                                (
                                    "effects",
                                    Tag::compound([
                                        ("fog_color", Tag::Int(12638463)),
                                        ("water_color", Tag::Int(0x3f76e4)),
                                        ("water_fog_color", Tag::Int(0x050533)),
                                        ("sky_color", Tag::Int(0x78a7ff)),
                                    ]),
                                ),
                            ])),
                        });
                    }
                    out.packets.push(bootstrap::packet(
                        cfg::Clientbound::RegistryData(registry).encode(),
                    ));
                }
                out.packets.push(bootstrap::packet(
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
                    .push(emit(play::Clientbound::Login(bootstrap::login(
                        &join,
                        self.online_mode,
                    )?)));
                out.packets.push(emit(bootstrap::commands()));
                out.packets
                    .push(emit(play::Clientbound::GameEvent(play::GameEvent {
                        event: 13,
                        param: 0.0,
                    })));
                self.inventory.set_creative(join.mode == 1);
                self.common = Some(common::State::new(join.entity));
                self.entities.local(join.entity);
                self.join = Some(join);
                self.finish_queued = true;
                out.phase = Some(State::Play);
            }
            State::Play => {
                out.packets
                    .extend(self.play(id, &payload)?.into_iter().map(emit));
                out.outgoing.append(&mut self.pending_outgoing);
            }
            State::Handshake => {
                return Err(Error::new("server packet before protocol340 handshake"));
            }
        }
        Ok(out)
    }

    fn play(&mut self, id: i32, payload: &[u8]) -> Result<Vec<play::Clientbound>> {
        if let Some(out) = self.recipes.clientbound(id, payload)? {
            return Ok(out);
        }
        if let Some(mut out) = super::worldfx::clientbound(id, payload)? {
            let mut affected = std::collections::BTreeSet::new();
            for (pos, raw) in out.block_changes {
                if !(0..256).contains(&pos[1]) {
                    continue;
                }
                self.terrain.set_raw(pos, raw)?;
                if raw >> 4 != 144 {
                    self.skulls.remove(&pos);
                }
                if raw >> 4 != 26 {
                    self.beds.remove(&pos);
                }
                if raw >> 4 != 140 {
                    self.pots.remove(&pos);
                }
                for delta in [
                    [0, 0, 0],
                    [1, 0, 0],
                    [-1, 0, 0],
                    [0, 1, 0],
                    [0, -1, 0],
                    [0, 0, 1],
                    [0, 0, -1],
                ] {
                    let p = std::array::from_fn(|k| pos[k] + delta[k]);
                    if (0..256).contains(&p[1]) {
                        affected.insert(p);
                    }
                }
            }
            out.packets.extend(self.context_updates(affected)?);
            return Ok(out.packets);
        }
        if let Some(old_id) = inventory_id(id) {
            let mut state = std::mem::take(&mut self.inventory);
            let result = state.clientbound(old_id, payload, &Context(self));
            for update in state.take_mount_updates() {
                self.entities.remember_mount(update)?;
            }
            for p in state.take_outgoing() {
                self.pending_outgoing
                    .push(wire(inventory_out_id(p.id)?, &p.payload));
            }
            self.inventory = state;
            if let Some(out) = result? {
                return Ok(out);
            }
        }
        if let Some(old_id) = common_id(id) {
            let mut state = self
                .common
                .take()
                .ok_or_else(|| Error::new("protocol340 common packet before join"))?;
            let out = state.clientbound(old_id, payload, &Context(self));
            self.common = Some(state);
            if let Some(mut out) = out? {
                out.extend(self.retain_blocks(id, payload)?);
                for p in &out {
                    if let play::Clientbound::GameEvent(p) = p
                        && p.event == 3
                        && (0.0..=3.0).contains(&p.param)
                    {
                        self.inventory.set_creative(p.param as i32 == 1);
                    }
                }
                return Ok(out);
            }
        }
        if let Some(old_id) = presentation_id(id) {
            let mut state = std::mem::take(&mut self.presentation);
            let out = state.clientbound(old_id, payload, &Context(self));
            for p in state.take_outgoing() {
                self.pending_outgoing.push(completion_wire(&p.payload)?);
            }
            self.presentation = state;
            if let Some(out) = out? {
                return Ok(out);
            }
        }
        let mut entity = std::mem::take(&mut self.entities);
        let result = entity.clientbound(id, payload, &Context(self));
        self.entities = entity;
        if let Some(out) = result? {
            return Ok(out);
        }
        let mut r = Reader::new(payload);
        let out = match id {
            0x24 => vec![play::Clientbound::MapItemData(map(&mut r)?)],
            0x17 => {
                let raw = i16::try_from(r.varint()?)
                    .map_err(|_| Error::new("invalid protocol340 cooldown item"))?;
                let duration = r.varint()?;
                if duration < 0 {
                    return Err(Error::new("negative protocol340 cooldown"));
                }
                let tag = crate::item::item47::Raw47Slot {
                    id: raw,
                    count: 1,
                    damage: 0,
                    tag: Tag::End,
                }
                .upgrade_at(facts::old_item_name, 1343)?;
                let group = d::as_str(tag.get("id"))
                    .ok_or_else(|| Error::new("protocol340 cooldown upgrade lost item id"))?;
                vec![play::Clientbound::Cooldown(play::Cooldown {
                    group,
                    duration,
                })]
            }
            0x29 => {
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                if position.iter().any(|v| !v.is_finite()) || !yaw.is_finite() || !pitch.is_finite()
                {
                    return Err(Error::new("nonfinite protocol340 vehicle teleport"));
                }
                vec![play::Clientbound::MoveVehicle(play::MoveVehicle {
                    position,
                    yaw,
                    pitch,
                })]
            }
            0x2d => match r.varint()? {
                0 => vec![play::Clientbound::Unknown(
                    ids::play::clientbound::PLAYER_COMBAT_ENTER,
                    vec![],
                )],
                1 => {
                    let duration = r.varint()?;
                    r.i32()?;
                    if duration < 0 {
                        return Err(Error::new("negative protocol340 combat duration"));
                    }
                    let mut w = Writer::new();
                    w.varint(duration);
                    vec![play::Clientbound::Unknown(
                        ids::play::clientbound::PLAYER_COMBAT_END,
                        w.data,
                    )]
                }
                2 => {
                    let id = r.varint()?;
                    r.i32()?;
                    let message = crate::adapter::read_legacy_component(&mut r)?;
                    vec![play::Clientbound::PlayerCombatKill(
                        play::PlayerCombatKill { id, message },
                    )]
                }
                _ => return Err(Error::new("unknown protocol340 combat action")),
            },
            0x37 => {
                let tab = if boolean(&mut r)? {
                    Some(r.string(32767)?)
                } else {
                    None
                };
                vec![play::Clientbound::SelectAdvancementsTab(
                    play::SelectAdvancementsTab { tab },
                )]
            }
            0x44 => vec![play::Clientbound::SetPlayerTeam(team(&mut r)?)],
            0x1f => vec![play::Clientbound::KeepAlive(cfg::KeepAlive {
                id: r.i64()?,
            })],
            0x20 => {
                let chunk = terrain::read(
                    payload,
                    self.join.as_ref().is_some_and(|j| j.dimension == 0),
                )?;
                let full = chunk.update.full;
                let x = chunk.update.x;
                let z = chunk.update.z;
                if full {
                    self.skulls.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                    self.pots.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                    self.beds.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                }
                for tag in &chunk.block_entities {
                    self.contextual_tile(tag)?;
                }
                let snapshot = self.terrain.snapshot(std::slice::from_ref(&chunk.update))?;
                let skulls = &self.skulls;
                let pots = &self.pots;
                let beds = &self.beds;
                let mut out = self.terrain.apply(
                    vec![chunk.update],
                    |x, z, y, i, raw| {
                        mapped(
                            raw,
                            [
                                x * 16 + (i & 15) as i32,
                                (y * 16 + (i >> 8)) as i32,
                                z * 16 + ((i >> 4) & 15) as i32,
                            ],
                            skulls,
                            pots,
                            beds,
                            |p| {
                                snapshot
                                    .get(&(p[0] >> 4, p[2] >> 4))
                                    .and_then(|c| world::raw_in(c, p))
                            },
                        )
                    },
                    biome,
                    16,
                    6,
                )?;
                for tag in chunk.block_entities {
                    out.push(self.tile(tag)?);
                }
                out.extend(self.refresh_columns(x, z)?);
                return Ok(out);
            }
            0x1d => {
                let x = r.i32()?;
                let z = r.i32()?;
                finish(&r)?;
                self.skulls.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                self.pots.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                self.beds.retain(|p, _| p[0] >> 4 != x || p[2] >> 4 != z);
                let mut out = self.terrain.apply(
                    vec![old::terrain::Update {
                        x,
                        z,
                        full: false,
                        unload: true,
                        has_sky: false,
                        mask: 0,
                        sections: vec![None; 16],
                        biomes: None,
                    }],
                    |_, _, _, _, _| unreachable!(),
                    biome,
                    16,
                    6,
                )?;
                out.extend(self.refresh_columns(x, z)?);
                return Ok(out);
            }
            0x2f => {
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let flags = r.u8()?;
                let teleport = r.varint()?;
                if position.iter().any(|v| !v.is_finite())
                    || !yaw.is_finite()
                    || !pitch.is_finite()
                    || flags & !31 != 0
                    || teleport < 0
                {
                    return Err(Error::new("invalid protocol340 teleport"));
                }
                if self.teleports.len() >= 128 || !self.teleports.insert(teleport) {
                    return Err(Error::new("duplicate/excessive protocol340 teleport"));
                }
                vec![play::Clientbound::PlayerPosition(play::PlayerPosition {
                    teleport,
                    position,
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives: i32::from(flags) | ((i32::from(flags) & 7) << 5),
                })]
            }
            0x35 => {
                let dimension = r.i32()?;
                let difficulty = r.u8()?;
                let mode = r.u8()?;
                let level_type = r.string(16)?;
                finish(&r)?;
                bootstrap::dimension(dimension)?;
                if difficulty > 3 || mode > 3 {
                    return Err(Error::new("invalid protocol340 respawn"));
                }
                let join = self
                    .join
                    .as_mut()
                    .ok_or_else(|| Error::new("protocol340 respawn before join"))?;
                join.dimension = dimension as i8;
                join.difficulty = difficulty;
                join.mode = mode;
                join.level_type = level_type;
                let spawn = bootstrap::spawn(join)?;
                let player = join.entity;
                self.terrain.reset();
                self.skulls.clear();
                self.pots.clear();
                self.beds.clear();
                self.entities.clear();
                self.entities.local(player);
                self.teleports.clear();
                self.inventory.on_respawn();
                self.inventory.set_creative(mode == 1);
                return Ok(vec![
                    play::Clientbound::Respawn(play::Respawn { spawn, keep: 0 }),
                    play::Clientbound::ChangeDifficulty(play::ChangeDifficulty {
                        difficulty: i32::from(difficulty),
                        locked: false,
                    }),
                ]);
            }
            0x09 => {
                let pos = common::position_up(r.i64()?);
                let _action = r.u8()?;
                let mut tag = r.named_nbt()?;
                finish(&r)?;
                d::set(&mut tag, "x", Tag::Int(pos[0]));
                d::set(&mut tag, "y", Tag::Int(pos[1]));
                d::set(&mut tag, "z", Tag::Int(pos[2]));
                self.contextual_tile(&tag)?;
                let mut out = Vec::new();
                if let Some(raw) = self.terrain.raw_state(pos)
                    && matches!(raw >> 4, 26 | 140 | 144)
                {
                    out.push(play::Clientbound::BlockUpdate(play::BlockUpdate {
                        pos,
                        state: mapped(raw, pos, &self.skulls, &self.pots, &self.beds, |p| {
                            self.terrain.raw_state(p)
                        })?
                        .id,
                    }));
                }
                out.push(self.tile(tag)?);
                return Ok(out);
            }
            0x48 => {
                let action = r.varint()?;
                if action == 2 {
                    let content = crate::adapter::read_legacy_component(&mut r)?;
                    vec![play::Clientbound::SystemChat(play::SystemChat {
                        content,
                        overlay: true,
                    })]
                } else {
                    let target = match action {
                        0 | 1 => action,
                        3 => 2,
                        4 => 3,
                        5 => 4,
                        _ => return Err(Error::new("unknown protocol340 title action")),
                    };
                    let mut w = Writer::new();
                    w.varint(target);
                    w.bytes(r.rest());
                    return self
                        .common
                        .as_mut()
                        .ok_or_else(|| Error::new("title before join"))?
                        .clientbound(0x45, &w.data, &TextContext)?
                        .ok_or_else(|| Error::new("unhandled title"));
                }
            }
            0x18 => {
                let channel = r.string(20)?;
                match channel.as_str() {
                    "MC|Brand" => {
                        let data = r.rest().to_vec();
                        r.bytes(r.remaining())?;
                        vec![play::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: "minecraft:brand".into(),
                            data,
                        })]
                    }
                    "MC|BOpen" => {
                        let hand = r.varint()?;
                        if !(0..=1).contains(&hand) {
                            return Err(Error::new("invalid protocol340 book hand"));
                        }
                        vec![play::Clientbound::OpenBook(play::OpenBook { hand })]
                    }
                    "MC|TrList" => {
                        let mut inv = std::mem::take(&mut self.inventory);
                        let result = inv.clientbound(0x3f, payload, &Context(self));
                        self.inventory = inv;
                        return result?.ok_or_else(|| Error::new("unhandled merchant message"));
                    }
                    _ => {
                        r.bytes(r.remaining())?;
                        vec![]
                    }
                }
            }
            0x34 => {
                let url = r.string(32767)?;
                let hash = r.string(40)?;
                finish(&r)?;
                match self.resource_pack.request(url, hash, false) {
                    Ok(request) => vec![play::Clientbound::ResourcePackPush(request)],
                    Err(status) => {
                        self.pending_outgoing.push(wire(0x18, &status));
                        vec![]
                    }
                }
            }
            0x33 => {
                let id = r.varint()?;
                let effect = i32::from(r.u8()?);
                if !(1..=27).contains(&effect) {
                    return Err(Error::new("invalid protocol340 effect"));
                }
                vec![play::Clientbound::RemoveMobEffect(play::RemoveMobEffect {
                    id,
                    effect: effect - 1,
                })]
            }
            0x4f => {
                let id = r.varint()?;
                let effect = i32::from(r.u8()?);
                let amplifier = i32::from(r.u8()?);
                let duration = r.varint()?;
                let flags = r.u8()?;
                if !(1..=27).contains(&effect) || duration < 0 || flags & !3 != 0 {
                    return Err(Error::new("invalid protocol340 effect"));
                }
                vec![play::Clientbound::UpdateMobEffect(play::UpdateMobEffect {
                    id,
                    effect: effect - 1,
                    amplifier,
                    duration,
                    flags: (flags | 4) as i8,
                })]
            }
            _ => {
                return Err(Error::new(format!(
                    "unimplemented protocol340 play packet {id:#x}"
                )));
            }
        };
        finish(&r)?;
        Ok(out)
    }

    pub fn serverbound(&mut self, state: State, id: i32, payload: &[u8]) -> Result<Outgoing> {
        self.rejected = false;
        let mut out = Outgoing::default();
        match state {
            State::Status => out.packet = Some(wire(id, payload)),
            State::Login => match id {
                ids::login::serverbound::HELLO => {
                    let mut r = Reader::new(payload);
                    let name = r.string(16)?;
                    r.uuid()?;
                    finish(&r)?;
                    let mut w = Writer::new();
                    w.string(&name);
                    out.packet = Some(wire(0, &w.data));
                }
                ids::login::serverbound::KEY => out.packet = Some(wire(1, payload)),
                ids::login::serverbound::LOGIN_ACKNOWLEDGED => {
                    if !payload.is_empty() || !self.profile_finished || self.acknowledged {
                        return Err(Error::new("unexpected protocol340 login acknowledgement"));
                    }
                    self.acknowledged = true;
                    out.phase = Some(State::Configuration);
                }
                _ => {}
            },
            State::Configuration => {
                if id == ids::configuration::serverbound::FINISH_CONFIGURATION {
                    if !payload.is_empty() || !self.acknowledged || !self.delivered || self.finished
                    {
                        return Err(Error::new("premature protocol340 configuration finish"));
                    }
                    self.finished = true;
                    out.phase = Some(State::Play);
                    out.extra.append(&mut self.deferred);
                    return Ok(out);
                }
                let packet = match id {
                    ids::configuration::serverbound::CLIENT_INFORMATION => Some(settings(payload)?),
                    ids::configuration::serverbound::CUSTOM_PAYLOAD => custom(payload)?,
                    ids::configuration::serverbound::KEEP_ALIVE => {
                        let mut r = Reader::new(payload);
                        r.i64()?;
                        finish(&r)?;
                        out.packet = Some(wire(0x0b, payload));
                        return Ok(out);
                    }
                    _ => None,
                };
                if let Some(p) = packet {
                    if self.deferred.len() >= 128
                        || self.deferred.iter().map(Vec::len).sum::<usize>() + p.len() > 1 << 20
                    {
                        return Err(Error::new("protocol340 deferred packet limit"));
                    }
                    self.deferred.push(p);
                }
            }
            State::Play => {
                if id == ids::play::serverbound::KEEP_ALIVE {
                    let mut r = Reader::new(payload);
                    r.i64()?;
                    finish(&r)?;
                    out.packet = Some(wire(0x0b, payload));
                    return Ok(out);
                }
                if id == ids::play::serverbound::ACCEPT_TELEPORTATION {
                    let mut r = Reader::new(payload);
                    let p = play::AcceptTeleportation::read(&mut r)?;
                    finish(&r)?;
                    if [p.x, p.y, p.z].iter().any(|v| !v.is_finite())
                        || !p.yaw.is_finite()
                        || !p.pitch.is_finite()
                    {
                        return Err(Error::new("invalid protocol340 teleport acknowledgement"));
                    }
                    if !self.teleports.remove(&p.id) {
                        return Err(Error::new("unknown protocol340 teleport acknowledgement"));
                    }
                    let mut w = Writer::new();
                    w.varint(p.id);
                    out.packet = Some(wire(0, &w.data));
                    return Ok(out);
                }
                if id == ids::play::serverbound::CLIENT_INFORMATION {
                    out.packet = Some(settings(payload)?);
                    return Ok(out);
                }
                if id == ids::play::serverbound::CUSTOM_PAYLOAD {
                    out.packet = custom(payload)?;
                    return Ok(out);
                }
                if id == ids::play::serverbound::RESOURCE_PACK {
                    let mut r = Reader::new(payload);
                    let p = cfg::ResourcePackResponse::read(&mut r)?;
                    finish(&r)?;
                    out.packet = self
                        .resource_pack
                        .status(p.id, p.action, false)?
                        .map(|body| wire(0x18, &body));
                    return Ok(out);
                }
                let p = play::Serverbound::decode(id, payload)?;
                let mut inv = std::mem::take(&mut self.inventory);
                let result = inv.serverbound(&p, &Context(self));
                self.predictions
                    .extend(inv.take_incoming().into_iter().map(emit));
                if result.is_err()
                    && let Some(correction) = inv.correction()
                {
                    self.predictions.push(emit(correction));
                }
                self.inventory = inv;
                match result {
                    Ok(Some(packets)) => {
                        for p in packets {
                            let p = wire(inventory_out_id(p.id)?, &p.payload);
                            if out.packet.is_none() {
                                out.packet = Some(p);
                            } else {
                                out.extra.push(p);
                            }
                        }
                        return Ok(out);
                    }
                    Err(e) => {
                        self.reject(&e);
                        return Ok(out);
                    }
                    Ok(None) => {}
                }
                let result = self.write_play(&p);
                match result {
                    Ok(packets) => {
                        let mut p = packets.into_iter();
                        out.packet = p.next();
                        out.extra.extend(p);
                    }
                    Err(e) => self.reject(&e),
                }
            }
            State::Handshake => {
                return Err(Error::new(
                    "protocol340 handshake must pass through Session",
                ));
            }
        }
        Ok(out)
    }

    fn write_play(&mut self, p: &play::Serverbound) -> Result<Vec<Vec<u8>>> {
        if let Some((id, payload)) = self.recipes.serverbound(p)? {
            return Ok(vec![wire(id, &payload)]);
        }
        use play::Serverbound as S;
        let mut w = Writer::new();
        let id = match p {
            S::SeenAdvancements(p) => {
                if !(0..=1).contains(&p.action) {
                    return Err(Error::new("unknown protocol340 advancement tab action"));
                }
                w.varint(p.action);
                if p.action == 0 {
                    w.string(
                        p.tab.as_deref().ok_or_else(|| {
                            Error::new("opened advancement tab has no identifier")
                        })?,
                    );
                }
                0x19
            }
            S::Chat(p) => {
                chat(&p.message, &mut w)?;
                0x02
            }
            S::ChatCommand(p) => {
                chat(&format!("/{}", p.command), &mut w)?;
                0x02
            }
            S::ChatCommandSigned(p) => {
                chat(&format!("/{}", p.command), &mut w)?;
                0x02
            }
            S::MoveVehicle(p) => {
                if p.position.iter().any(|v| !v.is_finite())
                    || !p.yaw.is_finite()
                    || !p.pitch.is_finite()
                {
                    return Err(Error::new("nonfinite protocol340 vehicle movement"));
                }
                for v in p.position {
                    w.f64(v);
                }
                w.f32(p.yaw);
                w.f32(p.pitch);
                0x10
            }
            S::PaddleBoat(p) => {
                w.bool(p.left);
                w.bool(p.right);
                0x11
            }
            S::Punch(_) => {
                w.varint(0);
                0x1d
            }
            S::UseItem(p) => {
                hand(p.hand)?;
                w.varint(p.hand);
                self.ack(p.sequence);
                0x20
            }
            S::UseItemOn(p) => {
                hand(p.hand)?;
                if !(0..=5).contains(&p.face)
                    || p.cursor
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(Error::new("invalid protocol340 block interaction"));
                }
                w.i64(position(p.pos)?);
                w.varint(p.face);
                w.varint(p.hand);
                for v in p.cursor {
                    w.f32(v);
                }
                self.ack(p.sequence);
                0x1f
            }
            S::Attack(p) => {
                if p.id < 0 {
                    return Err(Error::new("negative interaction entity"));
                }
                w.varint(p.id);
                w.varint(1);
                0x0a
            }
            S::Interact(p) => {
                hand(p.hand)?;
                if p.id < 0 || p.location.iter().any(|v| !v.is_finite()) {
                    return Err(Error::new("invalid protocol340 entity interaction"));
                }
                w.varint(p.id);
                if self.entities.kind(p.id) == Some(5) {
                    w.varint(2);
                    for v in p.location {
                        w.f32(v as f32);
                    }
                } else {
                    w.varint(0);
                }
                w.varint(p.hand);
                0x0a
            }
            S::PlayerAction(p) => {
                if p.direction > 5 {
                    return Err(Error::new("invalid protocol340 dig face"));
                }
                let action = match p.action {
                    0 => 0,
                    1 => {
                        self.ack(p.sequence);
                        return Ok(vec![]);
                    }
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 4,
                    6 => 5,
                    7 => 6,
                    _ => return Err(Error::new("unsupported protocol340 player action")),
                };
                w.varint(action);
                w.i64(position(p.pos)?);
                w.u8(p.direction);
                self.ack(p.sequence);
                0x14
            }
            S::SignUpdate(p) => {
                if !p.front {
                    return Err(Error::new("protocol340 has no sign back side"));
                }
                w.i64(position(p.pos)?);
                for line in &p.lines {
                    if line.encode_utf16().count() > 384 {
                        return Err(Error::new("protocol340 sign text too long"));
                    }
                    w.string(line);
                }
                0x1c
            }
            S::PlayerCommand(p) => {
                if self.join.as_ref().is_none_or(|j| j.entity != p.id)
                    || !(0..=100).contains(&p.data)
                {
                    return Err(Error::new("invalid protocol340 player command"));
                }
                let action = match p.action {
                    0 => 2,
                    1 => 3,
                    2 => 4,
                    3 => 5,
                    4 => 6,
                    5 => 7,
                    6 => 8,
                    _ => return Err(Error::new("unsupported protocol340 player command")),
                };
                w.varint(p.id);
                w.varint(action);
                w.varint(p.data);
                0x15
            }
            S::Unknown(id, _) if [ids::play::serverbound::CHUNK_BATCH_RECEIVED].contains(id) => {
                return Ok(vec![]);
            }
            _ => {
                let mut presentation = std::mem::take(&mut self.presentation);
                let out = presentation.serverbound(p, &Context(self));
                self.presentation = presentation;
                if let Some(out) = out? {
                    return out
                        .into_iter()
                        .map(|p| completion_wire(&p.payload))
                        .collect();
                }
                let mut common = self
                    .common
                    .take()
                    .ok_or_else(|| Error::new("protocol340 action before join"))?;
                let out = common.serverbound(p, &Context(self));
                self.common = Some(common);
                if let Some(out) = out? {
                    return out
                        .into_iter()
                        .map(|p| {
                            let id = match p.id {
                                0x01 => 0x02,
                                0x03 => 0x0c,
                                0x04 => 0x0d,
                                0x05 => 0x0f,
                                0x06 => 0x0e,
                                0x09 => 0x1a,
                                0x0b => 0x15,
                                0x0c => 0x16,
                                0x13 => 0x13,
                                0x16 => 0x03,
                                0x18 => 0x1e,
                                _ => {
                                    return Err(Error::new(
                                        "unmapped protocol340 common outgoing packet",
                                    ));
                                }
                            };
                            Ok(wire(id, &p.payload))
                        })
                        .collect();
                }
                return Err(Error::new(
                    "current action has no protocol340 representation",
                ));
            }
        };
        Ok(vec![wire(id, &w.data)])
    }

    fn ack(&mut self, sequence: i32) {
        self.predictions
            .push(emit(play::Clientbound::BlockChangedAck(
                play::BlockChangedAck { sequence },
            )));
    }
    fn retain_blocks(&mut self, id: i32, payload: &[u8]) -> Result<Vec<play::Clientbound>> {
        let mut r = Reader::new(payload);
        let mut changes = Vec::new();
        match id {
            0x0b => {
                let pos = common::position_up(r.i64()?);
                let raw = u16::try_from(r.varint()?)
                    .map_err(|_| Error::new("invalid protocol340 block state"))?;
                changes.push((pos, raw));
            }
            0x10 => {
                let x = r.i32()?;
                let z = r.i32()?;
                if !(-2_097_152..2_097_152).contains(&x) || !(-2_097_152..2_097_152).contains(&z) {
                    return Err(Error::new("protocol340 multi-block coordinate overflow"));
                }
                let n = r.count(65536)?;
                for _ in 0..n {
                    let p = r.u16()?;
                    let raw = u16::try_from(r.varint()?)
                        .map_err(|_| Error::new("invalid protocol340 block state"))?;
                    changes.push((
                        [
                            x * 16 + i32::from(p >> 12),
                            i32::from(p & 255),
                            z * 16 + i32::from((p >> 8) & 15),
                        ],
                        raw,
                    ));
                }
            }
            _ => return Ok(vec![]),
        }
        finish(&r)?;
        let mut affected = std::collections::BTreeSet::new();
        for (pos, raw) in changes {
            if !(0..256).contains(&pos[1]) {
                continue;
            }
            self.terrain.set_raw(pos, raw)?;
            for delta in [
                [0, 0, 0],
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ] {
                let p = std::array::from_fn(|k| pos[k] + delta[k]);
                if (0..256).contains(&p[1]) {
                    affected.insert(p);
                }
            }
            if raw >> 4 != 144 {
                self.skulls.remove(&pos);
            }
            if raw >> 4 != 26 {
                self.beds.remove(&pos);
            }
            if raw >> 4 != 140 {
                self.pots.remove(&pos);
            }
        }
        self.context_updates(affected)
    }
    fn context_updates(
        &self,
        positions: impl IntoIterator<Item = [i32; 3]>,
    ) -> Result<Vec<play::Clientbound>> {
        let mut out = Vec::new();
        for pos in positions {
            if let Some(raw) = self.terrain.raw_state(pos)
                && super::neighbors::needs_context(raw)
            {
                out.push(play::Clientbound::BlockUpdate(play::BlockUpdate {
                    pos,
                    state: mapped(raw, pos, &self.skulls, &self.pots, &self.beds, |p| {
                        self.terrain.raw_state(p)
                    })?
                    .id,
                }));
            }
        }
        Ok(out)
    }
    fn refresh_columns(&self, x: i32, z: i32) -> Result<Vec<play::Clientbound>> {
        let mut positions = std::collections::BTreeSet::new();
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if self.terrain.column(x + dx, z + dz).is_none() {
                continue;
            }
            for y in 0..256 {
                for i in 0..16 {
                    positions.insert(if dx != 0 {
                        [
                            if dx < 0 { x * 16 - 1 } else { (x + 1) * 16 },
                            y,
                            z * 16 + i,
                        ]
                    } else {
                        [
                            x * 16 + i,
                            y,
                            if dz < 0 { z * 16 - 1 } else { (z + 1) * 16 },
                        ]
                    });
                }
            }
        }
        self.context_updates(positions)
    }
    fn reject(&mut self, error: &Error) {
        self.rejected = true;
        crate::item::v1_20_3::reject_numeric47_transaction();
        log::warn!("ViaHorizon: outgoing protocol340 action rejected locally: {error}");
    }
    fn contextual_tile(&mut self, tag: &Tag) -> Result<()> {
        let pos = [
            d::as_int(tag.get("x"), i32::MIN),
            d::as_int(tag.get("y"), i32::MIN),
            d::as_int(tag.get("z"), i32::MIN),
        ];
        if !(0..256).contains(&pos[1]) {
            return Err(Error::new("protocol340 block entity outside world"));
        }
        match d::as_str(tag.get("id")).as_deref() {
            Some("minecraft:skull" | "Skull") => {
                let kind = d::as_int(tag.get("SkullType"), 0);
                if !(0..=5).contains(&kind) {
                    return Err(Error::new("unknown protocol340 skull type"));
                }
                self.skulls
                    .insert(pos, (kind as u8, (d::as_int(tag.get("Rot"), 0) & 15) as u8));
            }
            Some("minecraft:bed" | "Bed") => {
                let color = d::as_int(tag.get("color"), 14);
                if !(0..=15).contains(&color) {
                    return Err(Error::new("invalid protocol340 bed color"));
                }
                self.beds.insert(pos, color as u8);
            }
            Some("minecraft:flower_pot" | "FlowerPot") => {
                let item = match tag.get("Item") {
                    Some(Tag::String(v)) => d::namespaced(&v.to_string_lossy()),
                    v => facts::old_item_name(d::as_int(v, 0) as i16)
                        .ok_or_else(|| Error::new("unknown protocol340 flower pot item"))?,
                };
                let data = d::as_int(tag.get("Data"), 0);
                let state = old::facts::POTS
                    .iter()
                    .find(|&&(name, meta, _)| name == item && meta == data)
                    .map(|v| v.2)
                    .ok_or_else(|| Error::new("unknown protocol340 flower pot contents"))?;
                self.pots.insert(pos, state);
            }
            _ => {}
        }
        if self.skulls.len() + self.pots.len() + self.beds.len() > 65536 {
            return Err(Error::new(
                "protocol340 contextual block entity cache limit",
            ));
        }
        Ok(())
    }
    fn tile(&self, mut tag: Tag) -> Result<play::Clientbound> {
        datafix::update(Ty::BlockEntity, &mut tag, 1343, datafix::CURRENT);
        let name =
            d::as_str(tag.get("id")).ok_or_else(|| Error::new("protocol340 tile has no id"))?;
        let kind = registry("minecraft:block_entity_type", &name)?;
        let pos = [
            d::as_int(tag.get("x"), i32::MIN),
            d::as_int(tag.get("y"), i32::MIN),
            d::as_int(tag.get("z"), i32::MIN),
        ];
        Ok(play::Clientbound::BlockEntityData(play::BlockEntityData {
            pos,
            kind,
            tag,
        }))
    }
}

fn join(payload: &[u8]) -> Result<old::Join> {
    let mut r = Reader::new(payload);
    let entity = r.i32()?;
    let flags = r.u8()?;
    let dimension = r.i32()?;
    let difficulty = r.u8()?;
    let max_players = r.u8()?;
    let level_type = r.string(16)?;
    let reduced_debug = boolean(&mut r)?;
    finish(&r)?;
    bootstrap::dimension(dimension)?;
    if flags & !0x0b != 0 || difficulty > 3 {
        return Err(Error::new("invalid protocol340 JoinGame"));
    }
    Ok(old::Join {
        entity,
        hardcore: flags & 8 != 0,
        mode: flags & 3,
        dimension: dimension as i8,
        difficulty,
        max_players,
        level_type,
        reduced_debug,
    })
}
fn hand(hand: i32) -> Result<()> {
    if (0..=1).contains(&hand) {
        Ok(())
    } else {
        Err(Error::new("invalid protocol340 hand"))
    }
}
fn chat(text: &str, w: &mut Writer) -> Result<()> {
    if text.encode_utf16().count() > 256 || text.chars().any(|c| c < ' ' || c == '\u{7f}') {
        return Err(Error::new("invalid protocol340 chat text"));
    }
    w.string(text);
    Ok(())
}
fn team(r: &mut Reader<'_>) -> Result<play::SetPlayerTeam> {
    let name = r.string(16)?;
    let method = r.i8()?;
    if !(0..=4).contains(&method) {
        return Err(Error::new("unknown protocol340 team method"));
    }
    let parameters = if matches!(method, 0 | 2) {
        let display_name = presentation::legacy_text(&r.string(32)?);
        let prefix = presentation::legacy_text(&r.string(16)?);
        let suffix = presentation::legacy_text(&r.string(16)?);
        let options = r.i8()?;
        if options & !3 != 0 {
            return Err(Error::new("invalid protocol340 team flags"));
        }
        let name_tag_visibility = match r.string(32)?.as_str() {
            "always" => 0,
            "never" => 1,
            "hideForOtherTeams" => 2,
            "hideForOwnTeam" => 3,
            _ => return Err(Error::new("unknown protocol340 team visibility")),
        };
        let collision_rule = match r.string(32)?.as_str() {
            "always" => 0,
            "never" => 1,
            "pushOtherTeams" => 2,
            "pushOwnTeam" => 3,
            _ => return Err(Error::new("unknown protocol340 team collision rule")),
        };
        let color = match r.i8()? {
            -1 => None,
            v @ 0..=15 => Some(i32::from(v)),
            _ => return Err(Error::new("unknown protocol340 team color")),
        };
        Some(play::TeamParameters {
            display_name,
            prefix,
            suffix,
            options,
            name_tag_visibility,
            collision_rule,
            color,
        })
    } else {
        None
    };
    let mut players = vec![];
    if matches!(method, 0 | 3 | 4) {
        let n = r.count(65536)?;
        for _ in 0..n {
            players.push(r.string(40)?);
        }
    }
    Ok(play::SetPlayerTeam {
        name,
        method,
        parameters,
        players,
    })
}
fn position(p: [i32; 3]) -> Result<i64> {
    let [x, y, z] = p;
    if !(-33_554_432..33_554_432).contains(&x)
        || !(-33_554_432..33_554_432).contains(&z)
        || !(-2048..2048).contains(&y)
    {
        return Err(Error::new("protocol340 position outside packed range"));
    }
    Ok(((i64::from(x) & 0x3ffffff) << 38)
        | ((i64::from(y) & 0xfff) << 26)
        | (i64::from(z) & 0x3ffffff))
}
fn settings(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let p = cfg::ClientInformation::read(&mut r)?;
    finish(&r)?;
    if p.language.encode_utf16().count() > 16
        || !(0..=2).contains(&p.chat_visibility)
        || p.model_customisation & 128 != 0
        || !(0..=1).contains(&p.main_hand)
    {
        return Err(Error::new("invalid protocol340 client information"));
    }
    let mut w = Writer::new();
    w.string(&p.language);
    w.i8(p.view_distance);
    w.varint(p.chat_visibility);
    w.bool(p.chat_colors);
    w.u8(p.model_customisation);
    w.varint(p.main_hand);
    Ok(wire(0x04, &w.data))
}
fn custom(payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut r = Reader::new(payload);
    let name = r.string(32767)?;
    let data = r.rest();
    let name = match name.as_str() {
        "minecraft:brand" => "MC|Brand",
        "minecraft:register" => "REGISTER",
        "minecraft:unregister" => "UNREGISTER",
        _ => return Ok(None),
    };
    if data.len() > 32767 {
        return Err(Error::new("protocol340 custom payload too large"));
    }
    let mut w = Writer::new();
    w.string(name);
    w.bytes(data);
    Ok(Some(wire(0x09, &w.data)))
}
fn completion_wire(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let text = r.string(32767)?;
    let mut w = Writer::new();
    w.string(&text);
    w.bool(false);
    w.bytes(r.rest());
    Ok(wire(1, &w.data))
}
fn registry(kind: &str, name: &str) -> Result<i32> {
    super::names()
        .get(kind)
        .and_then(|t| t.id(name))
        .ok_or_else(|| {
            Error::new(format!(
                "missing protocol340 current registry entry {kind}/{name}"
            ))
        })
}
fn biome(raw: u8) -> Result<i32> {
    if raw == 127 {
        Ok(old::facts::BIOMES.len() as i32)
    } else {
        bootstrap::biome_id(raw)
    }
}
fn inventory_id(id: i32) -> Option<i32> {
    Some(match id {
        0x11 => 0x32,
        0x12 => 0x2e,
        0x13 => 0x2d,
        0x14 => 0x30,
        0x15 => 0x31,
        0x16 => 0x2f,
        _ => return None,
    })
}
fn inventory_out_id(id: i32) -> Result<i32> {
    Ok(match id {
        0x0d => 0x08,
        0x0e => 0x07,
        0x0f => 0x05,
        0x10 => 0x1b,
        0x11 => 0x06,
        0x17 => 0x09,
        _ => return Err(Error::new("unmapped protocol340 inventory output")),
    })
}
fn common_id(id: i32) -> Option<i32> {
    Some(match id {
        0x0f => 0x02,
        0x47 => 0x03,
        0x46 => 0x05,
        0x41 => 0x06,
        0x3a => 0x09,
        0x40 => 0x1f,
        0x10 => 0x22,
        0x0b => 0x23,
        0x0a => 0x24,
        0x08 => 0x25,
        0x1e => 0x2b,
        0x2a => 0x36,
        0x2c => 0x39,
        0x1a => 0x40,
        0x0d => 0x41,
        0x39 => 0x43,
        0x4a => 0x47,
        _ => return None,
    })
}
fn presentation_id(id: i32) -> Option<i32> {
    Some(match id {
        0x07 => 0x37,
        0x2e => 0x38,
        0x0e => 0x3a,
        0x42 => 0x3b,
        0x45 => 0x3c,
        0x3b => 0x3d,
        0x38 => 0x44,
        _ => return None,
    })
}
fn map(r: &mut Reader<'_>) -> Result<play::MapItemData> {
    let map_id = r.varint()?;
    let scale = r.i8()?;
    let _tracking = boolean(r)?;
    if map_id < 0 || !(0..=4).contains(&scale) {
        return Err(Error::new("invalid protocol340 map id/scale"));
    }
    let n = r.count(4096)?;
    let mut decorations = Vec::with_capacity(n.min(256));
    const NAMES: [&str; 10] = [
        "player",
        "frame",
        "red_marker",
        "blue_marker",
        "target_x",
        "target_point",
        "player_off_map",
        "player_off_limits",
        "mansion",
        "monument",
    ];
    for _ in 0..n {
        let packed = r.u8()?;
        let name = NAMES
            .get(usize::from(packed >> 4))
            .ok_or_else(|| Error::new("unknown protocol340 map decoration"))?;
        let kind = registry(
            "minecraft:map_decoration_type",
            &format!("minecraft:{name}"),
        )?;
        decorations.push(play::MapDecoration {
            kind,
            x: r.i8()?,
            y: r.i8()?,
            rot: (packed & 15) as i8,
            name: None,
        });
    }
    let width = i32::from(r.u8()?);
    let patch = if width == 0 {
        None
    } else {
        let height = i32::from(r.u8()?);
        let start_x = i32::from(r.u8()?);
        let start_y = i32::from(r.u8()?);
        let colors = r.byte_array(16384)?;
        if width > 128
            || height <= 0
            || height > 128
            || start_x + width > 128
            || start_y + height > 128
            || colors.len() != (width * height) as usize
        {
            return Err(Error::new("invalid protocol340 map color rectangle"));
        }
        Some(play::MapPatch {
            start_x,
            start_y,
            width,
            height,
            colors,
        })
    };
    Ok(play::MapItemData {
        map_id,
        scale,
        locked: false,
        decorations: Some(decorations),
        patch,
    })
}

fn mapped(
    raw: u16,
    pos: [i32; 3],
    skulls: &HashMap<[i32; 3], (u8, u8)>,
    pots: &HashMap<[i32; 3], i32>,
    beds: &HashMap<[i32; 3], u8>,
    neighbor: impl Fn([i32; 3]) -> Option<u16>,
) -> Result<world::Block> {
    let mut id = *facts::STATES
        .get(usize::from(raw))
        .ok_or_else(|| Error::new("protocol340 state exceeds numeric registry"))?;
    if raw >> 4 == 144 {
        let (kind, rot) = skulls.get(&pos).copied().unwrap_or((0, 0));
        let facing = raw & 7;
        let index = if (2..=5).contains(&facing) {
            16 + usize::from(facing - 2)
        } else {
            usize::from(rot)
        };
        id = *old::facts::SKULL_STATES
            .get(usize::from(kind))
            .and_then(|s| s.get(index))
            .ok_or_else(|| Error::new("unsupported protocol340 skull state"))?;
    }
    if raw >> 4 == 26 {
        id = facts::BED_STATES[usize::from(beds.get(&pos).copied().unwrap_or(14))]
            [usize::from(raw & 15)];
    }
    if raw >> 4 == 140 {
        id = pots.get(&pos).copied().unwrap_or(id);
    }
    if id < 0 {
        return Err(Error::new("unmapped protocol340 block state"));
    }
    if let Some(current) = super::contextual::state(raw, pos, neighbor)? {
        id = current;
    }
    Ok(world::Block {
        id,
        air: raw >> 4 == 0,
        fluid: matches!(raw >> 4, 8..=11),
        sky_opacity: facts::SKY_OPACITY[usize::from(raw)],
    })
}

pub(super) struct Context<'a>(pub &'a Bridge);
impl inventory::Context for Context<'_> {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>> {
        crate::item::item47::read(r)?
            .map(|raw| crate::item::v1_20_3::from_numeric_slot_at(raw, facts::old_item_name, 1343))
            .transpose()
    }
    fn try_write_old_stack(&self, p: Option<&Stack>, w: &mut Writer) -> Result<()> {
        crate::item::v1_20_3::try_write_numeric47_stack(p, w)
    }
    fn try_write_creative_stack(&self, p: Option<&Stack>, w: &mut Writer) -> Result<()> {
        crate::item::v1_20_3::try_write_numeric340_creative_stack(p, w)
    }
    fn entity_kind(&self, id: i32) -> Result<i32> {
        self.0
            .entities
            .kind(id)
            .ok_or_else(|| Error::new("protocol340 menu refers to unknown entity"))
    }
}
impl common::Context for Context<'_> {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn dimension(&self) -> &str {
        self.0
            .join
            .as_ref()
            .and_then(|j| bootstrap::dimension(i32::from(j.dimension)).ok())
            .map_or("minecraft:overworld", |i| bootstrap::DIMENSIONS[i])
    }
    fn block_state(&self, raw: i32, pos: [i32; 3]) -> Result<i32> {
        mapped(
            u16::try_from(raw).map_err(|_| Error::new("invalid protocol340 state"))?,
            pos,
            &self.0.skulls,
            &self.0.pots,
            &self.0.beds,
            |p| self.0.terrain.raw_state(p),
        )
        .map(|b| b.id)
    }
    fn block(&self, raw: i32) -> Result<i32> {
        let name = facts::current_block_name(raw)
            .ok_or_else(|| Error::new("unknown protocol340 block"))?;
        registry("minecraft:block", name)
    }
    fn block_entity_kind(&self, name: &str) -> Result<i32> {
        registry("minecraft:block_entity_type", name)
    }
    fn interact_at(&self, id: i32) -> Result<bool> {
        Ok(self.0.entities.kind(id) == Some(5))
    }
}
impl presentation::Context for Context<'_> {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn registry_id(&self, kind: &str, name: &str) -> Result<i32> {
        registry(kind, name)
    }
}
struct TextContext;
impl common::Context for TextContext {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn dimension(&self) -> &str {
        "minecraft:overworld"
    }
    fn block_state(&self, _: i32, _: [i32; 3]) -> Result<i32> {
        Err(Error::new("unexpected title block lookup"))
    }
    fn block(&self, _: i32) -> Result<i32> {
        Err(Error::new("unexpected title block lookup"))
    }
    fn block_entity_kind(&self, _: &str) -> Result<i32> {
        Err(Error::new("unexpected title block entity lookup"))
    }
    fn interact_at(&self, _: i32) -> Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("extra_goldens.rs");
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn ready() -> Bridge {
        let mut b = Bridge {
            profile_finished: true,
            acknowledged: true,
            ..Default::default()
        };
        let mut w = Writer::new();
        w.i32(17);
        w.u8(1);
        w.i32(0);
        w.u8(2);
        w.u8(20);
        w.string("flat");
        w.bool(false);
        let out = b.clientbound(State::Configuration, 0x23, w.data).unwrap();
        assert_eq!(out.phase, Some(State::Play));
        b
    }

    #[test]
    fn i32_dimension_and_virtual_finish_are_ordered() {
        let mut w = Writer::new();
        w.i32(42);
        w.u8(8 | 1);
        w.i32(-1);
        w.u8(3);
        w.u8(20);
        w.string("default");
        w.bool(true);
        let j = join(&w.data).unwrap();
        assert_eq!(
            (j.dimension, j.mode, j.hardcore, j.reduced_debug),
            (-1, 1, true, true)
        );
        w.u8(0);
        assert!(join(&w.data).is_err());
        let mut b = ready();
        assert!(
            b.serverbound(
                State::Configuration,
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            )
            .is_err()
        );
        b.finish_delivered();
        assert_eq!(
            b.serverbound(
                State::Configuration,
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            )
            .unwrap()
            .phase,
            Some(State::Play)
        );
        assert!(
            b.serverbound(
                State::Configuration,
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn real_offhand_inventory_preserves_all_46_slots() {
        let bridge = ready();
        let mut inventory = inventory::State::new();
        let mut w = Writer::new();
        w.u8(0);
        w.i16(46);
        for _ in 0..46 {
            w.i16(-1);
        }
        let out = inventory
            .clientbound(0x30, &w.data, &Context(&bridge))
            .unwrap()
            .unwrap();
        assert!(
            out.iter().any(
                |p| matches!(p,play::Clientbound::ContainerSetContent(c) if c.items.len()==46)
            )
        );
        let mut w = Writer::new();
        w.i8(-2);
        w.i16(40);
        w.i16(-1);
        let out = inventory
            .clientbound(0x2f, &w.data, &Context(&bridge))
            .unwrap()
            .unwrap();
        assert!(
            out.iter()
                .any(|p| matches!(p,play::Clientbound::ContainerSetSlot(c) if c.slot==45))
        );
    }

    #[test]
    fn keepalive_preserves_64_bits_and_teleports_keep_server_ids() {
        let mut b = ready();
        let mut w = Writer::new();
        w.i64(i64::MAX - 3);
        let p = b.clientbound(State::Play, 0x1f, w.data.clone()).unwrap();
        assert_eq!(
            p.packets,
            vec![(ids::play::clientbound::KEEP_ALIVE, w.data.clone())]
        );
        assert_eq!(
            b.serverbound(State::Play, ids::play::serverbound::KEEP_ALIVE, &w.data)
                .unwrap()
                .packet,
            Some(wire(0x0b, &w.data))
        );
        let mut w = Writer::new();
        for v in [12.5, 64., -7.25] {
            w.f64(v);
        }
        w.f32(90.);
        w.f32(15.);
        w.u8(0);
        w.varint(123456);
        let out = b.clientbound(State::Play, 0x2f, w.data.clone()).unwrap();
        let p = play::Clientbound::decode(out.packets[0].0, &out.packets[0].1).unwrap();
        assert!(
            matches!(p,play::Clientbound::PlayerPosition(p) if p.teleport==123456 && p.position==[12.5,64.,-7.25])
        );
        assert!(b.clientbound(State::Play, 0x2f, w.data).is_err());
        let mut ack = Writer::new();
        play::AcceptTeleportation::write(
            &play::AcceptTeleportation {
                id: 123456,
                x: 12.5,
                y: 64.,
                z: -7.25,
                yaw: 90.,
                pitch: 15.,
            },
            &mut ack,
        );
        let mut expected = Writer::new();
        expected.varint(123456);
        assert_eq!(
            b.serverbound(
                State::Play,
                ids::play::serverbound::ACCEPT_TELEPORTATION,
                &ack.data
            )
            .unwrap()
            .packet,
            Some(wire(0, &expected.data))
        );
        assert!(
            b.serverbound(
                State::Play,
                ids::play::serverbound::ACCEPT_TELEPORTATION,
                &ack.data
            )
            .is_err()
        );
    }
    #[test]
    fn actual_maps_recipe_ghost_and_object_writers_keep_semantics_and_boundaries() {
        for &(name, id, text) in EXTRAS {
            let bytes = hex(text);
            let mut b = ready();
            let out = b
                .clientbound(State::Play, id, bytes.clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let packets: Vec<_> = out
                .packets
                .into_iter()
                .map(|(id, body)| play::Clientbound::decode(id, &body).unwrap())
                .collect();
            match name {
                "map_patch_two_icons" => {
                    let play::Clientbound::MapItemData(p) = &packets[0] else {
                        panic!("map")
                    };
                    assert_eq!((p.map_id, p.scale), (312, 3));
                    assert_eq!(p.decorations.as_ref().unwrap().len(), 2);
                    let patch = p.patch.as_ref().unwrap();
                    assert_eq!(
                        (patch.start_x, patch.start_y, patch.width, patch.height),
                        (11, 13, 2, 3)
                    );
                    assert_eq!(patch.colors, vec![1, 2, 3, 4, 5, 6]);
                }
                "map_no_patch" => assert!(
                    matches!(&packets[0],play::Clientbound::MapItemData(p)if p.map_id==313&&p.patch.is_none())
                ),
                "ghost_crafting_table" => {
                    let play::Clientbound::PlaceGhostRecipe(p) = &packets[0] else {
                        panic!("recipe")
                    };
                    assert_eq!(p.container, 17);
                    let crate::packets::recipes::SlotDisplay::ItemStack(stack) = p.display.result()
                    else {
                        panic!("result stack")
                    };
                    assert_eq!(
                        stack.item,
                        registry("minecraft:item", "minecraft:crafting_table").unwrap()
                    );
                }
                "object_71" => assert!(
                    matches!(&packets[0],play::Clientbound::AddEntity(p)if p.data==3&&p.position==[1.25,64.5,-2.75])
                ),
                _ => {
                    let play::Clientbound::AddEntity(p) = &packets[0] else {
                        panic!("entity")
                    };
                    assert_eq!(p.velocity, [0.; 3]);
                    assert_eq!(b.take_projectile_acceleration(p.id), Some([0.1, -0.2, 0.3]));
                    assert!(b.take_projectile_acceleration(p.id).is_none());
                }
            }
            for end in 0..bytes.len() {
                assert!(
                    ready()
                        .clientbound(State::Play, id, bytes[..end].to_vec())
                        .is_err(),
                    "{name} at{end}"
                );
            }
            let mut extra = bytes;
            extra.push(0);
            assert!(
                ready().clientbound(State::Play, id, extra).is_err(),
                "{name} trailing"
            );
        }
    }
    #[test]
    fn old_tile_color_selects_actual_blue_bed_state() {
        let mut b = ready();
        let pos = [2, 64, 3];
        assert_eq!(
            mapped(26 << 4, pos, &b.skulls, &b.pots, &b.beds, |_| Some(0))
                .unwrap()
                .id,
            2261
        );
        let tag = Tag::compound([
            ("id", d::string("minecraft:bed")),
            ("x", Tag::Int(2)),
            ("y", Tag::Int(64)),
            ("z", Tag::Int(3)),
            ("color", Tag::Int(11)),
        ]);
        b.contextual_tile(&tag).unwrap();
        assert_eq!(
            mapped(26 << 4, pos, &b.skulls, &b.pots, &b.beds, |_| Some(0))
                .unwrap()
                .id,
            2213
        );
    }
}

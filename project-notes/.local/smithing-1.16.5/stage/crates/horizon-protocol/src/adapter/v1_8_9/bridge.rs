//! Protocol47's connection state. Native consumer packets are emitted directly as protocol777.
use super::{
    Join, bootstrap, common, entity, facts, ids as old_ids, inventory, lookup47, neighbors47,
    presentation, world, worldfx, worldfx_extra,
};
use crate::{
    Error, Result, State,
    buf::{Codec, Reader, Writer},
    ids,
    packets::{configuration as cfg, login, play},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

#[derive(Default)]
pub struct Bridge {
    online_mode: bool,
    local_rejection: bool,
    profile_finished: bool,
    login_acknowledged: bool,
    finish_queued: bool,
    finish_delivered: bool,
    finished: bool,
    deferred: Vec<Vec<u8>>,
    join: Option<Join>,
    primitives: super::State,
    terrain: world::World,
    skulls: RefCell<HashMap<[i32; 3], (u8, u8)>>,
    pots: RefCell<HashMap<[i32; 3], i32>>,
    common: Option<common::State>,
    entities: entity::lifecycle::Entities,
    presentation: Option<presentation::State>,
    profile_uuid: Option<u128>,
    inventory: Option<inventory::State>,
    selected: u8,
    last_notice: Option<std::time::Instant>,
    predictions: Vec<(i32, Vec<u8>)>,
    pending_outgoing: Vec<Vec<u8>>,
    projectile_acceleration: HashMap<i32, [f64; 3]>,
    resource_id: Option<u128>,
    worldfx: worldfx::State,
    listener: Option<[f64; 3]>,
    camera: Option<i32>,
    random_state: Cell<u64>,
    cosmetic_omissions: std::collections::BTreeSet<&'static str>,
}

#[derive(Default)]
pub struct Incoming {
    pub phase: Option<State>,
    pub packets: Vec<(i32, Vec<u8>)>,
    pub outgoing: Vec<Vec<u8>>,
}
#[derive(Default)]
pub struct Outgoing {
    pub phase: Option<State>,
    pub packet: Option<Vec<u8>>,
    pub extra: Vec<Vec<u8>>,
}

fn wire(id: i32, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    w.bytes(body);
    w.data
}

impl Bridge {
    /// Adapter-owned metadata, consumed exactly once with its translated AddEntity.
    pub fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        self.projectile_acceleration.remove(&id)
    }
    pub fn take_predictions(&mut self) -> Vec<(i32, Vec<u8>)> {
        std::mem::take(&mut self.predictions)
    }
    pub fn rejected_locally(&self) -> bool {
        self.local_rejection
    }
    pub fn online_mode(&self) -> bool {
        self.online_mode
    }
    pub fn finish_delivered(&mut self) {
        if self.finish_queued {
            self.finish_delivered = true;
        }
    }

    pub fn clientbound(&mut self, state: State, id: i32, payload: Vec<u8>) -> Result<Incoming> {
        let mut out = Incoming::default();
        match state {
            State::Status => out.packets.push((id, payload)),
            State::Login => match id {
                0 | 3 => out.packets.push((id, payload)),
                1 => {
                    let mut r = Reader::new(&payload);
                    let packet = login::Hello {
                        server_id: r.string(20)?,
                        public_key: r.byte_array(1 << 20)?,
                        challenge: r.byte_array(1 << 20)?,
                        should_authenticate: true,
                    };
                    super::finish(&r)?;
                    self.online_mode = true;
                    out.packets.push(bootstrap::packet(
                        login::Clientbound::Hello(packet).encode(),
                    ));
                }
                2 => {
                    if self.profile_finished {
                        return Err(Error::new("duplicate legacy login profile"));
                    }
                    let profile = super::login_finished(&payload)?;
                    let profile = login::GameProfile::read(&mut Reader::new(&profile))?;
                    self.profile_uuid = Some(profile.uuid);
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
                _ => return Err(Error::new(format!("unknown protocol47 login packet {id}"))),
            },
            State::Configuration => {
                if !self.profile_finished || self.finish_queued {
                    return Err(Error::new("unexpected legacy configuration state"));
                }
                if id == 0 {
                    out.packets.push((
                        ids::configuration::clientbound::KEEP_ALIVE,
                        super::keep_alive_up(&payload)?,
                    ));
                    return Ok(out);
                }
                if id == 0x40 {
                    let reason = crate::adapter::read_legacy_component(&mut Reader::new(&payload))?;
                    out.packets.push(bootstrap::packet(
                        cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode(),
                    ));
                    return Ok(out);
                }
                if id != 1 {
                    return Err(Error::new("legacy play packet precedes JoinGame"));
                }
                let join = super::join(&payload)?;
                let login = bootstrap::login(&join, self.online_mode)?;
                for registry in bootstrap::registries() {
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
                    Vec::new(),
                ));
                out.packets
                    .push(bootstrap::packet(play::Clientbound::Login(login).encode()));
                out.packets
                    .push(bootstrap::packet(bootstrap::commands().encode()));
                // Old protocol has no chunk-batch/world-loading-start event.
                out.packets.push(bootstrap::packet(
                    play::Clientbound::GameEvent(play::GameEvent {
                        event: 13,
                        param: 0.0,
                    })
                    .encode(),
                ));
                let mut seed = [0; 8];
                crate::crypto::random_bytes(&mut seed)?;
                self.random_state.set(u64::from_ne_bytes(seed));
                self.common = Some(common::State::new(join.entity));
                self.presentation = Some(presentation::State::default());
                self.entities.register_local_player(
                    join.entity,
                    self.profile_uuid
                        .ok_or_else(|| Error::new("legacy join without profile"))?,
                )?;
                let mut inventory = inventory::State::new();
                inventory.set_creative(join.mode == 1);
                self.inventory = Some(inventory);
                self.join = Some(join);
                self.finish_queued = true;
                out.phase = Some(State::Play);
            }
            State::Play => {
                let packets = self.play(id, &payload)?;
                out.packets
                    .extend(packets.into_iter().map(|p| bootstrap::packet(p.encode())));
                out.outgoing.append(&mut self.pending_outgoing);
            }
            State::Handshake => return Err(Error::new("server packet before legacy handshake")),
        }
        Ok(out)
    }

    fn play(&mut self, id: i32, payload: &[u8]) -> Result<Vec<play::Clientbound>> {
        let neighbor_updates = self.retain_block_changes(id, payload)?;
        if let Some(output) = worldfx_extra::clientbound(id, payload, &Context(self))? {
            if let Some(reason) = output.omitted_cosmetic
                && self.cosmetic_omissions.insert(reason)
            {
                log::warn!("ViaHorizon: recognized legacy cosmetic omission: {reason}");
            }
            return Ok(output.packets);
        }
        let mut effects = std::mem::take(&mut self.worldfx);
        let mut uuid = [0u8; 16];
        if id == 44 {
            crate::crypto::random_bytes(&mut uuid)?;
            uuid[6] = (uuid[6] & 15) | 64;
            uuid[8] = (uuid[8] & 63) | 128;
        }
        let output =
            effects.clientbound(id, payload, &Context(self), |_| u128::from_be_bytes(uuid));
        self.worldfx = effects;
        if let Some(mut output) = output? {
            output
                .packets
                .extend(self.apply_raw_changes(&output.block_changes)?);
            return Ok(output.packets);
        }
        if let Some(mut inventory) = self.inventory.take() {
            let result = inventory.clientbound(id, payload, &Context(self));
            let outgoing = inventory.take_outgoing();
            let mounts = inventory.take_mount_updates();
            self.inventory = Some(inventory);
            for update in mounts {
                let raw = update.raw.map(|s| entity::raw::Slot {
                    id: s.id,
                    count: s.count,
                    damage: s.damage,
                    tag: s.tag,
                });
                self.entities.remember_horse_slot(
                    update.entity,
                    update.old_slot,
                    raw.as_ref(),
                    update.item,
                )?;
            }
            self.pending_outgoing
                .extend(outgoing.into_iter().map(|p| wire(p.id, &p.payload)));
            if let Some(packets) = result? {
                return Ok(packets);
            }
        }
        if let Some(mut presentation) = self.presentation.take() {
            let result = presentation.clientbound(id, payload, &Context(self));
            let outgoing = presentation.take_outgoing();
            self.presentation = Some(presentation);
            self.pending_outgoing
                .extend(outgoing.into_iter().map(|p| wire(p.id, &p.payload)));
            if let Some(packets) = result? {
                self.retain_player_state(&packets);
                return Ok(packets);
            }
        }
        if let Some(mut common) = self.common.take() {
            let result = common.clientbound(id, payload, &Context(self));
            self.common = Some(common);
            if let Some(mut packets) = result? {
                packets.extend(neighbor_updates);
                self.retain_player_state(&packets);
                return Ok(packets);
            }
        }
        let mut uuid = [0u8; 16];
        if matches!(id, 14..=17) {
            crate::crypto::random_bytes(&mut uuid)?;
            uuid[6] = (uuid[6] & 15) | 64;
            uuid[8] = (uuid[8] & 63) | 128;
        }
        let output = {
            let mut item = |slot: &entity::raw::Slot| {
                crate::item::v1_20_3::from_numeric47_slot(
                    crate::item::item47::Raw47Slot {
                        id: slot.id,
                        count: slot.count,
                        damage: slot.damage,
                        tag: slot.tag.clone(),
                    },
                    old_ids::old_item_name,
                )
            };
            let skulls = &self.skulls.borrow();
            let mut block = |raw| mapped_block(raw, [0, 0, 0], skulls).map(|b| b.id);
            let mut registry = |name: &str, value: &str| {
                bootstrap::names()
                    .get(name)
                    .and_then(|r| r.id(value))
                    .ok_or_else(|| {
                        Error::new(format!("missing declared registry value {name}/{value}"))
                    })
            };
            let mut component = |text: &str| {
                let mut w = Writer::new();
                w.string(text);
                crate::adapter::read_legacy_component(&mut Reader::new(&w.data))
            };
            self.entities.clientbound(
                id,
                payload,
                |_| u128::from_be_bytes(uuid),
                &mut entity::lifecycle::Callbacks {
                    item: &mut item,
                    block: &mut block,
                    registry: &mut registry,
                    component: &mut component,
                },
            )?
        };
        if let Some(output) = output {
            if !output.untranslated.is_empty() {
                log::warn!(
                    "ViaHorizon: {} unsupported legacy entity metadata entries",
                    output.untranslated.len()
                );
            }
            for extra in output.extras {
                match extra {
                    entity::objects::Extra::ProjectileAcceleration(vector) => {
                        let id = output
                            .packets
                            .iter()
                            .find_map(|packet| match packet {
                                play::Clientbound::AddEntity(p) => Some(p.id),
                                _ => None,
                            })
                            .ok_or_else(|| Error::new("legacy acceleration without AddEntity"))?;
                        if vector.iter().any(|v| !v.is_finite()) || vector.iter().all(|v| *v == 0.0)
                        {
                            return Err(Error::new(
                                "undefined legacy projectile acceleration direction",
                            ));
                        }
                        if self.projectile_acceleration.insert(id, vector).is_some() {
                            return Err(Error::new("unconsumed legacy projectile acceleration"));
                        }
                    }
                }
            }
            let mut packets = output.packets;
            for packet in &mut packets {
                self.worldfx.filter_entity_packet(packet);
            }
            return Ok(packets);
        }
        let mut r = Reader::new(payload);
        let packet = match id {
            0 => play::Clientbound::KeepAlive(cfg::KeepAlive {
                id: i64::from(r.varint()?),
            }),
            7 => {
                let dimension = r.i32()?;
                let difficulty = r.u8()?;
                let mode = r.u8()?;
                let level_type = r.string(16)?;
                super::finish(&r)?;
                bootstrap::dimension(dimension)?;
                if difficulty > 3 || mode > 3 {
                    return Err(Error::new("invalid legacy respawn"));
                }
                let join = self
                    .join
                    .as_mut()
                    .ok_or_else(|| Error::new("legacy respawn before join"))?;
                join.dimension = dimension as i8;
                join.difficulty = difficulty;
                join.mode = mode;
                join.level_type = level_type;
                let spawn = bootstrap::spawn(join)?;
                self.terrain.reset();
                self.skulls.borrow_mut().clear();
                self.pots.borrow_mut().clear();
                self.entities.clear_remote();
                self.projectile_acceleration.clear();
                self.worldfx.clear();
                self.listener = None;
                self.camera = None;
                self.entities.register_local_player(
                    join.entity,
                    self.profile_uuid
                        .ok_or_else(|| Error::new("legacy respawn without profile"))?,
                )?;
                self.common = Some(common::State::new(join.entity));
                if let Some(inventory) = &mut self.inventory {
                    inventory.on_respawn();
                    inventory.set_creative(join.mode == 1);
                }
                return Ok(vec![
                    play::Clientbound::Respawn(play::Respawn { spawn, keep: 0 }),
                    play::Clientbound::ChangeDifficulty(play::ChangeDifficulty {
                        difficulty: i32::from(join.difficulty),
                        locked: false,
                    }),
                ]);
            }
            8 => {
                let normalized = self.primitives.player_position(payload)?;
                let mut r = Reader::new(&normalized);
                let position = [r.f64()?, r.f64()?, r.f64()?];
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let flags = i32::from(r.u8()?);
                let teleport = r.varint()?;
                r.bool()?;
                super::finish(&r)?;
                if flags & 7 == 0 || self.listener.is_some() {
                    let previous = self.listener.unwrap_or([0.; 3]);
                    self.listener = Some(std::array::from_fn(|k| {
                        position[k]
                            + if flags & (1 << k) != 0 {
                                previous[k]
                            } else {
                                0.
                            }
                    }));
                }
                return Ok(vec![play::Clientbound::PlayerPosition(
                    play::PlayerPosition {
                        teleport,
                        position,
                        delta: [0.0; 3],
                        yaw,
                        pitch,
                        relatives: flags | ((flags & 7) << 5),
                    },
                )]);
            }
            0x21 | 0x26 => {
                let has_sky = self.join.as_ref().is_some_and(|j| j.dimension == 0);
                let updates = if id == 0x21 {
                    vec![super::terrain::single(payload, has_sky)?]
                } else {
                    super::terrain::bulk(payload)?
                };
                if updates.iter().any(|u| u.has_sky != has_sky) {
                    return Err(Error::new("legacy chunk sky disagrees with dimension"));
                }
                let snapshot = self.terrain.snapshot(&updates)?;
                let touched: std::collections::BTreeSet<_> =
                    updates.iter().map(|u| (u.x, u.z)).collect();
                let replaced: std::collections::BTreeSet<_> = updates
                    .iter()
                    .filter(|u| u.full || u.unload)
                    .map(|u| (u.x, u.z))
                    .collect();
                let mut skulls = self.skulls.borrow().clone();
                let mut pots = self.pots.borrow().clone();
                let retains_kind = |pos: [i32; 3], kind| {
                    let key = (pos[0] >> 4, pos[2] >> 4);
                    !replaced.contains(&key)
                        && snapshot.get(&key).is_none_or(|c| {
                            world::raw_in(c, pos).is_some_and(|raw| raw >> 4 == kind)
                        })
                };
                skulls.retain(|pos, _| retains_kind(*pos, 144));
                pots.retain(|pos, _| retains_kind(*pos, 140));
                let mut packets = self.terrain.apply(
                    updates,
                    |x, z, y, i, raw| {
                        let pos = [
                            x * 16 + (i & 15) as i32,
                            (y * 16 + (i >> 8)) as i32,
                            z * 16 + ((i >> 4) & 15) as i32,
                        ];
                        mapped_contextual(raw, pos, &skulls, &pots, |p| {
                            snapshot
                                .get(&(p[0] >> 4, p[2] >> 4))
                                .and_then(|c| world::raw_in(c, p))
                        })
                    },
                    bootstrap::biome_id,
                    16,
                    6,
                )?;
                *self.skulls.borrow_mut() = skulls;
                *self.pots.borrow_mut() = pots;
                // Updated columns already used the full atomic snapshot. Only the outside
                // neighbor's boundary needs a narrow refresh; replacing it would erase NBT.
                let mut refresh = std::collections::BTreeSet::new();
                for &(x, z) in &touched {
                    for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let neighbor = (x + dx, z + dz);
                        if touched.contains(&neighbor)
                            || self.terrain.column(neighbor.0, neighbor.1).is_none()
                        {
                            continue;
                        }
                        for y in 0..256 {
                            for i in 0..16 {
                                let pos = if dx != 0 {
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
                                };
                                if self
                                    .terrain
                                    .raw_state(pos)
                                    .is_some_and(neighbors47::needs_context)
                                {
                                    refresh.insert(pos);
                                }
                            }
                        }
                    }
                }
                packets.extend(self.context_updates(refresh)?);
                return Ok(packets);
            }
            0x3f => {
                let channel = r.string(20)?;
                return match channel.as_str() {
                    "MC|Brand" => Ok(vec![play::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: r.rest().to_vec(),
                    })]),
                    "MC|BOpen" => {
                        super::finish(&r)?;
                        Ok(vec![play::Clientbound::OpenBook(play::OpenBook {
                            hand: 0,
                        })])
                    }
                    "MC|TrList" => Err(Error::new(
                        "legacy merchant offers require the separate trade adapter",
                    )),
                    // Vanilla's old client ignores unrecognized plugin messages. Such channels are
                    // not packet layouts which may be misread as a different native packet.
                    _ => Ok(Vec::new()),
                };
            }
            0x48 => {
                let url = r.string(32767)?;
                let hash = r.string(40)?;
                super::finish(&r)?;
                let id = crate::crypto::offline_uuid(&format!("HorizonViaLegacyPack:{url}:{hash}"));
                self.primitives.resource_pack(payload)?;
                self.resource_id = Some(id);
                return Ok(vec![play::Clientbound::ResourcePackPush(
                    cfg::ResourcePackPush {
                        id,
                        url,
                        hash,
                        required: false,
                        prompt: None,
                    },
                )]);
            }
            _ => return Err(Error::new(format!("unhandled protocol47 play packet {id}"))),
        };
        super::finish(&r)?;
        Ok(vec![packet])
    }

    pub fn serverbound(&mut self, state: State, id: i32, payload: &[u8]) -> Result<Outgoing> {
        self.local_rejection = false;
        let mut out = Outgoing::default();
        match state {
            State::Status => out.packet = Some(wire(id, payload)),
            State::Login => match id {
                ids::login::serverbound::HELLO => {
                    let mut r = Reader::new(payload);
                    let name = r.string(16)?;
                    r.uuid()?;
                    super::finish(&r)?;
                    let mut w = Writer::new();
                    w.string(&name);
                    out.packet = Some(wire(0, &w.data));
                }
                ids::login::serverbound::KEY => out.packet = Some(wire(1, payload)),
                ids::login::serverbound::LOGIN_ACKNOWLEDGED => {
                    if !payload.is_empty() || !self.profile_finished || self.login_acknowledged {
                        return Err(Error::new(
                            "unexpected virtual legacy login acknowledgement",
                        ));
                    }
                    self.login_acknowledged = true;
                    out.phase = Some(State::Configuration);
                }
                _ => {}
            },
            State::Configuration => {
                if id == ids::configuration::serverbound::FINISH_CONFIGURATION {
                    if !payload.is_empty()
                        || !self.login_acknowledged
                        || !self.finish_delivered
                        || self.finished
                    {
                        return Err(Error::new("premature virtual legacy configuration finish"));
                    }
                    self.finished = true;
                    out.phase = Some(State::Play);
                    out.extra.append(&mut self.deferred);
                    return Ok(out);
                }
                let packet = match id {
                    ids::configuration::serverbound::CLIENT_INFORMATION => {
                        Some(self.settings(payload)?)
                    }
                    ids::configuration::serverbound::CUSTOM_PAYLOAD => custom_payload(payload)?,
                    ids::configuration::serverbound::KEEP_ALIVE => {
                        out.packet = Some(wire(0, &super::keep_alive_down(payload)?));
                        return Ok(out);
                    }
                    _ => None,
                };
                if let Some(packet) = packet {
                    if self.deferred.len() >= 128
                        || self.deferred.iter().map(Vec::len).sum::<usize>() + packet.len()
                            > 1 << 20
                    {
                        return Err(Error::new("legacy deferred packet limit exceeded"));
                    }
                    self.deferred.push(packet);
                }
            }
            State::Play => match id {
                ids::play::serverbound::KEEP_ALIVE => {
                    out.packet = Some(wire(0, &super::keep_alive_down(payload)?))
                }
                ids::play::serverbound::ACCEPT_TELEPORTATION => {
                    let mut r = Reader::new(payload);
                    let ack = play::AcceptTeleportation::read(&mut r)?;
                    super::finish(&r)?;
                    if [ack.x, ack.y, ack.z].iter().any(|x| !x.is_finite())
                        || !ack.yaw.is_finite()
                        || !ack.pitch.is_finite()
                    {
                        return Err(Error::new("invalid legacy teleport acceptance"));
                    }
                    let mut ack_id = Writer::new();
                    ack_id.varint(ack.id);
                    self.primitives.acknowledge_teleport(&ack_id.data)?;
                    self.listener = Some([ack.x, ack.y, ack.z]);
                    let mut w = Writer::new();
                    for value in [ack.x, ack.y, ack.z] {
                        w.f64(value);
                    }
                    w.f32(ack.yaw);
                    w.f32(ack.pitch);
                    w.bool(false);
                    out.packet = Some(wire(6, &w.data));
                }
                ids::play::serverbound::CLIENT_INFORMATION => {
                    out.packet = Some(self.settings(payload)?)
                }
                ids::play::serverbound::CUSTOM_PAYLOAD => out.packet = custom_payload(payload)?,
                ids::play::serverbound::RESOURCE_PACK => {
                    let mut r = Reader::new(payload);
                    let response = cfg::ResourcePackResponse::read(&mut r)?;
                    super::finish(&r)?;
                    if self.resource_id != Some(response.id) {
                        return Err(Error::new("legacy resource response has wrong request ID"));
                    }
                    let status = match response.action {
                        0..=3 => Some(response.action),
                        4 => None,
                        5 | 6 => Some(2),
                        7 => Some(1),
                        _ => return Err(Error::new("unknown resource pack response")),
                    };
                    if let Some(status) = status {
                        let mut w = Writer::new();
                        w.varint(status);
                        out.packet = Some(wire(0x19, &self.primitives.resource_status(&w.data)?));
                    }
                }
                _ => {
                    let p = play::Serverbound::decode(id, payload)?;
                    if matches!(&p,play::Serverbound::Interact(p) if p.hand==1) {
                        return Ok(out);
                    }
                    if let Some(mut inventory) = self.inventory.take() {
                        let result = inventory.serverbound(&p, &Context(self));
                        self.predictions.extend(
                            inventory
                                .take_incoming()
                                .into_iter()
                                .map(|p| bootstrap::packet(p.encode())),
                        );
                        if result.is_err()
                            && let Some(correction) = inventory.correction()
                        {
                            self.predictions
                                .push(bootstrap::packet(correction.encode()));
                        }
                        self.inventory = Some(inventory);
                        match result {
                            Ok(Some(packets)) => {
                                let mut packets =
                                    packets.into_iter().map(|p| wire(p.id, &p.payload));
                                out.packet = packets.next();
                                out.extra.extend(packets);
                                return Ok(out);
                            }
                            Err(error) => {
                                self.unavailable(&error);
                                return Ok(out);
                            }
                            Ok(None) => {}
                        }
                    }
                    if let play::Serverbound::UseItemOn(_) | play::Serverbound::UseItem(_) = &p {
                        match self.use_item(&p) {
                            Ok(packet) => out.packet = packet,
                            Err(error) => self.unavailable(&error),
                        }
                        let sequence = match &p {
                            play::Serverbound::UseItemOn(p) => p.sequence,
                            play::Serverbound::UseItem(p) => p.sequence,
                            _ => unreachable!(),
                        };
                        self.queue_prediction_ack(sequence);
                        return Ok(out);
                    }
                    if let Some(mut presentation) = self.presentation.take() {
                        let result = presentation.serverbound(&p, &Context(self));
                        self.presentation = Some(presentation);
                        if let Some(packets) = result? {
                            let mut packets = packets.into_iter().map(|p| wire(p.id, &p.payload));
                            out.packet = packets.next();
                            out.extra.extend(packets);
                            return Ok(out);
                        }
                    }
                    if let Some(mut common) = self.common.take() {
                        let result = common.serverbound(&p, &Context(self));
                        self.common = Some(common);
                        let packets = match result {
                            Ok(packets) => packets,
                            Err(error) => {
                                if let Some(correction) = self
                                    .inventory
                                    .as_ref()
                                    .and_then(inventory::State::correction)
                                {
                                    self.predictions
                                        .push(bootstrap::packet(correction.encode()));
                                }
                                if let play::Serverbound::PlayerAction(action) = &p {
                                    self.queue_prediction_ack(action.sequence);
                                }
                                self.unavailable(&error);
                                return Ok(out);
                            }
                        };
                        if let Some(packets) = packets {
                            let mut packets = packets.into_iter().map(|p| wire(p.id, &p.payload));
                            out.packet = packets.next();
                            out.extra.extend(packets);
                            if let play::Serverbound::PlayerAction(action) = &p {
                                self.queue_prediction_ack(action.sequence);
                            }
                            match &p {
                                play::Serverbound::SetCarriedItem(p) => {
                                    self.selected = p.slot as u8
                                }
                                play::Serverbound::MovePlayerPos(p) => {
                                    self.listener = Some([p.x, p.y, p.z])
                                }
                                play::Serverbound::MovePlayerPosRot(p) => {
                                    self.listener = Some([p.x, p.y, p.z])
                                }
                                _ => {}
                            }
                        } else {
                            self.unavailable(&Error::new(format!(
                                "current action {id} has no protocol47 representation"
                            )));
                        }
                    }
                }
            },
            State::Handshake => {
                return Err(Error::new(
                    "legacy handshake must pass through Session intention",
                ));
            }
        }
        Ok(out)
    }

    fn settings(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(payload);
        let info = cfg::ClientInformation::read(&mut r)?;
        super::finish(&r)?;
        if info.language.encode_utf16().count() > 7 || !(0..=2).contains(&info.chat_visibility) {
            return Err(Error::new(
                "client information is outside protocol47 limits",
            ));
        }
        let mut w = Writer::new();
        w.string(&info.language);
        w.i8(info.view_distance);
        w.u8(info.chat_visibility as u8);
        w.bool(info.chat_colors);
        w.u8(info.model_customisation);
        Ok(wire(0x15, &w.data))
    }

    fn queue_prediction_ack(&mut self, sequence: i32) {
        // Session will expose the prediction acknowledgement directly to the native consumer.
        self.predictions.push(bootstrap::packet(
            play::Clientbound::BlockChangedAck(play::BlockChangedAck { sequence }).encode(),
        ));
    }

    fn retain_player_state(&mut self, packets: &[play::Clientbound]) {
        for packet in packets {
            match packet {
                play::Clientbound::SetHeldSlot(p) => self.selected = p.slot as u8,
                play::Clientbound::SetCamera(p) => self.camera = Some(p.camera_id),
                play::Clientbound::GameEvent(p)
                    if p.event == 3 && (0.0..=3.0).contains(&p.param) =>
                {
                    if let Some(join) = &mut self.join {
                        join.mode = p.param as u8;
                    }
                    if let Some(inventory) = &mut self.inventory {
                        inventory.set_creative(p.param as i32 == 1);
                    }
                }
                _ => {}
            }
        }
    }

    fn unavailable(&mut self, error: &Error) {
        self.local_rejection = true;
        crate::item::v1_20_3::reject_numeric47_transaction();
        log::warn!("ViaHorizon: outgoing legacy action rejected locally: {error}");
        if self
            .last_notice
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        self.last_notice = Some(std::time::Instant::now());
        use horizon_world::nbt::Tag;
        let content = Tag::Compound(
            [(
                "translate".into(),
                Tag::String("horizon.multiplayer.horizon_via.unavailable_action".into()),
            )]
            .into(),
        );
        self.predictions.push(bootstrap::packet(
            play::Clientbound::SystemChat(play::SystemChat {
                content,
                overlay: false,
            })
            .encode(),
        ));
    }

    fn use_item(&self, packet: &play::Serverbound) -> Result<Option<Vec<u8>>> {
        let mut w = Writer::new();
        let (hand, pos, face, cursor) = match packet {
            play::Serverbound::UseItemOn(p) => {
                if !(0..=5).contains(&p.face)
                    || p.cursor
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(Error::new("invalid legacy block interaction"));
                }
                (
                    p.hand,
                    ((i64::from(p.pos[0]) & 0x3ff_ffff) << 38)
                        | ((i64::from(p.pos[1]) & 0xfff) << 26)
                        | (i64::from(p.pos[2]) & 0x3ff_ffff),
                    p.face as u8,
                    p.cursor,
                )
            }
            play::Serverbound::UseItem(p) => (p.hand, -1, 255, [0.0; 3]),
            _ => unreachable!(),
        };
        if hand == 1 {
            return Ok(None);
        } // The virtual offhand is empty on this protocol.
        if hand != 0 {
            return Err(Error::new("unknown interaction hand"));
        }
        let stack = self
            .inventory
            .as_ref()
            .ok_or_else(|| Error::new("legacy inventory not initialized"))?
            .main_hand(self.selected)?;
        w.i64(pos);
        w.u8(face);
        crate::item::v1_20_3::try_write_numeric47_stack(stack, &mut w)?;
        for value in cursor {
            w.u8((value * 16.0) as u8);
        }
        Ok(Some(wire(0x08, &w.data)))
    }

    fn retain_block_changes(&mut self, id: i32, payload: &[u8]) -> Result<Vec<play::Clientbound>> {
        let mut r = Reader::new(payload);
        let changes = match id {
            0x23 => vec![(common::position_up(r.i64()?), r.varint()?)],
            0x22 => {
                let x = r.i32()?;
                let z = r.i32()?;
                if !(-2_097_152..2_097_152).contains(&x) || !(-2_097_152..2_097_152).contains(&z) {
                    return Err(Error::new(
                        "legacy block-change chunk coordinate out of range",
                    ));
                }
                let count = r.count(65536)?;
                let mut changes = Vec::with_capacity(count);
                for _ in 0..count {
                    let packed = r.u16()?;
                    changes.push((
                        [
                            x * 16 + i32::from(packed >> 12),
                            i32::from(packed & 255),
                            z * 16 + i32::from((packed >> 8) & 15),
                        ],
                        r.varint()?,
                    ));
                }
                changes
            }
            _ => return Ok(Vec::new()),
        };
        super::finish(&r)?;
        let changes = changes
            .into_iter()
            .map(|(pos, raw)| {
                Ok((
                    pos,
                    u16::try_from(raw).map_err(|_| Error::new("legacy raw state out of range"))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        self.apply_raw_changes(&changes)
    }

    fn context_updates(
        &self,
        positions: impl IntoIterator<Item = [i32; 3]>,
    ) -> Result<Vec<play::Clientbound>> {
        positions
            .into_iter()
            .filter_map(|pos| {
                self.terrain
                    .raw_state(pos)
                    .filter(|&raw| neighbors47::needs_context(raw))
                    .map(|raw| (pos, raw))
            })
            .map(|(pos, raw)| {
                Ok(play::Clientbound::BlockUpdate(play::BlockUpdate {
                    pos,
                    state: Context(self).block_state_checked(i32::from(raw), pos)?,
                }))
            })
            .collect()
    }

    fn apply_raw_changes(&mut self, changes: &[([i32; 3], u16)]) -> Result<Vec<play::Clientbound>> {
        let overlay: HashMap<_, _> = changes.iter().copied().collect();
        let mut affected = std::collections::BTreeSet::new();
        for &(pos, raw) in changes {
            if !(0..256).contains(&pos[1]) {
                continue;
            }
            mapped_block(raw, pos, &self.skulls.borrow())?;
            for delta in [
                [0, 0, 0],
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ] {
                let at = std::array::from_fn(|k| pos[k] + delta[k]);
                if (0..256).contains(&at[1]) {
                    affected.insert(at);
                }
            }
        }
        let lookup = |p| {
            overlay
                .get(&p)
                .copied()
                .or_else(|| self.terrain.raw_state(p))
        };
        let mut packets = Vec::new();
        for pos in affected {
            if let Some(raw) = lookup(pos).filter(|&raw| neighbors47::needs_context(raw)) {
                let state = mapped_contextual(
                    raw,
                    pos,
                    &self.skulls.borrow(),
                    &self.pots.borrow(),
                    lookup,
                )?
                .id;
                packets.push(play::Clientbound::BlockUpdate(play::BlockUpdate {
                    pos,
                    state,
                }));
            }
        }
        for &(pos, raw) in changes {
            if !(0..256).contains(&pos[1]) {
                continue;
            }
            self.terrain.set_raw(pos, raw)?;
            if raw >> 4 != 144 {
                self.skulls.borrow_mut().remove(&pos);
            }
            if raw >> 4 != 140 {
                self.pots.borrow_mut().remove(&pos);
            }
        }
        Ok(packets)
    }
}

struct Context<'a>(&'a Bridge);
impl Context<'_> {
    fn block_state_checked(&self, raw: i32, pos: [i32; 3]) -> Result<i32> {
        common::Context::block_state(self, raw, pos)
    }
}
impl common::Context for Context<'_> {
    fn text(&self, r: &mut Reader<'_>) -> Result<horizon_world::nbt::Tag> {
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
        let raw = u16::try_from(raw).map_err(|_| Error::new("legacy block state out of range"))?;
        mapped_contextual(
            raw,
            pos,
            &self.0.skulls.borrow(),
            &self.0.pots.borrow(),
            |p| self.0.terrain.raw_state(p),
        )
        .map(|b| b.id)
    }
    fn block(&self, raw: i32) -> Result<i32> {
        usize::try_from(raw)
            .ok()
            .and_then(|i| facts::BLOCKS.get(i))
            .copied()
            .filter(|&id| id >= 0)
            .ok_or_else(|| Error::new("unknown legacy block ID"))
    }
    fn block_entity_kind(&self, name: &str) -> Result<i32> {
        bootstrap::names()
            .get("minecraft:block_entity_type")
            .and_then(|r| r.id(name))
            .ok_or_else(|| Error::new("unknown current block entity type"))
    }
    fn interact_at(&self, entity: i32) -> Result<bool> {
        self.0
            .entities
            .kind(entity)
            .map(|kind| kind == 5)
            .ok_or_else(|| Error::new("unknown legacy interaction entity"))
    }
    fn contextual_block_entity(
        &self,
        pos: [i32; 3],
        action: u8,
        tag: &horizon_world::nbt::Tag,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use horizon_world::{datafix::dynamic as d, nbt::Tag};
        if !matches!(action, 4 | 5) {
            return Ok(None);
        }
        if !(0..256).contains(&pos[1]) {
            return Ok(Some(Vec::new()));
        }
        let raw = self.0.terrain.column(pos[0] >> 4, pos[2] >> 4).map(|c| {
            c.sections[(pos[1] >> 4) as usize].as_ref().map_or(0, |s| {
                s.states[(((pos[1] & 15) << 8) | ((pos[2] & 15) << 4) | (pos[0] & 15)) as usize]
            })
        });
        let Some(raw) = raw else {
            return Ok(Some(Vec::new()));
        };
        let mut block_entity = None;
        let state = if action == 4 {
            if raw >> 4 != 144 {
                return Ok(Some(Vec::new()));
            }
            let kind = d::as_int(tag.get("SkullType"), 0);
            let rotation = d::as_int(tag.get("Rot"), 0) & 15;
            if !(0..=4).contains(&kind) {
                return Err(Error::new("unsupported protocol47 skull type"));
            }
            let upgraded = common::upgrade_block_entity(pos, action, tag.clone())?;
            let current_kind = self.block_entity_kind("minecraft:skull")?;
            let facing = raw & 7;
            let index = if (2..=5).contains(&facing) {
                16 + usize::from(facing - 2)
            } else {
                rotation as usize
            };
            let state = *facts::SKULL_STATES
                .get(kind as usize)
                .and_then(|r| r.get(index))
                .ok_or_else(|| Error::new("invalid legacy skull state"))?;
            block_entity = Some(play::Clientbound::BlockEntityData(play::BlockEntityData {
                pos,
                kind: current_kind,
                tag: upgraded,
            }));
            let mut cache = self.0.skulls.borrow_mut();
            if cache.len() >= 65536 && !cache.contains_key(&pos) {
                return Err(Error::new("legacy skull cache is full"));
            }
            cache.insert(pos, (kind as u8, rotation as u8));
            drop(cache);
            state
        } else {
            if raw >> 4 != 140 {
                return Ok(Some(Vec::new()));
            }
            let item = match tag.get("Item") {
                Some(Tag::String(name)) => d::namespaced(&name.to_string_lossy()),
                value => {
                    let id = d::as_int(value, 0);
                    if id == 0 {
                        "minecraft:air".into()
                    } else {
                        i16::try_from(id)
                            .ok()
                            .and_then(old_ids::old_item_name)
                            .ok_or_else(|| Error::new("unknown legacy flower-pot item"))?
                    }
                }
            };
            let data = d::as_int(tag.get("Data"), 0);
            let state = facts::POTS
                .iter()
                .find(|&&(name, meta, _)| name == item && meta == data)
                .map(|v| v.2)
                .ok_or_else(|| Error::new("unsupported protocol47 flower-pot contents"))?;
            let mut cache = self.0.pots.borrow_mut();
            if cache.len() >= 65536 && !cache.contains_key(&pos) {
                return Err(Error::new("legacy flower-pot cache is full"));
            }
            cache.insert(pos, state);
            state
        };
        let mut packets = vec![play::Clientbound::BlockUpdate(play::BlockUpdate {
            pos,
            state,
        })];
        packets.extend(block_entity);
        Ok(Some(packets))
    }
}

impl worldfx::Context for Context<'_> {
    fn registry(&self, registry: &str, name: &str) -> Result<i32> {
        presentation::Context::registry_id(self, registry, name)
    }
    fn block(&self, raw: u16) -> Result<i32> {
        mapped_block(raw, [0; 3], &HashMap::new()).map(|b| b.id)
    }
    fn item(&self, id: i16, damage: i16) -> Result<crate::item::Stack> {
        crate::item::v1_20_3::from_numeric47_slot(
            crate::item::item47::Raw47Slot {
                id,
                count: 1,
                damage,
                tag: horizon_world::nbt::Tag::End,
            },
            old_ids::old_item_name,
        )
    }
    fn modifier_id(&self, uuid: u128) -> String {
        crate::item::v1_20_5::modifier_id(uuid, "")
    }
    fn random_seed(&self) -> i64 {
        // Local visual RNG: server47 sends no client RNG state.
        let next = self.0.random_state.get().wrapping_add(0x9e3779b97f4a7c15);
        self.0.random_state.set(next);
        let mut z = next;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (z ^ (z >> 31)) as i64
    }
}
impl worldfx_extra::Context for Context<'_> {
    fn raw_block(&self, pos: [i32; 3]) -> Result<u16> {
        let raw = self.0.terrain.raw_state(pos).unwrap_or(0); // Old client EmptyChunk is air.
        mapped_block(raw, pos, &self.0.skulls.borrow())?;
        Ok(raw)
    }
    fn listener_position(&self) -> Result<[f64; 3]> {
        if let Some(camera) = self.0.camera
            && self.0.join.as_ref().is_some_and(|j| j.entity != camera)
        {
            return self
                .0
                .entities
                .position(camera)
                .ok_or_else(|| Error::new("legacy sound references unknown camera entity"));
        }
        self.0
            .listener
            .ok_or_else(|| Error::new("legacy global sound before player position"))
    }
}

impl presentation::Context for Context<'_> {
    fn text(&self, r: &mut Reader<'_>) -> Result<horizon_world::nbt::Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn registry_id(&self, registry: &str, name: &str) -> Result<i32> {
        bootstrap::names()
            .get(registry)
            .and_then(|r| r.id(name))
            .ok_or_else(|| Error::new(format!("unknown registry value {registry}/{name}")))
    }
}

impl inventory::Context for Context<'_> {
    fn entity_kind(&self, entity: i32) -> Result<i32> {
        self.0
            .entities
            .kind(entity)
            .ok_or_else(|| Error::new("inventory for unknown legacy entity"))
    }
    fn text(&self, r: &mut Reader<'_>) -> Result<horizon_world::nbt::Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<crate::item::Stack>> {
        crate::item::v1_20_3::read_numeric47_stack(r, old_ids::old_item_name)
    }
    fn try_write_old_stack(
        &self,
        stack: Option<&crate::item::Stack>,
        w: &mut Writer,
    ) -> Result<()> {
        crate::item::v1_20_3::try_write_numeric47_stack(stack, w)
    }
    fn try_write_creative_stack(
        &self,
        stack: Option<&crate::item::Stack>,
        w: &mut Writer,
    ) -> Result<()> {
        crate::item::v1_20_3::try_write_numeric47_creative_stack(stack, w)
    }
}

fn custom_payload(payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut r = Reader::new(payload);
    let channel = r.string(32767)?;
    if channel != "minecraft:brand" {
        return Ok(None);
    }
    let mut w = Writer::new();
    w.string("MC|Brand");
    w.bytes(r.rest());
    Ok(Some(wire(0x17, &w.data)))
}

fn mapped_contextual(
    raw: u16,
    pos: [i32; 3],
    skulls: &HashMap<[i32; 3], (u8, u8)>,
    pots: &HashMap<[i32; 3], i32>,
    neighbor: impl Fn([i32; 3]) -> Option<u16>,
) -> Result<world::Block> {
    let mut block = mapped_block(raw, pos, skulls)?;
    if raw >> 4 == 140
        && let Some(&id) = pots.get(&pos)
    {
        block.id = id;
    }
    if neighbors47::needs_context(raw) {
        let descriptor = neighbors47::derive(raw, pos, neighbor, lookup47::base)?
            .unwrap_or(neighbors47::temporary_base(raw, lookup47::base)?);
        block.id = i32::try_from(lookup47::resolve(&descriptor)?)
            .map_err(|_| Error::new("legacy contextual state out of range"))?;
    }
    Ok(block)
}

pub fn mapped_block(
    raw: u16,
    pos: [i32; 3],
    skulls: &HashMap<[i32; 3], (u8, u8)>,
) -> Result<world::Block> {
    let id = old_ids::STATES
        .get(usize::from(raw))
        .copied()
        .ok_or_else(|| Error::new("legacy block state exceeds table"))?;
    let id = if id == -2 {
        let (kind, rot) = skulls.get(&pos).copied().unwrap_or((0, 0));
        let facing = raw & 7;
        let index = if (2..=5).contains(&facing) {
            16 + usize::from(facing - 2)
        } else {
            usize::from(rot & 15)
        };
        *facts::SKULL_STATES
            .get(usize::from(kind))
            .and_then(|row| row.get(index))
            .ok_or_else(|| Error::new("unknown legacy skull type"))?
    } else if id < 0 {
        return Err(Error::new(format!("unknown legacy block state {raw}")));
    } else {
        id
    };
    Ok(world::Block {
        id,
        air: raw >> 4 == 0,
        fluid: matches!(raw >> 4, 8..=11),
        sky_opacity: old_ids::OPACITY[usize::from(raw >> 4)],
    })
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod integrated_world_tests;

#[cfg(test)]
mod legacy47_acceleration_transport_tests {
    use super::*;
    #[test]
    fn actual_vanilla_fireball_extras_are_paired_with_spawn_and_consumed_once() {
        // Actual checksummed vanilla47 fk writers, with data1 and shorts800/-1600/2400.
        for (id, hex) in [
            (
                8063,
                "ff3e3f0000002800000800ffffffb0e040000000010320f9c00960",
            ),
            (
                8064,
                "803f400000002800000800ffffffb0e040000000010320f9c00960",
            ),
            (
                8066,
                "823f420000002800000800ffffffb0e040000000010320f9c00960",
            ),
        ] {
            let bytes: Vec<u8> = hex
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
                .collect();
            let mut bridge = Bridge::default();
            let packets = bridge.play(14, &bytes).unwrap();
            let spawn = packets
                .iter()
                .find_map(|packet| match packet {
                    play::Clientbound::AddEntity(p) => Some(p),
                    _ => None,
                })
                .unwrap();
            assert_eq!((spawn.id, spawn.velocity), (id, [0.0; 3]));
            assert_eq!(bridge.take_projectile_acceleration(id + 100), None);
            assert_eq!(
                bridge.take_projectile_acceleration(id),
                Some([0.1, -0.2, 0.3])
            );
            assert_eq!(bridge.take_projectile_acceleration(id), None);
        }
    }
}

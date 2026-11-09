//! Protocol340 entity framing. Metadata is decoded by its actual serializer before semantic mapping.
use super::{boolean, bridge::Context, facts, finish, inventory::Context as _};
use crate::{
    Error, Result,
    buf::Reader,
    entity_data::{Particle, ParticleData, Value, serializer as s},
    packets::play,
};
use horizon_world::{datafix::dynamic as d, nbt::Tag};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone)]
struct Record {
    name: &'static str,
    spawn: play::AddEntity,
    extra: Extra,
}
#[derive(Clone)]
struct Extra {
    mob_flags: i8,
    display_state: i32,
    custom_display: bool,
    cloud_color: i32,
    cloud_particle: i32,
    cloud_params: [i32; 2],
}
impl Default for Extra {
    fn default() -> Self {
        Self {
            mob_flags: 0,
            display_state: 0,
            custom_display: false,
            cloud_color: 0,
            cloud_particle: 15,
            cloud_params: [0; 2],
        }
    }
}
#[derive(Default)]
pub struct State {
    records: HashMap<i32, Record>,
    local: Option<i32>,
    mount_items: HashMap<(i32, u8), Option<crate::item::Stack>>,
    acceleration: HashMap<i32, [f64; 3]>,
}

fn kind(name: &str) -> Result<i32> {
    super::names()
        .get("minecraft:entity_type")
        .and_then(|v| v.id(name))
        .ok_or_else(|| Error::new(format!("missing current entity {name}")))
}
fn entity(r: &mut Reader<'_>) -> Result<i32> {
    let id = r.varint()?;
    if id < 0 {
        Err(Error::new("negative protocol340 entity id"))
    } else {
        Ok(id)
    }
}
fn finite(v: f32) -> Result<f32> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::new("nonfinite protocol340 entity float"))
    }
}
fn vec3(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    let p = [r.f64()?, r.f64()?, r.f64()?];
    if p.iter().any(|v| !v.is_finite()) {
        Err(Error::new("nonfinite protocol340 entity position"))
    } else {
        Ok(p)
    }
}
fn velocity(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    Ok([
        f64::from(r.i16()?) / 8000.,
        f64::from(r.i16()?) / 8000.,
        f64::from(r.i16()?) / 8000.,
    ])
}

impl State {
    pub fn local(&mut self, id: i32) {
        self.local = Some(id);
    }
    pub fn clear(&mut self) {
        self.records.clear();
        self.mount_items.clear();
        self.acceleration.clear();
    }
    pub fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        self.acceleration.remove(&id)
    }
    pub fn remember_mount(&mut self, update: super::inventory::MountUpdate) -> Result<()> {
        if update.old_slot > 1 || self.kind(update.entity).is_none() {
            return Err(Error::new("protocol340 inventory for unknown mount"));
        }
        self.mount_items.insert(
            (update.entity, if update.old_slot == 0 { 7 } else { 6 }),
            update.item,
        );
        Ok(())
    }
    fn preserve_mount_items(&mut self, packets: &mut [play::Clientbound]) {
        for p in packets {
            if let play::Clientbound::SetEquipment(p) = p {
                for (slot, item) in &mut p.slots {
                    if let Some(original) = self.mount_items.get(&(p.id, *slot)) {
                        if original.as_ref().map(|s| s.item) == item.as_ref().map(|s| s.item) {
                            *item = original.clone();
                        } else {
                            self.mount_items.remove(&(p.id, *slot));
                        }
                    }
                }
            }
        }
    }
    pub fn kind(&self, id: i32) -> Option<i32> {
        if self.local == Some(id) {
            Some(159)
        } else {
            self.records.get(&id).map(|v| v.spawn.kind)
        }
    }
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        cx: &Context<'_>,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let out = match id {
            0x04 => {
                let id = entity(&mut r)?;
                let uuid = r.uuid()?;
                let title = r.string(13)?;
                let pos = r.i64()?;
                let direction = r.u8()?;
                finish(&r)?;
                let mut w = crate::buf::Writer::new();
                w.varint(id);
                w.string(&title);
                w.i64(pos);
                w.u8(direction);
                let p = crate::adapter::v1_8_9::entity::objects::painting(
                    &w.data,
                    uuid,
                    |registry, name| {
                        super::names()
                            .get(registry)
                            .and_then(|v| v.id(name))
                            .ok_or_else(|| Error::new("missing painting variant"))
                    },
                )?;
                if self.records.len() >= 65536 || self.records.contains_key(&id) {
                    return Err(Error::new("duplicate/excessive protocol340 entity"));
                }
                self.records.insert(
                    id,
                    Record {
                        name: "minecraft:painting",
                        spawn: p.spawn.entity.clone(),
                        extra: Default::default(),
                    },
                );
                return Ok(Some(vec![
                    C::AddEntity(p.spawn.entity),
                    C::SetEntityData(play::SetEntityData {
                        id,
                        values: p.values,
                    }),
                ]));
            }
            0x00 | 0x03 | 0x05 => {
                let entity = entity(&mut r)?;
                let uuid = r.uuid()?;
                let mut object_type = None;
                let (name, pinned) = if id == 0x05 {
                    ("minecraft:player", 159)
                } else if id == 0x03 {
                    facts::entity(r.varint()?)
                        .ok_or_else(|| Error::new("unknown protocol340 mob registry id"))?
                } else {
                    let raw = r.u8()?;
                    object_type = Some(raw);
                    let name = object(raw)?;
                    (name, kind(name)?)
                };
                let position = vec3(&mut r)?;
                let (pitch, yaw, head_yaw) = if id == 0x05 {
                    let yaw = r.i8()?;
                    let pitch = r.i8()?;
                    (pitch, yaw, yaw)
                } else if id == 0x03 {
                    let yaw = r.i8()?;
                    let pitch = r.i8()?;
                    let head = r.i8()?;
                    (pitch, yaw, head)
                } else {
                    let pitch = r.i8()?;
                    let yaw = r.i8()?;
                    (pitch, yaw, yaw)
                };
                let mut data = if id == 0x00 { r.i32()? } else { 0 };
                let mut motion = if id == 0x05 {
                    [0.; 3]
                } else {
                    velocity(&mut r)?
                };
                let acceleration = if matches!(object_type, Some(63 | 64 | 66 | 93)) {
                    if motion.iter().all(|v| *v == 0.) {
                        return Err(Error::new("undefined protocol340 fireball acceleration"));
                    }
                    let value = motion;
                    motion = [0.; 3];
                    Some(value)
                } else {
                    None
                };
                if object_type == Some(71) {
                    data = crate::adapter::v1_8_9::entity::objects::horizontal(data)?;
                    motion = [0.; 3];
                }
                let name = if id == 0x00 && name == "minecraft:minecart" {
                    match data {
                        0 => name,
                        1 => "minecraft:chest_minecart",
                        2 => "minecraft:furnace_minecart",
                        3 => "minecraft:tnt_minecart",
                        4 => "minecraft:spawner_minecart",
                        5 => "minecraft:hopper_minecart",
                        6 => "minecraft:command_block_minecart",
                        _ => return Err(Error::new("unknown protocol340 minecart kind")),
                    }
                } else {
                    name
                };
                let pinned = if name == "minecraft:falling_block" {
                    let raw = (((data & 4095) << 4) | ((data >> 12) & 15)) as usize;
                    data = *facts::STATES
                        .get(raw)
                        .filter(|v| **v >= 0)
                        .ok_or_else(|| Error::new("unmapped protocol340 falling block"))?;
                    pinned
                } else if name.ends_with("minecart") {
                    kind(name)?
                } else {
                    pinned
                };
                let spawn = play::AddEntity {
                    id: entity,
                    uuid,
                    kind: pinned,
                    position,
                    velocity: motion,
                    pitch,
                    yaw,
                    head_yaw,
                    data,
                };
                let mut record = Record {
                    name,
                    spawn,
                    extra: Default::default(),
                };
                let mut out = vec![C::AddEntity(record.spawn.clone())];
                if id != 0x00 {
                    let meta = metadata(&mut r, &mut record, cx)?;
                    out[0] = C::AddEntity(record.spawn.clone());
                    out.extend(meta);
                }
                if object_type == Some(71) {
                    out.push(C::SetEntityData(play::SetEntityData {
                        id: entity,
                        values: vec![(8, s::DIRECTION, Value::Id(data))],
                    }));
                }
                if self.records.contains_key(&entity) || self.records.len() >= 65536 {
                    return Err(Error::new("duplicate/excessive protocol340 entities"));
                }
                finish(&r)?;
                self.records.insert(entity, record);
                if let Some(value) = acceleration {
                    self.acceleration.insert(entity, value);
                }
                return Ok(Some(out));
            }
            0x01 => {
                let id = entity(&mut r)?;
                let position = vec3(&mut r)?;
                let amount = r.i16()?;
                if amount < 0 {
                    return Err(Error::new("negative experience orb amount"));
                }
                let uuid = crate::crypto::offline_uuid(&format!("Horizon340Orb:{id}"));
                vec![C::AddEntity(play::AddEntity {
                    id,
                    uuid,
                    kind: kind("minecraft:experience_orb")?,
                    position,
                    velocity: [0.; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: i32::from(amount),
                })]
            }
            0x02 => {
                let id = entity(&mut r)?;
                if r.u8()? != 1 {
                    return Err(Error::new("unknown protocol340 global entity"));
                }
                let position = vec3(&mut r)?;
                vec![C::AddEntity(play::AddEntity {
                    id,
                    uuid: crate::crypto::offline_uuid(&format!("Horizon340Lightning:{id}")),
                    kind: kind("minecraft:lightning_bolt")?,
                    position,
                    velocity: [0.; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                })]
            }
            0x06 => {
                let id = entity(&mut r)?;
                let action = r.u8()?;
                vec![C::Animate(play::Animate { id, action })]
            }
            0x1b => {
                let id = r.i32()?;
                let event = r.i8()?;
                vec![C::EntityEvent(play::EntityEvent { id, event })]
            }
            0x25 => {
                entity(&mut r)?;
                vec![]
            }
            0x26..=0x28 => {
                let entity = entity(&mut r)?;
                let delta = if id != 0x28 {
                    Some([r.i16()?, r.i16()?, r.i16()?])
                } else {
                    None
                };
                let rotation = if id != 0x26 {
                    Some((r.i8()?, r.i8()?))
                } else {
                    None
                };
                let on_ground = boolean(&mut r)?;
                if let Some(record) = self.records.get_mut(&entity) {
                    if let Some(delta) = delta {
                        for (k, v) in delta.iter().enumerate() {
                            record.spawn.position[k] += f64::from(*v) / 4096.;
                        }
                    }
                    if let Some((yaw, pitch)) = rotation {
                        record.spawn.yaw = yaw;
                        record.spawn.pitch = pitch;
                    }
                }
                let p = play::MoveEntity {
                    id: entity,
                    delta: delta.map(play::VecDelta::Linear),
                    rotation,
                    on_ground,
                };
                vec![match id {
                    0x26 => C::MoveEntityPos(play::MoveEntityPos(p)),
                    0x27 => C::MoveEntityPosRot(play::MoveEntityPosRot(p)),
                    _ => C::MoveEntityRot(play::MoveEntityRot(p)),
                }]
            }
            0x32 => {
                let n = r.count(65536)?;
                let mut ids = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    ids.push(entity(&mut r)?);
                }
                finish(&r)?;
                for id in &ids {
                    self.records.remove(id);
                    self.acceleration.remove(id);
                }
                self.mount_items
                    .retain(|(id, _), _| self.records.contains_key(id));
                return Ok(Some(vec![C::RemoveEntities(play::RemoveEntities { ids })]));
            }
            0x36 => {
                let id = entity(&mut r)?;
                let head_yaw = r.i8()?;
                if let Some(e) = self.records.get_mut(&id) {
                    e.spawn.head_yaw = head_yaw;
                }
                vec![C::RotateHead(play::RotateHead { id, head_yaw })]
            }
            0x3c => {
                let id = entity(&mut r)?;
                let mut record = if let Some(v) = self.records.get(&id) {
                    v.clone()
                } else if self.local == Some(id) {
                    Record {
                        name: "minecraft:player",
                        extra: Default::default(),
                        spawn: play::AddEntity {
                            id,
                            uuid: 0,
                            kind: 159,
                            position: [0.; 3],
                            velocity: [0.; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        },
                    }
                } else {
                    return Err(Error::new("protocol340 metadata for unknown entity"));
                };
                let old_kind = record.spawn.kind;
                let mut out = metadata(&mut r, &mut record, cx)?;
                self.preserve_mount_items(&mut out);
                finish(&r)?;
                if record.spawn.kind != old_kind {
                    out.insert(0, C::AddEntity(record.spawn.clone()));
                    out.insert(0, C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
                }
                if self.local != Some(id) {
                    self.records.insert(id, record);
                }
                return Ok(Some(out));
            }
            0x3e => {
                let id = entity(&mut r)?;
                let velocity = velocity(&mut r)?;
                if let Some(e) = self.records.get_mut(&id) {
                    e.spawn.velocity = velocity;
                }
                vec![C::SetEntityMotion(play::SetEntityMotion { id, velocity })]
            }
            0x3d => {
                let source = r.i32()?;
                let dest = r.i32()?;
                if source < 0 || dest < -1 || source == dest {
                    return Err(Error::new("invalid protocol340 leash attachment"));
                }
                vec![C::SetEntityLink(play::SetEntityLink {
                    source,
                    dest: if dest == -1 { 0 } else { dest },
                })]
            }
            0x30 => {
                let id = entity(&mut r)?;
                let pos = crate::adapter::v1_8_9::common::position_up(r.i64()?);
                vec![C::SetEntityData(play::SetEntityData {
                    id,
                    values: vec![
                        (6, s::POSE, Value::Id(2)),
                        (14, s::OPTIONAL_BLOCK_POS, Value::OptPos(Some(pos))),
                    ],
                })]
            }
            0x3f => {
                let id = entity(&mut r)?;
                let raw = r.varint()?;
                let slot = match raw {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    4 => 4,
                    5 => 5,
                    _ => return Err(Error::new("invalid protocol340 equipment slot")),
                };
                let item = cx.read_old_slot(&mut r)?;
                vec![C::SetEquipment(play::SetEquipment {
                    id,
                    slots: vec![(slot, item)],
                })]
            }
            0x43 => {
                let vehicle = entity(&mut r)?;
                let n = r.count(65536)?;
                let mut passengers = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    passengers.push(entity(&mut r)?);
                }
                vec![C::SetPassengers(play::SetPassengers {
                    vehicle,
                    passengers,
                })]
            }
            0x4b => {
                let item = entity(&mut r)?;
                let player = entity(&mut r)?;
                let amount = r.varint()?;
                if amount < 0 {
                    return Err(Error::new("negative protocol340 collect count"));
                }
                vec![C::TakeItemEntity(play::TakeItemEntity {
                    item,
                    player,
                    amount,
                })]
            }
            0x4c => {
                let id = entity(&mut r)?;
                let position = vec3(&mut r)?;
                let yaw = f32::from(r.i8()?) * 360. / 256.;
                let pitch = f32::from(r.i8()?) * 360. / 256.;
                let on_ground = boolean(&mut r)?;
                if let Some(e) = self.records.get_mut(&id) {
                    e.spawn.position = position;
                }
                vec![C::TeleportEntity(play::TeleportEntity {
                    id,
                    position,
                    delta: [0.; 3],
                    yaw,
                    pitch,
                    relatives: 0,
                    on_ground,
                })]
            }
            _ => return Ok(None),
        };
        finish(&r)?;
        Ok(Some(out))
    }
}

fn object(id: u8) -> Result<&'static str> {
    Ok(match id {
        1 => "minecraft:oak_boat",
        2 => "minecraft:item",
        3 => "minecraft:area_effect_cloud",
        10 => "minecraft:minecart",
        50 => "minecraft:tnt",
        51 => "minecraft:end_crystal",
        60 => "minecraft:arrow",
        61 => "minecraft:snowball",
        62 => "minecraft:egg",
        63 => "minecraft:fireball",
        64 => "minecraft:small_fireball",
        65 => "minecraft:ender_pearl",
        66 => "minecraft:wither_skull",
        67 => "minecraft:shulker_bullet",
        68 => "minecraft:llama_spit",
        70 => "minecraft:falling_block",
        71 => "minecraft:item_frame",
        72 => "minecraft:eye_of_ender",
        73 => "minecraft:splash_potion",
        75 => "minecraft:experience_bottle",
        76 => "minecraft:firework_rocket",
        77 => "minecraft:leash_knot",
        78 => "minecraft:armor_stand",
        79 => "minecraft:evoker_fangs",
        90 => "minecraft:fishing_bobber",
        91 => "minecraft:spectral_arrow",
        93 => "minecraft:dragon_fireball",
        _ => return Err(Error::new("unknown protocol340 object type")),
    })
}

#[derive(Clone)]
enum Meta {
    Value(i32, Value),
    Nbt(Tag),
}
fn read_meta(r: &mut Reader<'_>, cx: &Context<'_>) -> Result<(i32, Meta)> {
    let ty = r.varint()?;
    let out = match ty {
        0 => (s::BYTE, Value::Byte(r.i8()?)),
        1 => (s::INT, Value::Int(r.varint()?)),
        2 => (s::FLOAT, Value::Float(finite(r.f32()?)?)),
        3 => (s::STRING, Value::String(r.string(32767)?)),
        4 => (
            s::COMPONENT,
            Value::Text(crate::adapter::read_legacy_component(r)?),
        ),
        5 => (s::ITEM_STACK, Value::Item(cx.read_old_slot(r)?)),
        6 => (s::BOOLEAN, Value::Bool(boolean(r)?)),
        7 => {
            let v = vec![finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?];
            (s::ROTATIONS, Value::Floats(v))
        }
        8 => (
            s::BLOCK_POS,
            Value::Pos(crate::adapter::v1_8_9::common::position_up(r.i64()?)),
        ),
        9 => {
            let v = if boolean(r)? {
                Some(crate::adapter::v1_8_9::common::position_up(r.i64()?))
            } else {
                None
            };
            (s::OPTIONAL_BLOCK_POS, Value::OptPos(v))
        }
        10 => (s::DIRECTION, Value::Id(r.varint()?)),
        11 => {
            let v = if boolean(r)? { Some(r.uuid()?) } else { None };
            (s::OPTIONAL_LIVING_ENTITY_REFERENCE, Value::OptUuid(v))
        }
        12 => {
            let raw = r.varint()?;
            let mapped = if raw == 0 {
                0
            } else {
                *facts::STATES
                    .get(raw as usize)
                    .filter(|v| **v >= 0)
                    .ok_or_else(|| Error::new("unknown protocol340 metadata block state"))?
            };
            (s::OPTIONAL_BLOCK_STATE, Value::Id(mapped))
        }
        13 => return Ok((ty, Meta::Nbt(r.named_nbt()?))),
        _ => return Err(Error::new("unknown protocol340 metadata serializer")),
    };
    Ok((ty, Meta::Value(out.0, out.1)))
}
fn cloud_particle(raw: i32, args: [i32; 2]) -> Result<Particle> {
    let mut w = crate::buf::Writer::new();
    w.i32(raw);
    w.bool(false);
    for _ in 0..7 {
        w.f32(0.);
    }
    w.i32(1);
    match raw {
        36 => {
            w.varint(args[0]);
            w.varint(args[1]);
        }
        37 | 38 | 46 => w.varint(args[0]),
        _ => {}
    }
    let out = super::worldfx::clientbound(0x22, &w.data)?
        .ok_or_else(|| Error::new("unrecognized protocol340 cloud particle"))?;
    out.packets
        .into_iter()
        .find_map(|p| {
            if let play::Clientbound::LevelParticles(p) = p {
                Some(p.particle)
            } else {
                None
            }
        })
        .ok_or_else(|| Error::new("unsupported protocol340 cloud cosmetic particle"))
}

fn metadata(
    r: &mut Reader<'_>,
    record: &mut Record,
    cx: &Context<'_>,
) -> Result<Vec<play::Clientbound>> {
    let name = record.name;
    let living = facts::living(name);
    let player = name == "minecraft:player";
    let mut values = Vec::new();
    let mut equipment = Vec::new();
    let mut seen = BTreeSet::new();
    let mut ocelot_variant = None;
    let mut display_changed = false;
    let mut cloud_changed = false;
    let mut extra_packets = Vec::new();
    loop {
        let index = r.u8()?;
        if index == 255 {
            break;
        }
        if !seen.insert(index) {
            return Err(Error::new("duplicate protocol340 metadata index"));
        }
        let (old_serializer, meta) = read_meta(r, cx)?;
        let schema_name = if name.ends_with("_boat") {
            "minecraft:oak_boat"
        } else {
            name
        };
        if let Some(expected) = facts::metadata_type(schema_name, index)
            && expected != old_serializer
        {
            return Err(Error::new(
                "protocol340 metadata type differs from vanilla field schema",
            ));
        }
        if let Meta::Nbt(tag) = meta {
            if player && matches!(index, 15 | 16) {
                let variant = if matches!(tag, Tag::Compound(_))
                    && d::as_str(tag.get("id")).is_some_and(|v| v.ends_with("parrot"))
                {
                    d::as_int(tag.get("Variant"), 0) + 1
                } else {
                    0
                };
                values.push((index + 4, s::OPTIONAL_UNSIGNED_INT, Value::Id(variant)));
                continue;
            }
            return Err(Error::new("unsupported protocol340 NBT metadata field"));
        }
        let Meta::Value(serializer, value) = meta else {
            unreachable!()
        };
        if name.ends_with("minecart") && matches!(index, 9 | 11) {
            match (index, value) {
                (9, Value::Int(v)) => record.extra.display_state = v,
                (11, Value::Bool(v)) => record.extra.custom_display = v,
                _ => return Err(Error::new("invalid protocol340 minecart display metadata")),
            }
            display_changed = true;
            continue;
        }
        if name == "minecraft:area_effect_cloud" && matches!(index, 7 | 9..=11) {
            let Value::Int(v) = value else {
                return Err(Error::new("invalid protocol340 cloud metadata"));
            };
            match index {
                7 => record.extra.cloud_color = v,
                9 => record.extra.cloud_particle = v,
                10 => record.extra.cloud_params[0] = v,
                11 => record.extra.cloud_params[1] = v,
                _ => unreachable!(),
            }
            cloud_changed = true;
            continue;
        }
        let mapped = if index <= 5 {
            if index == 2 {
                let Value::String(text) = value else {
                    return Err(Error::new("invalid protocol340 custom name type"));
                };
                values.push((
                    2,
                    s::OPTIONAL_COMPONENT,
                    Value::OptText((!text.is_empty()).then(|| Tag::String(text.as_str().into()))),
                ));
                continue;
            }
            if index == 0
                && player
                && let Value::Byte(flags) = &value
            {
                values.push((
                    6,
                    s::POSE,
                    Value::Id(if (*flags as u8) & 0x80 != 0 {
                        1
                    } else if flags & 2 != 0 {
                        5
                    } else {
                        0
                    }),
                ));
            }
            index
        } else if living && index <= 10 {
            if index == 8 {
                let Value::Int(color) = value else {
                    return Err(Error::new("invalid protocol340 effect color"));
                };
                let particles = if color == 0 {
                    vec![]
                } else {
                    vec![Particle {
                        id: 28,
                        data: Some(ParticleData::Color(color | 0xff000000u32 as i32)),
                    }]
                };
                values.push((10, s::PARTICLES, Value::Particles(particles)));
                continue;
            }
            index + 2
        } else if player {
            match index {
                11 => 17,
                12 => 18,
                13 => 16,
                14 => {
                    let Value::Byte(v) = value else {
                        return Err(Error::new("invalid protocol340 main arm"));
                    };
                    if !(0..=1).contains(&v) {
                        return Err(Error::new("invalid protocol340 main arm"));
                    }
                    values.push((15, s::HUMANOID_ARM, Value::Id(i32::from(v))));
                    continue;
                }
                _ => return Err(Error::new("unknown protocol340 player metadata")),
            }
        } else if living && index == 11 && name != "minecraft:armor_stand" {
            let Value::Byte(flags) = &value else {
                return Err(Error::new("invalid protocol340 mob flags"));
            };
            record.extra.mob_flags = *flags;
            15
        } else {
            match (name, index) {
                ("minecraft:item", 6) | ("minecraft:firework_rocket", 6) => 8,
                ("minecraft:splash_potion", 6) => {
                    if let Value::Item(Some(stack)) = &value {
                        let lingering = super::names()
                            .get("minecraft:item")
                            .and_then(|v| v.name(stack.item))
                            .is_some_and(|name| name == "minecraft:lingering_potion");
                        record.spawn.kind = kind(if lingering {
                            "minecraft:lingering_potion"
                        } else {
                            "minecraft:splash_potion"
                        })?;
                    }
                    8
                }
                ("minecraft:item_frame", 6) => 9,
                ("minecraft:item_frame", 7) => 10,
                ("minecraft:arrow" | "minecraft:spectral_arrow", 6) => 8,
                ("minecraft:arrow", 7) => 11,
                (
                    "minecraft:cow"
                    | "minecraft:chicken"
                    | "minecraft:pig"
                    | "minecraft:sheep"
                    | "minecraft:wolf"
                    | "minecraft:ocelot"
                    | "minecraft:horse"
                    | "minecraft:donkey"
                    | "minecraft:mule"
                    | "minecraft:skeleton_horse"
                    | "minecraft:zombie_horse"
                    | "minecraft:llama"
                    | "minecraft:parrot"
                    | "minecraft:rabbit"
                    | "minecraft:polar_bear"
                    | "minecraft:villager"
                    | "minecraft:mooshroom",
                    12,
                ) => 16,
                ("minecraft:sheep", 13) => 18,
                ("minecraft:pig", 13) => {
                    let Value::Bool(saddled) = value else {
                        return Err(Error::new("invalid protocol340 saddle metadata"));
                    };
                    equipment.push((7, saddled.then(|| crate::item::Stack::new(949, 1))));
                    continue;
                }
                ("minecraft:pig", 14) => 18,
                ("minecraft:polar_bear", 13) => 18,
                ("minecraft:rabbit", 13) => 18,
                ("minecraft:creeper", 12) => 16,
                ("minecraft:creeper", 13 | 14) => index + 4,
                (
                    "minecraft:zombie"
                    | "minecraft:husk"
                    | "minecraft:zombie_villager"
                    | "minecraft:zombified_piglin",
                    12,
                ) => 16,
                (
                    "minecraft:zombie"
                    | "minecraft:husk"
                    | "minecraft:zombie_villager"
                    | "minecraft:zombified_piglin",
                    13,
                ) => {
                    if !matches!(value, Value::Int(0)) {
                        return Err(Error::new(
                            "unsupported protocol340 obsolete zombie subtype",
                        ));
                    }
                    continue;
                }
                (
                    "minecraft:zombie"
                    | "minecraft:husk"
                    | "minecraft:zombie_villager"
                    | "minecraft:zombified_piglin",
                    14,
                ) => {
                    let Value::Bool(raised) = value else {
                        return Err(Error::new("invalid protocol340 zombie arm flag"));
                    };
                    record.extra.mob_flags =
                        (record.extra.mob_flags & !4) | if raised { 4 } else { 0 };
                    values.push((15, s::BYTE, Value::Byte(record.extra.mob_flags)));
                    continue;
                }
                ("minecraft:skeleton" | "minecraft:wither_skeleton" | "minecraft:stray", 12) => {
                    let Value::Bool(raised) = value else {
                        return Err(Error::new("invalid protocol340 skeleton arm flag"));
                    };
                    record.extra.mob_flags =
                        (record.extra.mob_flags & !4) | if raised { 4 } else { 0 };
                    values.push((15, s::BYTE, Value::Byte(record.extra.mob_flags)));
                    continue;
                }
                (
                    "minecraft:bat"
                    | "minecraft:spider"
                    | "minecraft:cave_spider"
                    | "minecraft:silverfish"
                    | "minecraft:endermite"
                    | "minecraft:blaze"
                    | "minecraft:ghast"
                    | "minecraft:snow_golem"
                    | "minecraft:iron_golem",
                    12,
                ) => 16,
                ("minecraft:witch", 12) => 17,
                ("minecraft:slime" | "minecraft:magma_cube", 12) => 18,
                ("minecraft:armor_stand", 11) => 15,
                ("minecraft:armor_stand", 12..=17) => index + 4,
                ("minecraft:enderman", 12) => 16,
                ("minecraft:enderman", 13) => 17,
                ("minecraft:guardian" | "minecraft:elder_guardian", 12) => 16,
                ("minecraft:guardian" | "minecraft:elder_guardian", 13) => 17,
                ("minecraft:wither", 12..=15) => index + 4,
                ("minecraft:vex", 12) => 16,
                ("minecraft:evoker" | "minecraft:illusioner" | "minecraft:vindicator", 12) => {
                    let Value::Byte(flags) = value else {
                        return Err(Error::new("invalid protocol340 illager flags"));
                    };
                    record.extra.mob_flags = (record.extra.mob_flags & !4) | ((flags & 1) << 2);
                    values.push((15, s::BYTE, Value::Byte(record.extra.mob_flags)));
                    continue;
                }
                ("minecraft:evoker" | "minecraft:illusioner", 13) => 17,
                ("minecraft:shulker", 12) => 16,
                ("minecraft:shulker", 13) => {
                    let Value::OptPos(pos) = value else {
                        return Err(Error::new("invalid protocol340 shulker attachment"));
                    };
                    if let Some(pos) = pos {
                        let position = [
                            f64::from(pos[0]) + 0.5,
                            f64::from(pos[1]),
                            f64::from(pos[2]) + 0.5,
                        ];
                        record.spawn.position = position;
                        extra_packets.push(play::Clientbound::TeleportEntity(
                            play::TeleportEntity {
                                id: record.spawn.id,
                                position,
                                delta: [0.; 3],
                                yaw: f32::from(record.spawn.yaw) * 360. / 256.,
                                pitch: f32::from(record.spawn.pitch) * 360. / 256.,
                                relatives: 0,
                                on_ground: false,
                            },
                        ));
                    }
                    continue;
                }
                ("minecraft:shulker", 14) => 17,
                ("minecraft:shulker", 15) => 18,
                ("minecraft:zombie_villager", 15) => 19,
                ("minecraft:zombie_villager", 16) => {
                    let Value::Int(profession) = value else {
                        return Err(Error::new("invalid protocol340 zombie villager profession"));
                    };
                    let profession = match profession {
                        0 => 5,
                        1 => 9,
                        2 => 4,
                        3 => 1,
                        4 => 2,
                        5 => 11,
                        _ => {
                            return Err(Error::new(
                                "unknown protocol340 zombie villager profession",
                            ));
                        }
                    };
                    values.push((20, s::VILLAGER_DATA, Value::Ids(vec![2, profession, 1])));
                    continue;
                }
                ("minecraft:firework_rocket", 7) => {
                    let Value::Int(id) = value else {
                        return Err(Error::new("invalid protocol340 firework target"));
                    };
                    if id < 0 || id == i32::MAX {
                        return Err(Error::new("invalid protocol340 firework target"));
                    }
                    values.push((
                        9,
                        s::OPTIONAL_UNSIGNED_INT,
                        Value::Id(if id == 0 { 0 } else { id + 1 }),
                    ));
                    continue;
                }
                ("minecraft:fishing_bobber", 6) => 8,
                ("minecraft:wither_skull", 6) => 8,
                ("minecraft:area_effect_cloud", 6) => 8,
                ("minecraft:area_effect_cloud", 8) => 9,
                (name, 6..=8) if name.ends_with("minecart") => index + 2,
                (name, 10) if name.ends_with("minecart") => 12,
                ("minecraft:furnace_minecart", 12) => 13,
                ("minecraft:command_block_minecart", 12) => 13,
                ("minecraft:command_block_minecart", 13) => 14,
                (
                    "minecraft:horse"
                    | "minecraft:donkey"
                    | "minecraft:mule"
                    | "minecraft:skeleton_horse"
                    | "minecraft:zombie_horse"
                    | "minecraft:llama",
                    13,
                ) => {
                    let Value::Byte(flags) = value else {
                        return Err(Error::new("invalid protocol340 horse flags"));
                    };
                    values.push((18, s::BYTE, Value::Byte(flags & !4)));
                    equipment.push((7, (flags & 4 != 0).then(|| crate::item::Stack::new(949, 1))));
                    continue;
                }
                (
                    "minecraft:horse"
                    | "minecraft:donkey"
                    | "minecraft:mule"
                    | "minecraft:skeleton_horse"
                    | "minecraft:zombie_horse"
                    | "minecraft:llama",
                    14,
                ) => {
                    continue;
                }
                ("minecraft:horse", 15) => 19,
                ("minecraft:horse", 16) => {
                    let Value::Int(armor) = value else {
                        return Err(Error::new("invalid protocol340 horse armor"));
                    };
                    let item = match armor {
                        0 => None,
                        1 => Some(crate::item::Stack::new(1407, 1)),
                        2 => Some(crate::item::Stack::new(1408, 1)),
                        3 => Some(crate::item::Stack::new(1409, 1)),
                        _ => return Err(Error::new("unknown protocol340 horse armor")),
                    };
                    equipment.push((6, item));
                    continue;
                }
                ("minecraft:donkey" | "minecraft:mule" | "minecraft:llama", 15) => 19,
                ("minecraft:llama", 16) => 20,
                ("minecraft:llama", 17) => {
                    let Value::Int(color) = value else {
                        return Err(Error::new("invalid protocol340 llama carpet"));
                    };
                    let item = if color == -1 {
                        None
                    } else {
                        let color = usize::try_from(color)
                            .ok()
                            .and_then(|i| {
                                [
                                    "white",
                                    "orange",
                                    "magenta",
                                    "light_blue",
                                    "yellow",
                                    "lime",
                                    "pink",
                                    "gray",
                                    "light_gray",
                                    "cyan",
                                    "purple",
                                    "blue",
                                    "brown",
                                    "green",
                                    "red",
                                    "black",
                                ]
                                .get(i)
                            })
                            .ok_or_else(|| Error::new("invalid protocol340 llama carpet color"))?;
                        let name = format!("minecraft:{color}_carpet");
                        let id = super::names()
                            .get("minecraft:item")
                            .and_then(|v| v.id(&name))
                            .ok_or_else(|| Error::new("current llama carpet item unavailable"))?;
                        Some(crate::item::Stack::new(id, 1))
                    };
                    equipment.push((6, item));
                    continue;
                }
                ("minecraft:llama", 18) => 21,
                ("minecraft:wolf", 13) => {
                    let Value::Byte(flags) = value else {
                        return Err(Error::new("invalid protocol340 wolf flags"));
                    };
                    values.push((18, s::BYTE, Value::Byte(flags & 5)));
                    values.push((
                        22,
                        s::LONG,
                        Value::Long(if flags & 2 != 0 { i64::MAX } else { 0 }),
                    ));
                    continue;
                }
                ("minecraft:ocelot" | "minecraft:parrot", 13) => 18,
                ("minecraft:wolf" | "minecraft:ocelot" | "minecraft:parrot", 14) => 19,
                ("minecraft:wolf", 15) => {
                    continue;
                }
                ("minecraft:wolf", 16) => 20,
                ("minecraft:wolf", 17) => 21,
                ("minecraft:parrot", 15) => 20,
                ("minecraft:ocelot", 15) => {
                    let Value::Int(variant) = value else {
                        return Err(Error::new("invalid protocol340 cat variant"));
                    };
                    if !(0..=3).contains(&variant) {
                        return Err(Error::new("unknown protocol340 cat variant"));
                    }
                    ocelot_variant = Some(variant);
                    continue;
                }
                ("minecraft:villager", 13) => {
                    let Value::Int(profession) = value else {
                        return Err(Error::new("invalid protocol340 villager profession"));
                    };
                    let profession = match profession {
                        0 => 5,
                        1 => 9,
                        2 => 4,
                        3 => 1,
                        4 => 2,
                        5 => 11,
                        _ => return Err(Error::new("unknown protocol340 villager profession")),
                    };
                    values.push((19, s::VILLAGER_DATA, Value::Ids(vec![2, profession, 1])));
                    continue;
                }
                (
                    "minecraft:oak_boat"
                    | "minecraft:spruce_boat"
                    | "minecraft:birch_boat"
                    | "minecraft:jungle_boat"
                    | "minecraft:acacia_boat"
                    | "minecraft:dark_oak_boat",
                    6..=8,
                ) => index + 2,
                (
                    "minecraft:oak_boat"
                    | "minecraft:spruce_boat"
                    | "minecraft:birch_boat"
                    | "minecraft:jungle_boat"
                    | "minecraft:acacia_boat"
                    | "minecraft:dark_oak_boat",
                    9,
                ) => {
                    let Value::Int(wood) = value else {
                        return Err(Error::new("invalid protocol340 boat wood"));
                    };
                    let name = match wood {
                        0 => "minecraft:oak_boat",
                        1 => "minecraft:spruce_boat",
                        2 => "minecraft:birch_boat",
                        3 => "minecraft:jungle_boat",
                        4 => "minecraft:acacia_boat",
                        5 => "minecraft:dark_oak_boat",
                        _ => return Err(Error::new("unknown protocol340 boat wood")),
                    };
                    record.name = name;
                    record.spawn.kind = kind(name)?;
                    continue;
                }
                (
                    "minecraft:oak_boat"
                    | "minecraft:spruce_boat"
                    | "minecraft:birch_boat"
                    | "minecraft:jungle_boat"
                    | "minecraft:acacia_boat"
                    | "minecraft:dark_oak_boat",
                    10 | 11,
                ) => index + 1,
                ("minecraft:tnt", 6) => 8,
                ("minecraft:end_crystal", 6 | 7) => index + 2,
                ("minecraft:falling_block", 6) => 8,
                ("minecraft:ender_dragon", 12) => 16,
                _ => {
                    return Err(Error::new(format!(
                        "unsupported protocol340 metadata {name} index{index}"
                    )));
                }
            }
        };
        values.push((mapped, serializer, value));
    }
    if name == "minecraft:ocelot" {
        if let Some(variant) = ocelot_variant {
            if variant == 0 {
                values.retain(|v| !matches!(v.0, 18 | 19));
                values.push((18, s::BOOLEAN, Value::Bool(false)));
            } else {
                record.spawn.kind = kind("minecraft:cat")?;
                let name = ["minecraft:black", "minecraft:red", "minecraft:siamese"]
                    [(variant - 1) as usize];
                let id = super::names()
                    .get("minecraft:cat_variant")
                    .and_then(|v| v.id(name))
                    .ok_or_else(|| Error::new("missing old cat variant"))?;
                values.push((20, s::CAT_VARIANT, Value::Id(id)));
            }
        } else if record.spawn.kind != kind("minecraft:cat")? {
            values.retain(|v| !matches!(v.0, 18 | 19));
        }
    }
    if display_changed {
        let raw = record.extra.display_state;
        let current = if record.extra.custom_display {
            if !(0..=65535).contains(&raw) {
                return Err(Error::new("invalid protocol340 minecart block"));
            }
            let raw = (((raw & 4095) << 4) | ((raw >> 12) & 15)) as usize;
            *facts::STATES
                .get(raw)
                .filter(|v| **v >= 0)
                .ok_or_else(|| Error::new("unmapped protocol340 minecart block"))?
        } else {
            0
        };
        values.push((11, s::OPTIONAL_BLOCK_STATE, Value::Id(current)));
    }
    if cloud_changed {
        let particle = if matches!(record.extra.cloud_particle, 15 | 16) {
            Particle {
                id: 28,
                data: Some(ParticleData::Color(
                    record.extra.cloud_color | 0xff000000u32 as i32,
                )),
            }
        } else {
            cloud_particle(record.extra.cloud_particle, record.extra.cloud_params)?
        };
        values.push((10, s::PARTICLE, Value::Particle(particle)));
    }
    let values: std::collections::BTreeMap<_, _> =
        values.into_iter().map(|(i, s, v)| (i, (s, v))).collect();
    let values = values.into_iter().map(|(i, (s, v))| (i, s, v)).collect();
    let mut out = extra_packets;
    out.push(play::Clientbound::SetEntityData(play::SetEntityData {
        id: record.spawn.id,
        values,
    }));
    if !equipment.is_empty() {
        out.push(play::Clientbound::SetEquipment(play::SetEquipment {
            id: record.spawn.id,
            slots: equipment,
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::Writer;
    fn record(name: &'static str) -> Record {
        Record {
            name,
            extra: Default::default(),
            spawn: play::AddEntity {
                id: 7,
                uuid: 11,
                kind: kind(name).unwrap(),
                position: [1., 2., 3.],
                velocity: [0.; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            },
        }
    }

    #[test]
    fn player_arm_and_shoulders_use_actual777_accessor_ids() {
        let bridge = super::super::Bridge::default();
        let mut player = record("minecraft:player");
        let mut w = Writer::new();
        w.u8(14);
        w.varint(0);
        w.i8(1);
        for index in [15, 16] {
            w.u8(index);
            w.varint(13);
            w.named_nbt(&Tag::Compound(Default::default()));
        }
        w.u8(255);
        let out = metadata(&mut Reader::new(&w.data), &mut player, &Context(&bridge)).unwrap();
        let play::Clientbound::SetEntityData(p) = &out[0] else {
            panic!("metadata expected")
        };
        assert!(p.values.contains(&(15, 42, Value::Id(1))));
        assert!(p.values.contains(&(19, 19, Value::Id(0))));
        assert!(p.values.contains(&(20, 19, Value::Id(0))));
    }

    #[test]
    fn sheep_color_uses_actual_field_types_and_rejects_wrong_base_type() {
        let bridge = super::super::Bridge::default();
        let mut sheep = record("minecraft:sheep");
        let out = metadata(
            &mut Reader::new(&[13, 0, 0x1e, 255]),
            &mut sheep,
            &Context(&bridge),
        )
        .unwrap();
        let play::Clientbound::SetEntityData(p) = &out[0] else {
            panic!("metadata expected")
        };
        assert_eq!(p.values, vec![(18, 0, Value::Byte(30))]);
        assert!(
            metadata(
                &mut Reader::new(&[0, 1, 0, 255]),
                &mut sheep,
                &Context(&bridge)
            )
            .is_err()
        );
        assert!(
            metadata(
                &mut Reader::new(&[13, 0, 1, 13, 0, 2, 255]),
                &mut sheep,
                &Context(&bridge)
            )
            .is_err()
        );
    }
}

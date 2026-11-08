//! 754 entity framing with metadata indices measured in both official JVMs.
use super::*;
use crate::entity_data::{Particle, ParticleData, Value, serializer as s};
#[path = "runtime_fields.rs"]
mod fields;

#[derive(Clone)]
struct Entity {
    source: usize,
    spawn: play::AddEntity,
    metadata: Vec<(u8, i32, Value)>,
    passengers: Vec<i32>,
    color: i32,
    ambient: bool,
    cloud_particle: Option<Particle>,
    display: (i32, bool),
}
#[derive(Default)]
pub(super) struct Entities {
    entities: HashMap<i32, Entity>,
    acceleration: HashMap<i32, [f64; 3]>,
}
fn xyz(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    let v = crate::buf::Vec3d::read(r)?;
    if v.iter().any(|x| !x.is_finite()) {
        Err(Error::new("nonfinite old entity position"))
    } else {
        Ok(v)
    }
}
fn velocity(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    Ok([
        f64::from(r.i16()?) / 8000.0,
        f64::from(r.i16()?) / 8000.0,
        f64::from(r.i16()?) / 8000.0,
    ])
}
fn uuid() -> Result<u128> {
    let mut bytes = [0; 16];
    crate::crypto::random_bytes(&mut bytes)?;
    Ok(u128::from_be_bytes(bytes))
}
fn current_entity(name: &str) -> Result<i32> {
    names()
        .get("minecraft:entity_type")
        .and_then(|ids| ids.id(name))
        .ok_or_else(|| Error::new("missing native entity type"))
}
fn item(name: &str) -> Result<crate::item::Stack> {
    let raw = tables::ITEM_NAMES
        .iter()
        .position(|v| *v == name)
        .ok_or_else(|| Error::new("missing old equipment item"))?;
    Ok(crate::item::Stack::new(up(tables::ITEM_UP, raw as i32)?, 1))
}

impl Entities {
    pub(super) fn needs_interaction_point(&self, id: i32) -> bool {
        self.entities
            .get(&id)
            .and_then(|entity| tables::ENTITY_NAMES.get(entity.source))
            == Some(&"minecraft:armor_stand")
    }

    pub(super) fn clear(&mut self) {
        self.entities.clear();
        self.acceleration.clear();
    }
    pub(super) fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        self.acceleration.remove(&id)
    }
    pub(super) fn register_player(&mut self, id: i32) {
        let source = tables::ENTITY_NAMES
            .iter()
            .position(|v| *v == "minecraft:player")
            .unwrap();
        self.entities.insert(
            id,
            Entity {
                source,
                spawn: play::AddEntity {
                    id,
                    uuid: 0,
                    kind: 159,
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                },
                metadata: vec![],
                passengers: vec![],
                color: 0,
                ambient: false,
                cloud_particle: None,
                display: (0, false),
            },
        );
    }
    fn insert(&mut self, source: usize, spawn: play::AddEntity) -> Result<()> {
        if spawn.id < 0 || self.entities.len() >= 16384 && !self.entities.contains_key(&spawn.id) {
            return Err(Error::new("old entity cache limit exceeded"));
        }
        self.entities.insert(
            spawn.id,
            Entity {
                source,
                spawn,
                metadata: vec![],
                passengers: vec![],
                color: 0,
                ambient: false,
                cloud_particle: None,
                display: (0, false),
            },
        );
        Ok(())
    }
    pub(super) fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let mut out = vec![];
        match id {
            0 | 2 | 4 => {
                let entity = r.varint()?;
                let uuid = r.uuid()?;
                let source = if id == 4 {
                    tables::ENTITY_NAMES
                        .iter()
                        .position(|v| *v == "minecraft:player")
                        .unwrap()
                } else {
                    usize::try_from(r.varint()?)
                        .map_err(|_| Error::new("negative old entity type"))?
                };
                let kind = if tables::ENTITY_NAMES.get(source) == Some(&"minecraft:potion") {
                    current_entity("minecraft:splash_potion")?
                } else {
                    up(tables::ENTITY_UP, source as i32)?
                };
                let position = xyz(&mut r)?;
                let (pitch, yaw, head_yaw, data, velocity) = if id == 0 {
                    let pitch = r.i8()?;
                    let yaw = r.i8()?;
                    let mut data = r.i32()?;
                    if tables::ENTITY_NAMES[source] == "minecraft:falling_block" {
                        data = up(tables::STATE_UP, data)?;
                    }
                    (pitch, yaw, yaw, data, velocity(&mut r)?)
                } else if id == 2 {
                    let yaw = r.i8()?;
                    let pitch = r.i8()?;
                    let head = r.i8()?;
                    (pitch, yaw, head, 0, velocity(&mut r)?)
                } else {
                    let yaw = r.i8()?;
                    let pitch = r.i8()?;
                    (pitch, yaw, yaw, 0, [0.0; 3])
                };
                finish(&r)?;
                let mut spawn = play::AddEntity {
                    id: entity,
                    uuid,
                    kind,
                    position,
                    velocity,
                    pitch,
                    yaw,
                    head_yaw,
                    data,
                };
                let acceleration = (id == 0
                    && matches!(
                        tables::ENTITY_NAMES[source],
                        "minecraft:fireball"
                            | "minecraft:small_fireball"
                            | "minecraft:dragon_fireball"
                            | "minecraft:wither_skull"
                    ))
                .then_some(spawn.velocity);
                if acceleration.is_some() {
                    spawn.velocity = [0.0; 3];
                }
                self.insert(source, spawn.clone())?;
                if let Some(acceleration) = acceleration {
                    self.acceleration.insert(entity, acceleration);
                }
                out.push(C::AddEntity(spawn));
            }
            1 => {
                let entity = r.varint()?;
                let position = xyz(&mut r)?;
                let value = i32::from(r.i16()?);
                let source = tables::ENTITY_NAMES
                    .iter()
                    .position(|v| *v == "minecraft:experience_orb")
                    .unwrap();
                let spawn = play::AddEntity {
                    id: entity,
                    uuid: uuid()?,
                    kind: up(tables::ENTITY_UP, source as i32)?,
                    position,
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                };
                self.insert(source, spawn.clone())?;
                out.push(C::AddEntity(spawn));
                out.push(C::SetEntityData(play::SetEntityData {
                    id: entity,
                    values: vec![(8, s::INT, Value::Int(value))],
                }));
            }
            3 => {
                let entity = r.varint()?;
                let uuid = r.uuid()?;
                let raw = r.varint()?;
                let motive_name = usize::try_from(raw)
                    .ok()
                    .and_then(|i| tables::PAINTING_NAMES.get(i))
                    .ok_or_else(|| Error::new("unknown old painting motive"))?;
                let motive = super::super::v1_20_1_ids::PAINTING_VARIANT_NAMES
                    .iter()
                    .position(|name| name == motive_name)
                    .ok_or_else(|| Error::new("old painting lacks native local variant"))?
                    as i32
                    + 1;
                let pos = crate::buf::Pos::read(&mut r)?;
                let direction = r.u8()?;
                if direction > 3 {
                    return Err(Error::new("invalid old painting direction"));
                }
                let source = tables::ENTITY_NAMES
                    .iter()
                    .position(|v| *v == "minecraft:painting")
                    .unwrap();
                let spawn = play::AddEntity {
                    id: entity,
                    uuid,
                    kind: up(tables::ENTITY_UP, source as i32)?,
                    position: [pos[0] as f64, pos[1] as f64, pos[2] as f64],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: (direction as i8).wrapping_mul(64),
                    head_yaw: 0,
                    data: match direction {
                        0 => 3,
                        1 => 4,
                        2 => 2,
                        3 => 5,
                        _ => unreachable!(),
                    },
                };
                self.insert(source, spawn.clone())?;
                out.push(C::AddEntity(spawn));
                out.push(C::SetEntityData(play::SetEntityData {
                    id: entity,
                    values: vec![
                        (
                            8,
                            s::DIRECTION,
                            Value::Id(match direction {
                                0 => 3,
                                1 => 4,
                                2 => 2,
                                3 => 5,
                                _ => unreachable!(),
                            }),
                        ),
                        (9, s::PAINTING_VARIANT, Value::Id(motive)),
                    ],
                }));
            }
            5 => out.push(C::Animate(play::Animate::read(&mut r)?)),
            39..=41 => {
                let entity = r.varint()?;
                let delta = if id != 41 {
                    Some(play::VecDelta::Linear([r.i16()?, r.i16()?, r.i16()?]))
                } else {
                    None
                };
                let rotation = if id != 39 {
                    Some((r.i8()?, r.i8()?))
                } else {
                    None
                };
                let on_ground = boolean(&mut r)?;
                if let Some(cached) = self.entities.get_mut(&entity) {
                    if let Some(delta) = &delta {
                        cached.spawn.position = delta.end(cached.spawn.position);
                    }
                    if let Some((yaw, pitch)) = rotation {
                        cached.spawn.yaw = yaw;
                        cached.spawn.pitch = pitch;
                    }
                }
                let movement = play::MoveEntity {
                    id: entity,
                    delta,
                    rotation,
                    on_ground,
                };
                out.push(match id {
                    39 => C::MoveEntityPos(play::MoveEntityPos(movement)),
                    40 => C::MoveEntityPosRot(play::MoveEntityPosRot(movement)),
                    _ => C::MoveEntityRot(play::MoveEntityRot(movement)),
                });
            }
            42 => {
                r.varint()?;
            }
            54 => {
                let n = r.count(16384)?;
                let mut ids = vec![];
                for _ in 0..n {
                    let id = r.varint()?;
                    self.entities.remove(&id);
                    self.acceleration.remove(&id);
                    ids.push(id);
                }
                out.push(C::RemoveEntities(play::RemoveEntities { ids }));
            }
            55 => {
                let id = r.varint()?;
                let effect = up(tables::MOB_EFFECT_UP, i32::from(r.u8()?))?;
                out.push(C::RemoveMobEffect(play::RemoveMobEffect { id, effect }));
            }
            58 => out.push(C::RotateHead(play::RotateHead::read(&mut r)?)),
            68 => {
                let id = r.varint()?;
                let entity = self
                    .entities
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| Error::new("old metadata precedes entity spawn"))?;
                let mut next = entity.clone();
                let mut values = vec![];
                let mut extra = vec![];
                let mut seen = std::collections::BTreeSet::new();
                loop {
                    let index = r.u8()?;
                    if index == 255 {
                        break;
                    }
                    if !seen.insert(index) {
                        return Err(Error::new("duplicate old metadata index"));
                    }
                    let serializer = r.varint()?;
                    let value = read_value(serializer, &mut r)?;
                    if let Some((_, target, expected, new_serializer)) = fields::METADATA
                        [entity.source]
                        .iter()
                        .find(|f| f.0 == index)
                    {
                        if serializer != *expected {
                            return Err(Error::new(
                                "old metadata serializer disagrees with measured field",
                            ));
                        }
                        let value = if serializer == 14 {
                            let Value::Text(tag) = value else {
                                unreachable!()
                            };
                            let parrot = matches!(tag.get("id"),Some(Tag::String(id)) if id.to_string_lossy()=="minecraft:parrot");
                            let variant =
                                horizon_world::datafix::dynamic::as_int(tag.get("Variant"), 0)
                                    .clamp(0, 4);
                            Value::Id(if parrot { variant + 1 } else { 0 })
                        } else if *new_serializer == 42 {
                            let Value::Byte(n) = value else {
                                return Err(Error::new("invalid old arm type"));
                            };
                            Value::Id(i32::from(n))
                        } else {
                            value
                        };
                        if tables::ENTITY_NAMES[entity.source] == "minecraft:potion" && index == 7 {
                            if let Value::Item(Some(stack)) = &value {
                                let lingering = tables::ITEM_NAMES
                                    .iter()
                                    .position(|v| *v == "minecraft:lingering_potion")
                                    .map(|i| tables::ITEM_UP[i]);
                                next.spawn.kind =
                                    current_entity(if Some(stack.item) == lingering {
                                        "minecraft:lingering_potion"
                                    } else {
                                        "minecraft:splash_potion"
                                    })?;
                            }
                        }
                        if tables::ENTITY_NAMES[entity.source] == "minecraft:area_effect_cloud"
                            && index == 10
                            && let Value::Particle(particle) = &value
                        {
                            next.cloud_particle = Some(particle.clone());
                        }
                        if index == 16
                            && fields::REMOVED[entity.source]
                                .iter()
                                .any(|field| field.2 == "DATA_ID_OWNER_UUID")
                        {
                            let Value::Byte(flags) = value else {
                                return Err(Error::new("invalid old horse flags"));
                            };
                            extra.push(C::SetEquipment(play::SetEquipment {
                                id,
                                slots: vec![(
                                    7,
                                    if flags & 4 != 0 {
                                        Some(item("minecraft:saddle")?)
                                    } else {
                                        None
                                    },
                                )],
                            }));
                            values.push((*target, *new_serializer, Value::Byte(flags & !4)));
                        } else if fields::LIVING[entity.source] && index == 10 {
                            if let Value::Bool(ambient) = value {
                                next.ambient = ambient;
                                values.push((*target, *new_serializer, Value::Bool(ambient)));
                            } else {
                                return Err(Error::new("invalid effect ambience"));
                            }
                        } else {
                            values.push((*target, *new_serializer, value));
                        }
                    } else if let Some((_, expected, name)) =
                        fields::REMOVED[entity.source].iter().find(|f| f.0 == index)
                    {
                        if serializer != *expected {
                            return Err(Error::new("old removed metadata serializer mismatch"));
                        }
                        match (*name, value) {
                            ("DATA_EFFECT_COLOR_ID", Value::Int(color)) => next.color = color,
                            ("DATA_COLOR", Value::Int(color)) => next.color = color,
                            ("DATA_ID_TYPE", Value::Int(wood)) => {
                                if !(0..6).contains(&wood) {
                                    return Err(Error::new("invalid 1.16.5 boat wood"));
                                }
                                next.spawn.kind =
                                    super::super::translate::v1_21_1::boats::TYPES[wood as usize];
                            }
                            ("DATA_TYPE_ID", Value::Int(variant))
                                if tables::ENTITY_NAMES[entity.source] == "minecraft:cat" =>
                            {
                                if variant < 0 || variant as usize >= CAT_VARIANTS.len() {
                                    return Err(Error::new("invalid old cat variant"));
                                }
                                values.push((20, s::CAT_VARIANT, Value::Id(variant)));
                            }
                            ("DATA_SWAG_ID", Value::Int(color)) => {
                                let colors = [
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
                                ];
                                let stack = if color < 0 {
                                    None
                                } else {
                                    let color = colors.get(color as usize).ok_or_else(|| {
                                        Error::new("invalid old llama carpet colour")
                                    })?;
                                    Some(item(&format!("minecraft:{color}_carpet"))?)
                                };
                                extra.push(C::SetEquipment(play::SetEquipment {
                                    id,
                                    slots: vec![(6, stack)],
                                }));
                            }
                            ("DATA_ID_DISPLAY_BLOCK", Value::Int(block)) => {
                                next.display.0 = up(tables::STATE_UP, block)?
                            }
                            ("DATA_ID_CUSTOM_DISPLAY", Value::Bool(custom)) => {
                                next.display.1 = custom
                            }
                            ("DATA_SADDLE_ID", Value::Bool(saddled)) => {
                                extra.push(C::SetEquipment(play::SetEquipment {
                                    id,
                                    slots: vec![(
                                        7,
                                        if saddled {
                                            Some(item("minecraft:saddle")?)
                                        } else {
                                            None
                                        },
                                    )],
                                }))
                            }
                            ("DATA_ID_FLAGS", Value::Byte(flags)) => {
                                values.push((18, s::BYTE, Value::Byte(flags & !4)))
                            }
                            (
                                "DATA_REMAINING_ANGER_TIME"
                                | "TREASURE_POS"
                                | "HOME_POS"
                                | "TRAVEL_POS"
                                | "GOING_HOME"
                                | "TRAVELLING"
                                | "DATA_ID_OWNER_UUID"
                                | "DATA_ATTACH_POS_ID",
                                _,
                            ) => {}
                            (name, _) => {
                                return Err(Error::new(format!(
                                    "unsupported 1.16.5 entity field {name}"
                                )));
                            }
                        }
                    } else {
                        return Err(Error::new(
                            "old metadata index outside declared class hierarchy",
                        ));
                    }
                }
                finish(&r)?;
                if fields::LIVING[entity.source] {
                    let particles = if next.color == 0 {
                        vec![]
                    } else {
                        vec![Particle {
                            id: 28,
                            data: Some(ParticleData::Color(
                                (if next.ambient { 38_i32 } else { 255_i32 } << 24)
                                    | (next.color & 0xffffff),
                            )),
                        }]
                    };
                    values.push((10, s::PARTICLES, Value::Particles(particles)));
                }
                if tables::ENTITY_NAMES[entity.source] == "minecraft:area_effect_cloud" {
                    let mut particle = next.cloud_particle.clone().unwrap_or(Particle {
                        id: 28,
                        data: Some(ParticleData::Color(-1)),
                    });
                    if particle.id == 28 {
                        let alpha = match particle.data {
                            Some(ParticleData::Color(color)) => color & !0xffffff,
                            _ => -0x1000000,
                        };
                        particle.data = Some(ParticleData::Color(alpha | (next.color & 0xffffff)));
                    }
                    values.retain(|entry| entry.0 != 10);
                    values.push((10, s::PARTICLE, Value::Particle(particle)));
                }
                if tables::ENTITY_NAMES[entity.source].ends_with("minecart") {
                    values.push((
                        11,
                        s::OPTIONAL_BLOCK_STATE,
                        Value::Id(if next.display.1 { next.display.0 } else { 0 }),
                    ));
                }
                for value in &values {
                    next.metadata.retain(|v| v.0 != value.0);
                    next.metadata.push(value.clone());
                }
                if next.spawn.kind != entity.spawn.kind {
                    out.push(C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
                    out.push(C::AddEntity(next.spawn.clone()));
                    out.push(C::SetEntityData(play::SetEntityData {
                        id,
                        values: next.metadata.clone(),
                    }));
                    if !next.passengers.is_empty() {
                        out.push(C::SetPassengers(play::SetPassengers {
                            vehicle: id,
                            passengers: next.passengers.clone(),
                        }));
                    }
                } else {
                    out.push(C::SetEntityData(play::SetEntityData { id, values }));
                }
                out.extend(extra);
                self.entities.insert(id, next);
            }
            70 => {
                let id = r.varint()?;
                let velocity = velocity(&mut r)?;
                if let Some(e) = self.entities.get_mut(&id) {
                    e.spawn.velocity = velocity;
                }
                out.push(C::SetEntityMotion(play::SetEntityMotion { id, velocity }));
            }
            71 => {
                let id = r.varint()?;
                let mut slots = vec![];
                let mut seen = 0u8;
                loop {
                    let slot = r.u8()?;
                    let raw = slot & 127;
                    if raw > 5 || seen & (1 << raw) != 0 {
                        return Err(Error::new("invalid old equipment slot"));
                    }
                    seen |= 1 << raw;
                    let raw = if raw == 4
                        && self
                            .entities
                            .get(&id)
                            .is_some_and(|e| tables::ENTITY_NAMES[e.source] == "minecraft:horse")
                    {
                        6
                    } else {
                        raw
                    };
                    slots.push((raw, read_slot(&mut r)?));
                    if slot & 128 == 0 {
                        break;
                    }
                }
                out.push(C::SetEquipment(play::SetEquipment { id, slots }));
            }
            75 => {
                let p = play::SetPassengers::read(&mut r)?;
                if let Some(entity) = self.entities.get_mut(&p.vehicle) {
                    entity.passengers = p.passengers.clone();
                }
                out.push(C::SetPassengers(p));
            }
            86 => {
                let id = r.varint()?;
                let position = xyz(&mut r)?;
                let yaw = f32::from(r.i8()?) * 360.0 / 256.0;
                let pitch = f32::from(r.i8()?) * 360.0 / 256.0;
                let on_ground = boolean(&mut r)?;
                if let Some(e) = self.entities.get_mut(&id) {
                    e.spawn.position = position;
                    e.spawn.yaw = (yaw * 256.0 / 360.0) as i8;
                    e.spawn.pitch = (pitch * 256.0 / 360.0) as i8;
                }
                out.push(C::TeleportEntity(play::TeleportEntity {
                    id,
                    position,
                    delta: [0.0; 3],
                    yaw,
                    pitch,
                    relatives: 0,
                    on_ground,
                }));
            }
            88 => {
                let id = r.varint()?;
                let n = r.i32()?;
                if !(0..=1024).contains(&n) {
                    return Err(Error::new("invalid old attribute count"));
                }
                let mut values = vec![];
                for _ in 0..n {
                    let name = r.string(32767)?;
                    let raw = tables::ATTRIBUTE_NAMES
                        .iter()
                        .position(|v| *v == name)
                        .ok_or_else(|| Error::new("unknown old attribute"))?;
                    let attribute = up(tables::ATTRIBUTE_UP, raw as i32)?;
                    let base = r.f64()?;
                    let n = r.count(1024)?;
                    let mut modifiers = vec![];
                    for _ in 0..n {
                        let id = r.uuid()?;
                        let amount = r.f64()?;
                        let operation = i32::from(r.u8()?);
                        if operation > 2 {
                            return Err(Error::new("invalid old attribute operation"));
                        }
                        modifiers.push(play::AttributeModifier {
                            id: format!("horizon_via:legacy_{id:032x}"),
                            amount,
                            operation,
                        });
                    }
                    values.push(play::AttributeSnapshot {
                        attribute,
                        base,
                        modifiers,
                    });
                }
                out.push(C::UpdateAttributes(play::UpdateAttributes { id, values }));
            }
            89 => {
                let id = r.varint()?;
                let effect = up(tables::MOB_EFFECT_UP, i32::from(r.u8()?))?;
                let amplifier = i32::from(r.u8()?);
                let duration = r.varint()?;
                let flags = r.i8()?;
                out.push(C::UpdateMobEffect(play::UpdateMobEffect {
                    id,
                    effect,
                    amplifier,
                    duration,
                    flags,
                }));
            }
            _ => return Ok(None),
        }
        finish(&r)?;
        Ok(Some(out))
    }
}

fn read_value(serializer: i32, r: &mut Reader<'_>) -> Result<Value> {
    Ok(match serializer {
        0 => Value::Byte(r.i8()?),
        1 => Value::Int(r.varint()?),
        2 => Value::Float(r.f32()?),
        3 => Value::String(r.string(32767)?),
        4 => Value::Text(text(r)?),
        5 => Value::OptText(if boolean(r)? { Some(text(r)?) } else { None }),
        6 => Value::Item(read_slot(r)?),
        7 => Value::Bool(boolean(r)?),
        8 => Value::Floats(vec![r.f32()?, r.f32()?, r.f32()?]),
        9 => Value::Pos(crate::buf::Pos::read(r)?),
        10 => Value::OptPos(if boolean(r)? {
            Some(crate::buf::Pos::read(r)?)
        } else {
            None
        }),
        11 => Value::Id(r.varint()?),
        12 => Value::OptUuid(if boolean(r)? { Some(r.uuid()?) } else { None }),
        13 => {
            let state = r.varint()?;
            Value::Id(if state == 0 {
                0
            } else {
                up(tables::STATE_UP, state)?
            })
        }
        14 => Value::Text(r.named_nbt()?),
        15 => Value::Particle(particle(r)?),
        16 => Value::Ids(vec![
            up(tables::VILLAGER_TYPE_UP, r.varint()?)?,
            up(tables::VILLAGER_PROFESSION_UP, r.varint()?)?,
            r.varint()?,
        ]),
        17 | 18 => Value::Id(r.varint()?),
        _ => return Err(Error::new("unknown 1.16.5 metadata serializer")),
    })
}

pub(super) fn particle(r: &mut Reader<'_>) -> Result<Particle> {
    let old = r.varint()?;
    let name = tables::PARTICLE_NAMES
        .get(usize::try_from(old).map_err(|_| Error::new("negative old particle id"))?)
        .ok_or_else(|| Error::new("unknown old particle"))?;
    let id = if *name == "minecraft:barrier" {
        names()
            .get("minecraft:particle_type")
            .and_then(|ids| ids.id("minecraft:block_marker"))
            .ok_or_else(|| Error::new("missing native block marker"))?
    } else {
        up(tables::PARTICLE_UP, old)?
    };
    let data = match *name {
        "minecraft:block" | "minecraft:falling_dust" => {
            Some(ParticleData::State(up(tables::STATE_UP, r.varint()?)?))
        }
        "minecraft:dust" => {
            let rgb = [r.f32()?, r.f32()?, r.f32()?];
            let scale = r.f32()?;
            if rgb
                .iter()
                .chain(std::iter::once(&scale))
                .any(|v| !v.is_finite())
            {
                return Err(Error::new("nonfinite old dust options"));
            }
            let channel = |v: f32| ((v * 255.0) as i32).clamp(0, 255);
            Some(ParticleData::ColorScale(
                (channel(rgb[0]) << 16) | (channel(rgb[1]) << 8) | channel(rgb[2]),
                scale,
            ))
        }
        "minecraft:item" => Some(ParticleData::Item(
            read_slot(r)?.ok_or_else(|| Error::new("empty old item particle"))?,
        )),
        "minecraft:barrier" => Some(ParticleData::State(up(tables::STATE_UP, 7540)?)),
        "minecraft:entity_effect" => Some(ParticleData::Color(-1)),
        "minecraft:ambient_entity_effect" => Some(ParticleData::Color(0x26ffffff)),
        "minecraft:flash" => Some(ParticleData::Color(-1)),
        "minecraft:effect" | "minecraft:instant_effect" => Some(ParticleData::ColorPower(-1, 1.0)),
        "minecraft:dragon_breath" => Some(ParticleData::Scalar(1.0)),
        _ => None,
    };
    Ok(Particle { id, data })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parrot_indices_follow_actual_declaring_class_hierarchy() {
        let source = tables::ENTITY_NAMES
            .iter()
            .position(|v| *v == "minecraft:parrot")
            .unwrap();
        assert!(fields::METADATA[source].contains(&(18, 20, 1, 1)));
        assert!(fields::METADATA[source].contains(&(16, 18, 0, 0)));
        assert!(fields::METADATA[source].contains(&(17, 19, 12, 13)));
    }
    #[test]
    fn entity_relative_moves_preserve_short_deltas() {
        let mut entities = Entities::default();
        let mut w = Writer::new();
        w.varint(4);
        w.i16(-4096);
        w.i16(0);
        w.i16(8192);
        w.bool(true);
        let out = entities.clientbound(39, &w.data).unwrap().unwrap();
        let play::Clientbound::MoveEntityPos(p) = &out[0] else {
            panic!()
        };
        assert_eq!(p.0.delta, Some(play::VecDelta::Linear([-4096, 0, 8192])));
    }
}

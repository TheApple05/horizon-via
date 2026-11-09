//! The object-spawn namespace and object-specific data, from the vanilla47 client handler.
use super::{Error, Result, Spawn, boolean, entity_id, finish, position, raw, semantic::Values};
use crate::{
    buf::Reader,
    entity_data::{Value, serializer as s},
    item::Stack,
    packets::play,
};
use horizon_world::nbt::Tag;

#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    /// Legacy fireballs send an acceleration vector, whereas current projectiles derive direction from motion.
    /// This vector is retained for an explicit consumer physics policy; it is not mislabeled as velocity.
    ProjectileAcceleration([f64; 3]),
}
#[derive(Debug)]
pub struct Converted {
    pub spawn: Spawn,
    pub values: Values,
    pub extras: Vec<Extra>,
}
pub fn convert(
    payload: &[u8],
    uuid: u128,
    mut block: impl FnMut(u16) -> Result<i32>,
    mut item: impl FnMut(&raw::Slot) -> Result<Stack>,
) -> Result<Converted> {
    let object = super::spawn_object(payload)?;
    let old_kind = match object.object_type {
        1 => 41,
        2 => 1,
        10 => match object.data {
            0 => 42,
            1 => 43,
            2 => 44,
            3 => 45,
            4 => 47,
            5 => 46,
            6 => 40,
            _ => return Err(Error::new("invalid protocol47 minecart subtype")),
        },
        50 => 20,
        51 => 200,
        60 => 10,
        61 => 11,
        62 => 7,
        63 => 12,
        64 => 13,
        65 => 14,
        66 => 19,
        70 => 21,
        71 => 18,
        72 => 15,
        73 => 16,
        75 => 17,
        76 => 22,
        77 => 8,
        78 => 30,
        90 => -2,
        _ => return Err(Error::new("unsupported protocol47 object type")),
    };
    let kind = super::entity_ids::pinned(old_kind)
        .ok_or_else(|| Error::new("unmapped protocol47 object entity"))?;
    let mut data = 0;
    let mut values = Vec::new();
    let mut extras = Vec::new();
    let mut velocity = object.velocity.unwrap_or([0.; 3]);
    match object.object_type {
        60 | 90 => {
            if object.data < 0 {
                return Err(Error::new("negative protocol47 projectile owner"));
            }
            data = object.data;
        }
        70 => {
            // Block.getStateById uses id in low12bits and metadata in high4; root block maps use id<<4|meta.
            let state = object.data & 0xffff;
            data = block((((state & 4095) << 4) | ((state >> 12) & 15)) as u16)?;
            values.push((
                8,
                s::BLOCK_POS,
                Value::Pos(object.position.map(|value| value.floor() as i32)),
            ));
            velocity = [0.; 3];
        }
        71 => {
            data = horizontal(object.data)?;
            values.push((8, s::DIRECTION, Value::Id(data)));
            velocity = [0.; 3];
        }
        63 | 64 | 66 => {
            extras.push(Extra::ProjectileAcceleration(velocity));
            velocity = [0.; 3];
        }
        73 => {
            if !(0..=0xffff).contains(&object.data) {
                return Err(Error::new("invalid protocol47 potion damage"));
            }
            let potion = raw::Slot {
                id: 373,
                count: 1,
                damage: (object.data | 0x4000) as i16,
                tag: Tag::End,
            };
            values.push((8, s::ITEM_STACK, Value::Item(Some(item(&potion)?))));
            velocity = [0.; 3];
        }
        77 => velocity = [0.; 3],
        _ => {}
    }
    Ok(Converted {
        spawn: Spawn {
            entity: play::AddEntity {
                id: object.id,
                uuid,
                kind,
                position: object.position,
                velocity,
                pitch: object.pitch,
                yaw: object.yaw,
                head_yaw: object.yaw,
                data,
            },
            old_kind,
            metadata: Vec::new(),
            held_item_hint: None,
        },
        values,
        extras,
    })
}
/// Old Direction.getHorizontal indices: south0, west1, north2, east3; current 3D ids:3,4,2,5.
pub fn horizontal(old: i32) -> Result<i32> {
    match old {
        0 => Ok(3),
        1 => Ok(4),
        2 => Ok(2),
        3 => Ok(5),
        _ => Err(Error::new("invalid protocol47 horizontal direction")),
    }
}

pub fn experience_orb(payload: &[u8], uuid: u128) -> Result<Converted> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let position = position(&mut r)?;
    let value = i32::from(r.i16()?);
    finish(&r)?;
    if value < 0 {
        return Err(Error::new("negative protocol47 experience value"));
    }
    Ok(Converted {
        spawn: Spawn {
            entity: play::AddEntity {
                id,
                uuid,
                kind: 50,
                position,
                velocity: [0.; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            },
            old_kind: 2,
            metadata: Vec::new(),
            held_item_hint: None,
        },
        values: vec![(8, s::INT, Value::Int(value))],
        extras: Vec::new(),
    })
}
fn old_position(value: i64) -> [i32; 3] {
    [
        (value >> 38) as i32,
        ((value << 26) >> 52) as i32,
        ((value << 38) >> 38) as i32,
    ]
}
fn painting_name(old: &str) -> Option<&'static str> {
    Some(match old {
        "Kebab" => "kebab",
        "Aztec" => "aztec",
        "Alban" => "alban",
        "Aztec2" => "aztec2",
        "Bomb" => "bomb",
        "Plant" => "plant",
        "Wasteland" => "wasteland",
        "Pool" => "pool",
        "Courbet" => "courbet",
        "Sea" => "sea",
        "Sunset" => "sunset",
        "Creebet" => "creebet",
        "Wanderer" => "wanderer",
        "Graham" => "graham",
        "Match" => "match",
        "Bust" => "bust",
        "Stage" => "stage",
        "Void" => "void",
        "SkullAndRoses" => "skull_and_roses",
        "Wither" => "wither",
        "Fighters" => "fighters",
        "Pointer" => "pointer",
        "Pigscene" => "pigscene",
        "BurningSkull" => "burning_skull",
        "Skeleton" => "skeleton",
        "DonkeyKong" => "donkey_kong",
        _ => return None,
    })
}
/// Registry callback returns the actual connection-local (unbiased) painting id.
pub fn painting(
    payload: &[u8],
    uuid: u128,
    mut variant: impl FnMut(&str, &str) -> Result<i32>,
) -> Result<Converted> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let title = r.string(13)?;
    let anchor = old_position(r.i64()?);
    let direction = horizontal(i32::from(r.u8()?))?;
    finish(&r)?;
    let name =
        painting_name(&title).ok_or_else(|| Error::new("unknown protocol47 painting title"))?;
    let variant = variant("minecraft:painting_variant", &format!("minecraft:{name}"))?
        .checked_add(1)
        .filter(|id| *id > 0)
        .ok_or_else(|| Error::new("invalid painting holder id"))?;
    Ok(Converted {
        spawn: Spawn {
            entity: play::AddEntity {
                id,
                uuid,
                kind: 94,
                position: anchor.map(f64::from),
                velocity: [0.; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: direction,
            },
            old_kind: 9,
            metadata: Vec::new(),
            held_item_hint: None,
        },
        values: vec![
            (8, s::DIRECTION, Value::Id(direction)),
            (9, s::PAINTING_VARIANT, Value::Id(variant)),
        ],
        extras: Vec::new(),
    })
}

#[derive(Debug, PartialEq)]
pub struct Attachment {
    pub entity: i32,
    pub holder: Option<i32>,
    pub leash: bool,
}
pub fn attachment(payload: &[u8]) -> Result<Attachment> {
    let mut r = Reader::new(payload);
    let entity = r.i32()?;
    let holder = r.i32()?;
    let leash = boolean(&mut r)?;
    finish(&r)?;
    if entity < 0 || holder < -1 || entity == holder {
        return Err(Error::new("invalid protocol47 entity attachment"));
    }
    Ok(Attachment {
        entity,
        holder: (holder != -1).then_some(holder),
        leash,
    })
}
pub fn destroy(payload: &[u8]) -> Result<Vec<i32>> {
    let mut r = Reader::new(payload);
    let count = r.count(r.remaining())?;
    let mut ids = Vec::with_capacity(count.min(256));
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..count {
        let id = entity_id(&mut r)?;
        if !seen.insert(id) {
            return Err(Error::new("duplicate protocol47 removal id"));
        }
        ids.push(id);
    }
    finish(&r)?;
    Ok(ids)
}

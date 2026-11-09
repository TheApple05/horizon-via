//! Original protocol47 entity bridge candidate. All output indices/type ids are pinned777.
//! Do not feed these outputs through older-protocol metadata renumbering a second time.
use crate::{Error, Result, buf::Reader, packets::play};

pub mod effects;
pub mod entity_ids;
pub mod lifecycle;
pub mod objects;
pub mod raw;
pub mod semantic;

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 entity bytes"))
    }
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid protocol47 boolean")),
    }
}
fn entity_id(r: &mut Reader<'_>) -> Result<i32> {
    let id = r.varint()?;
    if id < 0 {
        Err(Error::new("negative protocol47 entity id"))
    } else {
        Ok(id)
    }
}
fn position(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    Ok([
        f64::from(r.i32()?) / 32.0,
        f64::from(r.i32()?) / 32.0,
        f64::from(r.i32()?) / 32.0,
    ])
}
fn velocity(r: &mut Reader<'_>) -> Result<[f64; 3]> {
    Ok([
        f64::from(r.i16()?) / 8000.0,
        f64::from(r.i16()?) / 8000.0,
        f64::from(r.i16()?) / 8000.0,
    ])
}
fn metadata(r: &mut Reader<'_>, id: i32) -> Result<Vec<raw::Entry>> {
    let mut w = crate::buf::Writer::new();
    w.varint(id);
    w.bytes(r.rest());
    Ok(raw::read(&w.data)?.entries)
}

#[derive(Clone, Debug)]
pub struct Spawn {
    pub entity: play::AddEntity,
    pub old_kind: i32,
    pub metadata: Vec<raw::Entry>,
    /// Player spawn contains a base numeric held-item id only, without count, damage or NBT.
    pub held_item_hint: Option<i16>,
}
/// Protocol47 does not send a mob UUID; caller supplies a connection-local synthetic identity.
pub fn spawn_mob(payload: &[u8], supplied_uuid: u128) -> Result<Spawn> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let old_kind = i32::from(r.u8()?);
    if !entity_ids::is_living_mob(old_kind) {
        return Err(Error::new("invalid protocol47 mob type"));
    }
    let kind =
        entity_ids::pinned(old_kind).ok_or_else(|| Error::new("unmapped protocol47 mob type"))?;
    let position = position(&mut r)?;
    let yaw = r.i8()?;
    let pitch = r.i8()?;
    let head_yaw = r.i8()?;
    let velocity = velocity(&mut r)?;
    let metadata = metadata(&mut r, id)?;
    Ok(Spawn {
        entity: play::AddEntity {
            id,
            uuid: supplied_uuid,
            kind,
            position,
            velocity,
            pitch,
            yaw,
            head_yaw,
            data: 0,
        },
        old_kind,
        metadata,
        held_item_hint: None,
    })
}
pub fn spawn_player(payload: &[u8]) -> Result<Spawn> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let uuid = r.uuid()?;
    let position = position(&mut r)?;
    let yaw = r.i8()?;
    let pitch = r.i8()?;
    let held = r.i16()?;
    if held < 0 {
        return Err(Error::new("negative protocol47 held item hint"));
    }
    let metadata = metadata(&mut r, id)?;
    Ok(Spawn {
        entity: play::AddEntity {
            id,
            uuid,
            kind: 159,
            position,
            velocity: [0.0; 3],
            pitch,
            yaw,
            head_yaw: yaw,
            data: 0,
        },
        old_kind: -1,
        metadata,
        held_item_hint: (held != 0).then_some(held),
    })
}

#[derive(Debug)]
pub struct ObjectSpawn {
    pub id: i32,
    pub object_type: u8,
    pub position: [f64; 3],
    pub pitch: i8,
    pub yaw: i8,
    pub data: i32,
    pub velocity: Option<[f64; 3]>,
}
/// Object type is a different namespace from EntityList ids. Preserve data for the per-type caller.
pub fn spawn_object(payload: &[u8]) -> Result<ObjectSpawn> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let object_type = r.u8()?;
    let position = position(&mut r)?;
    let pitch = r.i8()?;
    let yaw = r.i8()?;
    let data = r.i32()?;
    let velocity = if data > 0 {
        Some(velocity(&mut r)?)
    } else {
        None
    };
    finish(&r)?;
    Ok(ObjectSpawn {
        id,
        object_type,
        position,
        pitch,
        yaw,
        data,
        velocity,
    })
}
#[derive(Debug, PartialEq)]
pub struct Equipment {
    pub entity: i32,
    /// Pinned ordinal: main hand0, feet2, legs3, chest4, head5.
    pub slot: u8,
    pub item: Option<raw::Slot>,
}
pub fn equipment(payload: &[u8]) -> Result<Equipment> {
    let mut r = Reader::new(payload);
    let entity = entity_id(&mut r)?;
    let slot = match r.i16()? {
        0 => 0,
        1 => 2,
        2 => 3,
        3 => 4,
        4 => 5,
        _ => return Err(Error::new("invalid protocol47 equipment slot")),
    };
    let (item, n) = raw::slot(&payload[r.position()..])?;
    r.bytes(n)?;
    finish(&r)?;
    Ok(Equipment { entity, slot, item })
}
#[derive(Debug, PartialEq)]
pub struct RelativeMove {
    pub entity: i32,
    /// Modern relative move delta is /4096; old signed-byte deltas are /32.
    pub delta: [i16; 3],
    pub rotation: Option<(i8, i8)>,
    pub on_ground: bool,
}
pub fn relative_move(payload: &[u8], has_rotation: bool) -> Result<RelativeMove> {
    let mut r = Reader::new(payload);
    let entity = entity_id(&mut r)?;
    let delta = [
        i16::from(r.i8()?) * 128,
        i16::from(r.i8()?) * 128,
        i16::from(r.i8()?) * 128,
    ];
    let rotation = if has_rotation {
        Some((r.i8()?, r.i8()?))
    } else {
        None
    };
    let on_ground = boolean(&mut r)?;
    finish(&r)?;
    Ok(RelativeMove {
        entity,
        delta,
        rotation,
        on_ground,
    })
}
pub fn teleport(payload: &[u8]) -> Result<play::TeleportEntity> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let position = position(&mut r)?;
    let yaw = f32::from(r.i8()?) * (360.0 / 256.0);
    let pitch = f32::from(r.i8()?) * (360.0 / 256.0);
    let on_ground = boolean(&mut r)?;
    finish(&r)?;
    Ok(play::TeleportEntity {
        id,
        position,
        delta: [0.0; 3],
        yaw,
        pitch,
        relatives: 0xe0,
        on_ground,
    })
}

pub fn head_rotation(payload: &[u8]) -> Result<(i32, i8)> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let yaw = r.i8()?;
    finish(&r)?;
    Ok((id, yaw))
}
pub fn entity_velocity(payload: &[u8]) -> Result<(i32, [f64; 3])> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let velocity = velocity(&mut r)?;
    finish(&r)?;
    Ok((id, velocity))
}

#[cfg(test)]
mod tests;

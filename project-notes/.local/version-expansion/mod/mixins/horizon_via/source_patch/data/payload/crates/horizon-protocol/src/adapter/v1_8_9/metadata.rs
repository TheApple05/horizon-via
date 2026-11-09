//! Protocol47 packed DataWatcher decoding. All eight wire kinds are kept typed.
//! Entity-specific index conversion is deliberately separate from structural decoding.
use crate::{
    Error, Result,
    buf::{NBT_QUOTA, Reader},
};
use horizon_world::nbt::{self, Tag};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(i8),
    Short(i16),
    Int(i32),
    Float(f32),
    String(String),
    Item(Option<Slot>),
    Position([i32; 3]),
    Rotation([f32; 3]),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub id: i16,
    pub count: i8,
    pub damage: i16,
    pub tag: Tag,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub index: u8,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EntityMetadata {
    pub entity: i32,
    pub entries: Vec<Entry>,
}

pub fn slot(data: &[u8]) -> Result<(Option<Slot>, usize)> {
    let mut r = Reader::new(data);
    let id = r.i16()?;
    if id == -1 {
        return Ok((None, r.position()));
    }
    if id < 0 {
        return Err(Error::new("invalid legacy numeric item id"));
    }
    let count = r.i8()?;
    let damage = r.i16()?;
    let (root, n) = nbt::read_named(
        &data[r.position()..],
        nbt::Limits {
            max_bytes: NBT_QUOTA,
            ..Default::default()
        },
    )
    .map_err(|e| Error::new(format!("legacy item NBT: {e}")))?;
    if !matches!(root.tag, Tag::End | Tag::Compound(_)) {
        return Err(Error::new("legacy item NBT is not a compound"));
    }
    r.bytes(n)?;
    if count <= 0 {
        return Err(Error::new("invalid legacy item count"));
    }
    Ok((
        Some(Slot {
            id,
            count,
            damage,
            tag: root.tag,
        }),
        r.position(),
    ))
}

pub fn read(payload: &[u8]) -> Result<EntityMetadata> {
    let mut r = Reader::new(payload);
    let entity = r.varint()?;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        let head = r.u8()?;
        if head == 127 {
            if r.remaining() != 0 {
                return Err(Error::new("trailing legacy entity metadata"));
            }
            return Ok(EntityMetadata { entity, entries });
        }
        let index = head & 31;
        if !seen.insert(index) {
            return Err(Error::new("duplicate legacy metadata index"));
        }
        let value = match head >> 5 {
            0 => Value::Byte(r.i8()?),
            1 => Value::Short(r.i16()?),
            2 => Value::Int(r.i32()?),
            3 => Value::Float(r.f32()?),
            4 => Value::String(r.string(32767)?),
            5 => {
                let (value, n) = slot(&payload[r.position()..])?;
                r.bytes(n)?;
                Value::Item(value)
            }
            6 => Value::Position([r.i32()?, r.i32()?, r.i32()?]),
            7 => Value::Rotation([r.f32()?, r.f32()?, r.f32()?]),
            _ => unreachable!(),
        };
        entries.push(Entry { index, value });
    }
}

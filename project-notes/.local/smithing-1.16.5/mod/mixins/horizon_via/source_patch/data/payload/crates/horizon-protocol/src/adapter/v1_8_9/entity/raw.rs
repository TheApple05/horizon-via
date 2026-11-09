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

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> Vec<u8> {
        let rows: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/raw-fixtures.json")).unwrap();
        let hex = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap()["hex"]
            .as_str()
            .unwrap();
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn actual_vanilla_datawatcher_preserves_all_eight_types() {
        let value = read(&fixture("metadata_all_eight_types")).unwrap();
        assert_eq!(value.entity, 123);
        assert_eq!(
            value.entries,
            vec![
                Entry {
                    index: 0,
                    value: Value::Byte(2)
                },
                Entry {
                    index: 3,
                    value: Value::Short(17)
                },
                Entry {
                    index: 4,
                    value: Value::Int(12345)
                },
                Entry {
                    index: 5,
                    value: Value::Float(1.25)
                },
                Entry {
                    index: 6,
                    value: Value::String("legacy18".into())
                },
                Entry {
                    index: 7,
                    value: Value::Item(Some(Slot {
                        id: 35,
                        count: 3,
                        damage: 14,
                        tag: Tag::End
                    }))
                },
                Entry {
                    index: 8,
                    value: Value::Position([3, 64, -9])
                },
                Entry {
                    index: 9,
                    value: Value::Rotation([1., 2., 3.])
                },
            ]
        );
    }
    #[test]
    fn actual_vanilla_slot_keeps_outer_variant_damage_and_named_nbt() {
        let empty = fixture("empty");
        assert_eq!(slot(&empty).unwrap(), (None, 2));
        let wool = fixture("red_wool");
        let (value, n) = slot(&wool).unwrap();
        assert_eq!(n, 6);
        assert_eq!(
            value.unwrap(),
            Slot {
                id: 35,
                count: 3,
                damage: 14,
                tag: Tag::End
            }
        );
        let sword = fixture("damaged_named_diamond_sword");
        let (value, n) = slot(&sword).unwrap();
        assert_eq!(n, sword.len());
        let value = value.unwrap();
        assert_eq!(value.id, 276);
        assert_eq!(value.damage, 37);
        assert_eq!(value.tag.get("Unbreakable"), Some(&Tag::Byte(1)));
        assert_eq!(
            value.tag.get("display").unwrap().get("Name"),
            Some(&Tag::String("Legacy sword".into()))
        );
    }
    #[test]
    fn malformed_truncated_duplicated_or_unterminated_metadata_is_rejected() {
        let wire = fixture("metadata_all_eight_types");
        for n in 0..wire.len() {
            assert!(read(&wire[..n]).is_err(), "length{n}");
        }
        let mut extra = wire.clone();
        extra.push(0);
        assert!(read(&extra).is_err());
        assert!(read(&[123, 0, 1, 0, 2, 127]).is_err());
    }
    #[test]
    fn nbt_length_is_bounded_by_packet_not_an_unchecked_name_or_array() {
        for n in 0..fixture("damaged_named_diamond_sword").len() {
            assert!(slot(&fixture("damaged_named_diamond_sword")[..n]).is_err());
        }
        assert!(slot(&[255, 254]).is_err());
    }
}

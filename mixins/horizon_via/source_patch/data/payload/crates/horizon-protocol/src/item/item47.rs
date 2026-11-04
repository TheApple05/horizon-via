//! Protocol 47 numeric ItemStack framing and exact original-value cache.
//!
//! Numeric ids must resolve through the server version's vanilla item registry before
//! calling the datafixer: its historical ItemIdFix omits several real 1.8 item ids.
//! This module does not guess a reverse mapping for new or modified modern components.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use horizon_world::{
    datafix::{self, dynamic as d, schema::Ty},
    nbt::{self, Tag},
};
use std::collections::{HashMap, VecDeque};

const CACHE_ENTRIES: usize = 1024;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const CACHE_ENTRY_BYTES: usize = 256 * 1024;

/// Damage is the full signed short, not a four-bit block metadata value. It also
/// encodes durability, map ids, potion flags and spawn egg species in this version.
#[derive(Clone, Debug, PartialEq)]
pub struct Raw47Slot {
    pub id: i16,
    pub count: i8,
    pub damage: i16,
    pub tag: Tag,
}

/// Reads one slot without consuming the following packet fields.
pub fn read(r: &mut Reader<'_>) -> Result<Option<Raw47Slot>> {
    let id = r.i16()?;
    if id == -1 {
        return Ok(None);
    }
    if id < 0 {
        return Err(Error::new("invalid negative protocol 47 item id"));
    }
    let count = r.i8()?;
    let damage = r.i16()?;
    let tag = r.named_nbt()?;
    if !matches!(tag, Tag::End | Tag::Compound(_)) {
        return Err(Error::new("protocol 47 item NBT is not a compound"));
    }
    Ok(Some(Raw47Slot {
        id,
        count,
        damage,
        tag,
    }))
}

/// Test boundary which also reports the complete named-NBT consumption.
#[cfg(test)]
pub fn read_slot(data: &[u8]) -> Result<(Option<Raw47Slot>, usize)> {
    let mut r = Reader::new(data);
    let slot = read(&mut r)?;
    Ok((slot, r.position()))
}

impl Raw47Slot {
    fn validate(&self) -> Result<()> {
        if self.id < 0 || self.count <= 0 {
            return Err(Error::new("invalid nonempty protocol 47 item stack"));
        }
        if !matches!(self.tag, Tag::End | Tag::Compound(_)) {
            return Err(Error::new("protocol 47 item NBT is not a compound"));
        }
        Ok(())
    }

    /// Produces current saved ItemStack NBT, ready for v1_20_3::from_tag(saved,None).
    /// Resolve the upgraded id by its current name: a raw id's base item mapping loses variants.
    pub fn upgrade(&self, old_name: impl FnOnce(i16) -> Option<String>) -> Result<Tag> {
        self.validate()?;
        let name = old_name(self.id)
            .filter(|name| name.starts_with("minecraft:"))
            .ok_or_else(|| {
                Error::new(format!("unknown protocol 47 numeric item id {}", self.id))
            })?;
        // The empty compound is significant. Early potion and spawn egg fixes inspect tag;
        // omitting it for a wire TAG_End can discard the numeric damage variant.
        let tag = match &self.tag {
            Tag::End => d::empty_map(),
            tag => tag.clone(),
        };
        let mut saved = Tag::compound([
            // Actual saved ItemStacks in 1.8 use string ids. The historical numeric-id
            // fix lacks banner425 and other items added after its old-format source.
            // Resolve only with the exact vanilla1.8 registry; a modern name is premature.
            ("id", d::string(&name)),
            ("Count", Tag::Byte(self.count)),
            ("Damage", Tag::Short(self.damage)),
            ("tag", tag),
        ]);
        datafix::update(Ty::ItemStack, &mut saved, -1, datafix::CURRENT);
        let id = d::as_str(saved.get("id"))
            .filter(|id| id.starts_with("minecraft:"))
            .ok_or_else(|| Error::new("protocol 47 item upgrade produced no current id"))?;
        if id == "minecraft:air" && self.id != 0 {
            return Err(Error::new("protocol 47 item upgrade produced air"));
        }
        if d::as_int(saved.get("count"), -1) != i32::from(self.count) {
            return Err(Error::new("protocol 47 item upgrade changed stack count"));
        }
        Ok(saved)
    }

    fn identity_eq(&self, other: &Self) -> bool {
        self.id == other.id && self.damage == other.damage && self.tag == other.tag
    }
}

/// Fallible writer: unsupported conversions must propagate through Session::serverbound.
/// Validation happens before appending anything to the caller's packet buffer.
pub fn try_write_slot(slot: Option<&Raw47Slot>, w: &mut Writer) -> Result<()> {
    let Some(slot) = slot else {
        w.i16(-1);
        return Ok(());
    };
    slot.validate()?;
    let mut encoded = Writer::new();
    encoded.i16(slot.id);
    encoded.i8(slot.count);
    encoded.i16(slot.damage);
    // Writer::named_nbt deliberately substitutes TAG_End on serialization errors.
    // This fallible boundary must preserve NBT or fail the containing packet.
    let tag = nbt::write_named(
        &nbt::NamedTag {
            name: Default::default(),
            tag: slot.tag.clone(),
        },
        nbt::Limits {
            max_bytes: crate::buf::NBT_QUOTA,
            ..nbt::Limits::default()
        },
    )
    .map_err(|e| Error::new(format!("protocol 47 item NBT serialization: {e}")))?;
    encoded.data.extend_from_slice(&tag);
    w.data.extend_from_slice(&encoded.data);
    Ok(())
}

#[derive(Clone, Debug)]
struct Original {
    value: Option<Raw47Slot>,
    bytes: usize,
}

/// Connection-local, bounded originals keyed by a pinned current stack serialized with
/// count=1, an empty WirePatch and sorted component additions/removals. Count changes can
/// then reuse an original while any component/item change misses the cache. The caller
/// uses v1_20_3::Cache::key under ids::unrenumbered, exactly as the existing NBT cache does.
#[derive(Clone, Debug, Default)]
pub struct Cache {
    entries: HashMap<Vec<u8>, Original>,
    order: VecDeque<Vec<u8>>,
    bytes: usize,
}

impl Cache {
    pub fn contains(&self, key: &[u8]) -> bool {
        self.entries.contains_key(key)
    }
    /// Returns false if a single value cannot fit the cache. Colliding semantic keys
    /// stay ambiguous until eviction, even if a later observation repeats one original.
    pub fn remember(&mut self, key: Vec<u8>, value: &Raw47Slot) -> Result<bool> {
        value.validate()?;
        if key.len() > CACHE_ENTRY_BYTES / 2 {
            return Ok(false);
        }
        if let Some(original) = self.entries.get_mut(&key) {
            if original
                .value
                .as_ref()
                .is_some_and(|old| !old.identity_eq(value))
            {
                self.bytes -= original.bytes;
                original.value = None;
                original.bytes = key.len() * 2;
                self.bytes += original.bytes;
            }
            return Ok(true);
        }
        let mut encoded = Writer::new();
        try_write_slot(Some(value), &mut encoded)?;
        let bytes = key.len() * 2 + encoded.data.len();
        if bytes > CACHE_ENTRY_BYTES {
            return Ok(false);
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_BYTES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&old) {
                self.bytes -= old.bytes;
            }
        }
        let mut value = value.clone();
        value.count = 1;
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            Original {
                value: Some(value),
                bytes,
            },
        );
        Ok(true)
    }

    /// No id, variant, damage or NBT fallback is permitted on a cache miss/collision.
    pub fn original(&self, key: &[u8], count: i32) -> Result<Raw47Slot> {
        if !(1..=127).contains(&count) {
            return Err(Error::new("protocol 47 item count is outside 1..=127"));
        }
        let value = self
            .entries
            .get(key)
            .and_then(|entry| entry.value.as_ref())
            .ok_or_else(|| {
                Error::new(
                    "protocol 47 item has no unambiguous original; new/modified items require a verified reverse mapping",
                )
            })?;
        let mut result = value.clone();
        result.count = count as i8;
        Ok(result)
    }

    pub fn try_write_original(&self, key: &[u8], count: i32, w: &mut Writer) -> Result<()> {
        let value = self.original(key, count)?;
        try_write_slot(Some(&value), w)
    }
}

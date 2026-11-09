//! Entity-specific DataWatcher interpretation. Output serializer and index namespace is pinned777.
use super::{entity_ids, raw};
use crate::{
    Error, Result,
    entity_data::{Particle, ParticleData, Value, serializer as s},
    item::Stack,
};
use horizon_world::nbt::Tag;
use std::collections::{BTreeMap, BTreeSet};

pub type Values = Vec<(u8, i32, Value)>;
#[derive(Clone, Debug)]
pub struct State {
    old_kind: i32,
    fields: BTreeMap<u8, raw::Value>,
    pinned_kind: i32,
    /// Full inventory items keyed by current equipment slot and old presence/material.
    /// Metadata only carries saddle presence / armor material, not item NBT.
    horse_items: BTreeMap<u8, (i32, Option<Stack>)>,
}
#[derive(Debug)]
pub struct Update {
    pub pinned_kind: i32,
    /// A caller must replace/recreate an already-spawned entity before applying another entity type's fields.
    pub kind_changed: bool,
    pub values: Values,
    pub equipment: Vec<(u8, Option<Stack>)>,
    pub registry_values: Vec<RegistryValue>,
    pub component_values: Vec<(u8, String)>,
    /// Explicitly retained unsupported fields. They must never be blindly reindexed or silently advertised as supported.
    pub untranslated: Vec<raw::Entry>,
}
#[derive(Debug)]
pub struct RegistryValue {
    pub index: u8,
    pub serializer: i32,
    pub registry: &'static str,
    pub name: &'static str,
}
impl Update {
    pub fn resolve_components(
        &mut self,
        mut component: impl FnMut(&str) -> Result<Tag>,
    ) -> Result<()> {
        let mut resolved = Vec::new();
        for (index, json) in &self.component_values {
            resolved.push((*index, s::COMPONENT, Value::Text(component(json)?)));
        }
        self.values.extend(resolved);
        self.component_values.clear();
        self.values.sort_by_key(|value| value.0);
        Ok(())
    }
    pub fn resolve_registries(
        &mut self,
        mut lookup: impl FnMut(&str, &str) -> Result<i32>,
    ) -> Result<()> {
        let mut resolved = Vec::new();
        for value in &self.registry_values {
            // CatVariant.STREAM_CODEC is holderRegistry: its network ID is bare.
            // Painting's registry-or-inline holder bias is handled separately in objects.rs.
            let id = lookup(value.registry, value.name)?;
            if id < 0 {
                return Err(Error::new("invalid legacy registry variant id"));
            }
            resolved.push((value.index, value.serializer, Value::Id(id)));
        }
        self.values.extend(resolved);
        self.registry_values.clear();
        self.values.sort_by_key(|value| value.0);
        Ok(())
    }
}
fn wrong(index: u8) -> Error {
    Error::new(format!("wrong protocol47 metadata type at{index}"))
}
fn byte(index: u8, value: &raw::Value) -> Result<i8> {
    if let raw::Value::Byte(v) = value {
        Ok(*v)
    } else {
        Err(wrong(index))
    }
}
fn int(index: u8, value: &raw::Value) -> Result<i32> {
    if let raw::Value::Int(v) = value {
        Ok(*v)
    } else {
        Err(wrong(index))
    }
}
fn float(index: u8, value: &raw::Value) -> Result<f32> {
    if let raw::Value::Float(v) = value {
        if v.is_finite() {
            Ok(*v)
        } else {
            Err(wrong(index))
        }
    } else {
        Err(wrong(index))
    }
}
fn string(index: u8, value: &raw::Value) -> Result<&str> {
    if let raw::Value::String(v) = value {
        Ok(v)
    } else {
        Err(wrong(index))
    }
}
fn uuid(value: &str) -> Result<Option<u128>> {
    if value.is_empty() {
        return Ok(None);
    }
    let hex = if value.len() == 36 {
        if [8, 13, 18, 23]
            .iter()
            .any(|&at| value.as_bytes()[at] != b'-')
        {
            return Err(Error::new("invalid protocol47 owner UUID"));
        }
        value.chars().filter(|&c| c != '-').collect::<String>()
    } else {
        value.into()
    };
    if hex.len() != 32 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::new("invalid protocol47 owner UUID"));
    }
    Ok(Some(u128::from_str_radix(&hex, 16).map_err(|_| {
        Error::new("invalid protocol47 owner UUID")
    })?))
}
fn living(kind: i32) -> bool {
    kind == -1 || entity_ids::is_living_mob(kind)
}
fn ageable(kind: i32) -> bool {
    matches!(kind, 90..=93 | 95 | 96 | 98 | 100 | 101 | 120)
}
impl State {
    pub fn pickup_count(&self) -> Option<i32> {
        if self.old_kind == 1 {
            match self.fields.get(&10) {
                Some(raw::Value::Item(Some(slot))) => Some(i32::from(slot.count)),
                _ => None,
            }
        } else {
            Some(1)
        }
    }
    pub fn new(old_kind: i32) -> Result<Self> {
        let pinned_kind = if old_kind == -1 {
            159
        } else {
            entity_ids::pinned(old_kind)
                .ok_or_else(|| Error::new("unknown protocol47 entity kind"))?
        };
        Ok(Self {
            old_kind,
            fields: Default::default(),
            pinned_kind,
            horse_items: Default::default(),
        })
    }
    pub(crate) fn remember_horse_slot(
        &mut self,
        old_slot: u8,
        raw: Option<&raw::Slot>,
        canonical: Option<Stack>,
    ) -> Result<()> {
        if self.old_kind != 100 || old_slot > 1 {
            return Err(Error::new(
                "horse inventory update for nonhorse/invalid slot",
            ));
        }
        if raw.map(|s| i32::from(s.count)) != canonical.as_ref().map(|s| s.count) {
            return Err(Error::new(
                "horse inventory raw/canonical count disagreement",
            ));
        }
        let (slot, material, expected) = match (old_slot, raw.map(|s| s.id)) {
            (0, None) => (7, 0, None),
            (0, Some(329)) => (7, 1, Some(949)),
            (1, None) => (6, 0, None),
            (1, Some(id @ 417..=419)) => (6, i32::from(id - 416), Some(i32::from(id) + 990)),
            _ => return Err(Error::new("invalid old horse saddle/armor item")),
        };
        if canonical.as_ref().map(|s| s.item) != expected
            || raw.is_some_and(|s| s.count != 1 || !matches!(s.tag, Tag::End | Tag::Compound(_)))
        {
            return Err(Error::new("invalid canonical horse saddle/armor stack"));
        }
        // Synchronize the semantic flags as well as full equipment. A later HP-only
        // update reprocesses cached metadata, so leaving these stale would undo it.
        if old_slot == 0 {
            let flags = match self.fields.get(&16) {
                Some(raw::Value::Int(v)) => *v,
                None => 0,
                _ => return Err(wrong(16)),
            };
            self.fields.insert(
                16,
                raw::Value::Int((flags & !4) | if material == 0 { 0 } else { 4 }),
            );
        } else {
            self.fields.insert(22, raw::Value::Int(material));
        }
        self.horse_items.insert(slot, (material, canonical));
        Ok(())
    }
    /// Transactional: wrong field types or callback failures leave the previous entity state intact.
    pub fn apply(
        &mut self,
        entries: &[raw::Entry],
        mut upgrade_item: impl FnMut(&raw::Slot) -> Result<Stack>,
        mut block_state: impl FnMut(u16) -> Result<i32>,
    ) -> Result<Update> {
        let mut next = self.fields.clone();
        let mut seen = BTreeSet::new();
        for entry in entries {
            if entry.index > 31 || !seen.insert(entry.index) {
                return Err(Error::new("invalid or duplicate protocol47 metadata index"));
            }
            next.insert(entry.index, entry.value.clone());
        }
        let mut pinned_kind = if self.old_kind == -1 {
            159
        } else {
            entity_ids::pinned(self.old_kind).unwrap()
        };
        let mut values = BTreeMap::<u8, (i32, Value)>::new();
        let mut untranslated = Vec::new();
        let mut equipment = Vec::new();
        let mut registry_values = Vec::new();
        let mut component_values = Vec::new();
        let mut put = |index, serializer, value| {
            values.insert(index, (serializer, value));
        };
        // Some old subtypes are now separately registered entity types.
        if let Some((old_index, kinds)) = match self.old_kind {
            51 => Some((13, vec![118, 150])),
            54 => Some((13, vec![154, 157])),
            100 => Some((19, vec![67, 37, 88, 155, 119])),
            _ => None,
        } && let Some(value) = next.get(&old_index)
        {
            let variant = byte(old_index, value)?;
            pinned_kind = *kinds
                .get(usize::try_from(variant).map_err(|_| wrong(old_index))?)
                .ok_or_else(|| wrong(old_index))?;
        }
        if self.old_kind == 68
            && let Some(value) = next.get(&16)
            && int(16, value)? & 4 != 0
        {
            pinned_kind = 41;
        }
        if self.old_kind == 98
            && let Some(value) = next.get(&18)
            && byte(18, value)? != 0
        {
            pinned_kind = 21;
        }
        for (&index, value) in &next {
            match index {
                0 => {
                    let flags = byte(index, value)?;
                    // Riding0x04 is an attachment packet; using-item0x10 moved to LivingEntity flags.
                    put(0, s::BYTE, Value::Byte(flags & 0x2b));
                    if flags as u8 & 0xc0 != 0 {
                        untranslated.push(raw::Entry {
                            index,
                            value: value.clone(),
                        });
                    }
                    if self.old_kind == -1 {
                        put(6, s::POSE, Value::Id(if flags & 2 != 0 { 5 } else { 0 }));
                    }
                    if living(self.old_kind) {
                        put(8, s::BYTE, Value::Byte(i8::from(flags & 0x10 != 0)));
                    }
                }
                1 => {
                    let raw::Value::Short(v) = value else {
                        return Err(wrong(index));
                    };
                    put(1, s::INT, Value::Int(i32::from(*v)));
                }
                2 => {
                    let name = string(index, value)?;
                    put(
                        2,
                        s::OPTIONAL_COMPONENT,
                        Value::OptText(
                            (!name.is_empty())
                                .then(|| Tag::compound([("text", Tag::String(name.into()))])),
                        ),
                    );
                }
                3 | 4 => {
                    put(index, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                }
                6 if living(self.old_kind) => {
                    put(9, s::FLOAT, Value::Float(float(index, value)?));
                }
                7 if living(self.old_kind) => {
                    let color = int(index, value)?;
                    let particles = if color == 0 {
                        Vec::new()
                    } else {
                        vec![Particle {
                            id: 28,
                            data: Some(ParticleData::Color(color | 0xff000000u32 as i32)),
                        }]
                    };
                    put(10, s::PARTICLES, Value::Particles(particles));
                }
                8 if living(self.old_kind) => {
                    put(11, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                }
                9 if living(self.old_kind) => {
                    put(12, s::INT, Value::Int(i32::from(byte(index, value)? as u8)));
                }
                15 if living(self.old_kind) && self.old_kind != -1 && self.old_kind != 30 => {
                    put(15, s::BYTE, Value::Byte(byte(index, value)? & 1));
                }
                12 if ageable(self.old_kind) => {
                    put(16, s::BOOLEAN, Value::Bool(byte(index, value)? < 0));
                }
                _ => match (self.old_kind, index) {
                    (-1, 10) => {
                        put(16, s::BYTE, Value::Byte(byte(index, value)?));
                    }
                    // Actual vanilla player initialization sends reserved16 Byte0.
                    // Current16 is skin customization (old10), so it must not be shifted.
                    // Only the observed default is a no-op; other values remain explicit.
                    (-1, 16) if byte(index, value)? == 0 => {}
                    (-1, 17) => {
                        put(17, s::FLOAT, Value::Float(float(index, value)?));
                    }
                    (-1, 18) => {
                        put(18, s::INT, Value::Int(int(index, value)?));
                    }
                    (50, 16) => {
                        put(16, s::INT, Value::Int(i32::from(byte(index, value)?)));
                    }
                    (50, 17 | 18) => {
                        put(index, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (51, 13) => {
                        byte(index, value)?;
                    }
                    (54 | 57, 12) => {
                        put(16, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (54 | 57, 13) => {
                        byte(index, value)?;
                    }
                    (54 | 57, 14) if pinned_kind == 157 => {
                        put(19, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (54 | 57, 14) if byte(index, value)? == 0 => {}
                    (52 | 59 | 61 | 65 | 99, 16) => {
                        put(16, s::BYTE, Value::Byte(byte(index, value)?));
                    }
                    (55 | 62, 16) => {
                        let size = byte(index, value)?;
                        if size <= 0 {
                            return Err(wrong(index));
                        }
                        put(18, s::INT, Value::Int(i32::from(size)));
                    }
                    (56, 16) => {
                        put(16, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (66, 21) => {
                        put(17, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (68, 16) => {
                        put(16, s::BOOLEAN, Value::Bool(int(index, value)? & 2 != 0));
                    }
                    (68, 17) => {
                        put(17, s::INT, Value::Int(int(index, value)?));
                    }
                    (64, 17..=20) => {
                        put(index - 1, s::INT, Value::Int(int(index, value)?));
                    }
                    (91, 16) => {
                        put(18, s::BYTE, Value::Byte(byte(index, value)?));
                    }
                    (90, 16) => {
                        equipment.push((7, (byte(index, value)? != 0).then(|| Stack::new(949, 1))));
                    }
                    (95, 16) => {
                        let flags = byte(index, value)?;
                        put(18, s::BYTE, Value::Byte(flags & 5));
                        // The old wire supplies an angry boolean with no end tick: keep it until the next clear.
                        put(
                            22,
                            s::LONG,
                            Value::Long(if flags & 2 != 0 { i64::MAX } else { 0 }),
                        );
                    }
                    (95, 17) => {
                        put(
                            19,
                            s::OPTIONAL_LIVING_ENTITY_REFERENCE,
                            Value::OptUuid(uuid(string(index, value)?)?),
                        );
                    }
                    (98, 17) if pinned_kind == 21 => {
                        put(
                            19,
                            s::OPTIONAL_LIVING_ENTITY_REFERENCE,
                            Value::OptUuid(uuid(string(index, value)?)?),
                        );
                    }
                    (98, 17) if string(index, value)?.is_empty() => {}
                    (95, 18) => {
                        float(index, value)?;
                    }
                    (95, 19) => {
                        put(20, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (95, 20) => {
                        put(21, s::INT, Value::Int(i32::from(byte(index, value)? as u8)));
                    }
                    (98, 16) if pinned_kind == 21 => {
                        put(18, s::BYTE, Value::Byte(byte(index, value)? & 5));
                    }
                    (98, 16) if byte(index, value)? == 0 => {}
                    (98, 18) if byte(index, value)? == 0 => {}
                    (98, 18) => {
                        let name = match byte(index, value)? {
                            1 => "minecraft:black",
                            2 => "minecraft:red",
                            3 => "minecraft:siamese",
                            _ => return Err(wrong(index)),
                        };
                        registry_values.push(RegistryValue {
                            index: 20,
                            serializer: s::CAT_VARIANT,
                            registry: "minecraft:cat_variant",
                            name,
                        });
                    }
                    (100, 16) => {
                        let flags = int(index, value)?;
                        if !(0..=255).contains(&flags) {
                            return Err(wrong(index));
                        }
                        put(18, s::BYTE, Value::Byte((flags & 0xf2) as i8));
                        equipment.push((7, (flags & 4 != 0).then(|| Stack::new(949, 1))));
                        if matches!(pinned_kind, 37 | 88) {
                            put(19, s::BOOLEAN, Value::Bool(flags & 8 != 0));
                        }
                    }
                    (100, 19) => {
                        byte(index, value)?;
                    }
                    (100, 20) => {
                        let variant = int(index, value)?;
                        if pinned_kind == 67 {
                            put(19, s::INT, Value::Int(variant));
                        }
                    }
                    (100, 21) => {
                        uuid(string(index, value)?)?;
                    }
                    (100, 22) => {
                        let armor = match int(index, value)? {
                            0 => None,
                            1 => Some(Stack::new(1407, 1)),
                            2 => Some(Stack::new(1408, 1)),
                            3 => Some(Stack::new(1409, 1)),
                            _ => return Err(wrong(index)),
                        };
                        equipment.push((6, if pinned_kind == 67 { armor } else { None }));
                    }
                    (40..=47, 17 | 18) => {
                        put(index - 9, s::INT, Value::Int(int(index, value)?));
                    }
                    (40..=47, 19) => {
                        put(10, s::FLOAT, Value::Float(float(index, value)?));
                    }
                    (40 | 42..=47, 20 | 22) => {
                        let raw_state = match next.get(&20) {
                            Some(raw::Value::Int(v)) if *v >= 0 => *v as u32,
                            None => 0,
                            _ => return Err(wrong(20)),
                        };
                        let custom = match next.get(&22) {
                            Some(v) => byte(22, v)? != 0,
                            None => false,
                        };
                        let state = if custom {
                            block_state(
                                (((raw_state & 4095) << 4) | ((raw_state >> 12) & 15)) as u16,
                            )?
                        } else {
                            0
                        };
                        put(11, s::OPTIONAL_BLOCK_STATE, Value::Id(state));
                    }
                    (40 | 42..=47, 21) => {
                        put(12, s::INT, Value::Int(int(index, value)?));
                    }
                    (44, 16) => {
                        put(13, s::BOOLEAN, Value::Bool(byte(index, value)? & 1 != 0));
                    }
                    (40, 23) => {
                        put(13, s::STRING, Value::String(string(index, value)?.into()));
                    }
                    (40, 24) => {
                        let output = string(index, value)?;
                        if output.is_empty() {
                            put(14, s::COMPONENT, Value::Text(Tag::String("".into())));
                        } else {
                            component_values.push((14, output.into()));
                        }
                    }
                    (101, 18) => {
                        put(18, s::INT, Value::Int(i32::from(byte(index, value)? as u8)));
                    }
                    (120, 16) => {
                        let profession = match int(index, value)? {
                            0 => 5,
                            1 => 9,
                            2 => 4,
                            3 => 1,
                            4 => 2,
                            _ => return Err(wrong(index)),
                        };
                        // Protocol47 omits villager career and biome; use the old profession's default career.
                        put(19, s::VILLAGER_DATA, Value::Ids(vec![2, profession, 1]));
                    }
                    (30, 10) => {
                        put(15, s::BYTE, Value::Byte(byte(index, value)?));
                    }
                    (30, 11..=16) => {
                        let raw::Value::Rotation(v) = value else {
                            return Err(wrong(index));
                        };
                        if v.iter().any(|v| !v.is_finite()) {
                            return Err(wrong(index));
                        }
                        put(index + 5, s::ROTATIONS, Value::Floats(v.to_vec()));
                    }
                    (1, 10) | (18, 8) | (22, 8) => {
                        let raw::Value::Item(old) = value else {
                            return Err(wrong(index));
                        };
                        let target = if self.old_kind == 18 { 9 } else { 8 };
                        put(
                            target,
                            s::ITEM_STACK,
                            Value::Item(old.as_ref().map(&mut upgrade_item).transpose()?),
                        );
                    }
                    (18, 9) => {
                        put(10, s::INT, Value::Int(i32::from(byte(index, value)?)));
                    }
                    (10, 16) => {
                        put(8, s::BYTE, Value::Byte(byte(index, value)? & 1));
                    }
                    (19, 10) => {
                        put(8, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    (200, 8) => {
                        int(index, value)?;
                    } // Removed health/rotation counter; the current crystal animates locally.
                    (58, 16 | 17) => {
                        let block = match next.get(&16) {
                            Some(raw::Value::Short(v)) if (0..=4095).contains(v) => *v as u16,
                            _ => return Err(wrong(16)),
                        };
                        let meta = match next.get(&17) {
                            Some(raw::Value::Byte(v)) if (0..=15).contains(v) => *v as u16,
                            None => 0,
                            _ => return Err(wrong(17)),
                        };
                        put(
                            16,
                            s::OPTIONAL_BLOCK_STATE,
                            Value::Id(block_state((block << 4) | meta)?),
                        );
                    }
                    (58, 18) => {
                        put(17, s::BOOLEAN, Value::Bool(byte(index, value)? != 0));
                    }
                    _ => untranslated.push(raw::Entry {
                        index,
                        value: value.clone(),
                    }),
                },
            }
        }
        let kind_changed = pinned_kind != self.pinned_kind;
        // Keep full inventory NBT while authoritative metadata still reports its
        // material/presence. A genuine material change discards the old override.
        self.horse_items.retain(|slot, (material, _)| {
            let current = match (*slot, next.get(&if *slot == 7 { 16 } else { 22 })) {
                (7, Some(raw::Value::Int(v))) => i32::from(v & 4 != 0),
                (6, Some(raw::Value::Int(v))) => *v,
                _ => 0,
            };
            current == *material
        });
        for (&slot, (_, item)) in &self.horse_items {
            let value = if slot == 6 && pinned_kind != 67 {
                None
            } else {
                item.clone()
            };
            equipment.retain(|(existing, _)| *existing != slot);
            equipment.push((slot, value));
        }
        self.fields = next;
        self.pinned_kind = pinned_kind;
        Ok(Update {
            pinned_kind,
            kind_changed,
            values: values.into_iter().map(|(i, (s, v))| (i, s, v)).collect(),
            equipment,
            registry_values,
            component_values,
            untranslated,
        })
    }
}

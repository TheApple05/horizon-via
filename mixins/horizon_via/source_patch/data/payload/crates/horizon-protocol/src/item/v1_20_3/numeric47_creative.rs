//! A narrow verified reverse path for new creative stacks with vanilla default components.
//! Existing inventory values always use the exact-original cache. No arbitrary NBT is reconstructed.
use super::*;
use crate::adapter::v1_8_9::{facts, ids as old};
use std::sync::OnceLock;

fn key(stack: &Stack) -> Option<Vec<u8>> {
    let mut normalized = stack.clone();
    let egg_entity = ids::legacy_registry_name("minecraft:item", stack.item)
        .and_then(|name| name.strip_suffix("_spawn_egg").map(str::to_owned));
    // Vanilla prototypes already supply these defaults; their explicit patch is equivalent.
    // Keep every other component and every removal, so unsupported edited data cannot vanish.
    normalized
        .added
        .retain(|(kind, component)| match (kind, component) {
            (3 | 19, Component::Int(0)) => false,
            (13 | 45, Component::Enchantments(values)) if values.is_empty() => false,
            (60, Component::Raw(bytes))
                if egg_entity.as_ref().is_some_and(|expected| {
                    let mut r = Reader::new(bytes);
                    let Ok(id) = r.varint() else {
                        return false;
                    };
                    let Ok(tag) = r.nbt() else {
                        return false;
                    };
                    r.remaining() == 0
                        && matches!(tag,Tag::Compound(ref fields) if fields.is_empty())
                        && ids::legacy_registry_name("minecraft:entity_type", id).as_ref()
                            == Some(expected)
                }) =>
            {
                false
            }
            _ => true,
        });
    Cache::key(&normalized)
}

fn canonical() -> &'static HashMap<Vec<u8>, super::super::item47::Raw47Slot> {
    static VALUES: OnceLock<HashMap<Vec<u8>, super::super::item47::Raw47Slot>> = OnceLock::new();
    VALUES.get_or_init(|| {
        let start = std::time::Instant::now();
        let mut values = HashMap::new();
        // Fact rows come from all real vanilla old base IDs and independently exercised subtype,
        // potion and spawn-egg samples. Every chosen encoding must upgrade to the same current key.
        for &(id, damage) in facts::CREATIVE_VARIANTS {
            let raw = super::super::item47::Raw47Slot {
                id,
                count: 1,
                damage,
                tag: Tag::End,
            };
            let Ok(saved) = raw.upgrade(old::old_item_name) else {
                continue;
            };
            let Ok(stack) = from_tag(&saved, None) else {
                continue;
            };
            if let Some(key) = key(&stack) {
                values.entry(key).or_insert(raw);
            }
        }
        log::debug!(
            "ViaHorizon: prepared {} verified default creative item mappings in {:?}",
            values.len(),
            start.elapsed()
        );
        values
    })
}

pub(crate) fn try_write(stack: Option<&Stack>, writer: &mut Writer) -> Result<()> {
    let Some(stack) = stack else {
        return super::super::item47::try_write_slot(None, writer);
    };
    if !(1..=127).contains(&stack.count) {
        return Err(Error::new("creative count is outside protocol47 range"));
    }
    let exact_key =
        Cache::key(stack).ok_or_else(|| Error::new("creative item key is too large"))?;
    let cache =
        ids::legacy_item_cache().ok_or_else(|| Error::new("missing protocol47 item cache"))?;
    if cache.lock().unwrap().numeric47.contains(&exact_key) {
        // Known ambiguous originals remain errors; never use canonical data to conceal ambiguity.
        return cache
            .lock()
            .unwrap()
            .numeric47
            .try_write_original(&exact_key, stack.count, writer);
    }
    let semantic = key(stack).ok_or_else(|| Error::new("creative item key is too large"))?;
    let mut raw = canonical().get(&semantic).cloned().ok_or_else(|| {
        Error::new("this new or edited creative item has no verified protocol47 representation")
    })?;
    raw.count = stack.count as i8;
    let mut encoded = Writer::new();
    super::super::item47::try_write_slot(Some(&raw), &mut encoded)?;
    if !cache.lock().unwrap().numeric47.remember(exact_key, &raw)? {
        return Err(Error::new("creative item exceeds protocol47 cache budget"));
    }
    writer.bytes(&encoded.data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn context() -> (ids::Guard, ids::LegacyRegistryGuard, ids::LegacyItemGuard) {
        (
            ids::unrenumbered(),
            ids::legacy_registries(crate::adapter::v1_8_9::bootstrap::names()),
            ids::legacy_items(Arc::new(Mutex::new(Cache::default()))),
        )
    }
    fn current(name: &str, count: i32) -> Stack {
        let id = ids::legacy_registry_id("minecraft:item", name).unwrap();
        Stack::new(id, count)
    }
    #[test]
    fn vanilla_creative_blocks_and_undamaged_weapon_choose_verified_old_variants() {
        let _ctx = context();
        for (name, id, damage, count) in [
            ("minecraft:stone", 1, 0, 64),
            ("minecraft:blue_wool", 35, 11, 3),
            ("minecraft:diamond_sword", 276, 0, 1),
            ("minecraft:creeper_spawn_egg", 383, 50, 1),
        ] {
            let mut wire = Writer::new();
            try_write(Some(&current(name, count)), &mut wire).unwrap();
            let (old, used) = super::super::super::item47::read_slot(&wire.data).unwrap();
            assert_eq!(used, wire.data.len());
            let old = old.unwrap();
            assert_eq!((old.id, old.damage, old.count), (id, damage, count as i8));
            assert_eq!(old.tag, Tag::End);
        }
    }
    #[test]
    fn edited_components_removals_and_out_of_range_counts_are_atomic_errors() {
        let _ctx = context();
        let mut named = current("minecraft:stone", 1);
        named
            .added
            .push((6, Component::Text(Tag::String("cannot disappear".into()))));
        let mut removed = current("minecraft:stone", 1);
        removed.removed.push(6);
        for stack in [
            named,
            removed,
            current("minecraft:stone", 128),
            current("minecraft:stone", 0),
        ] {
            let mut wire = Writer::new();
            wire.u8(42);
            assert!(try_write(Some(&stack), &mut wire).is_err());
            assert_eq!(wire.data, vec![42]);
        }
    }
    #[test]
    fn ambiguous_originals_do_not_fall_back_to_a_default_creative_item() {
        let _ctx = context();
        let stack = current("minecraft:stone", 1);
        let key = Cache::key(&stack).unwrap();
        let cache = ids::legacy_item_cache().unwrap();
        let first = super::super::super::item47::Raw47Slot {
            id: 1,
            count: 1,
            damage: 0,
            tag: Tag::End,
        };
        let mut second = first.clone();
        second.tag = Tag::Compound(Default::default());
        cache
            .lock()
            .unwrap()
            .numeric47
            .remember(key.clone(), &first)
            .unwrap();
        cache
            .lock()
            .unwrap()
            .numeric47
            .remember(key, &second)
            .unwrap();
        let mut wire = Writer::new();
        assert!(try_write(Some(&stack), &mut wire).is_err());
        assert!(wire.data.is_empty());
    }
}

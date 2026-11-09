//! Fixed vanilla340 recipe displays and their original numeric registry IDs.
use super::{boolean, facts, finish};
use crate::{
    Error, Result,
    buf::{Reader, Writer},
    item::Stack,
    packets::{play, recipes as p},
};
use horizon_world::{datafix::dynamic as d, nbt::Tag};
use std::{collections::BTreeSet, sync::OnceLock};

#[derive(Clone, Copy)]
struct Raw {
    id: i16,
    count: i8,
    damage: i16,
}
struct Spec {
    id: i32,
    name: &'static str,
    width: i32,
    height: i32,
    group: i32,
    ingredients: &'static [&'static [Raw]],
    result: Raw,
}
include!("recipe_facts.rs");

fn current_item(name: &str) -> Result<i32> {
    super::names()
        .get("minecraft:item")
        .and_then(|v| v.id(name))
        .ok_or_else(|| Error::new("recipe item is absent from pinned registry"))
}
fn stack(raw: Raw) -> Result<Stack> {
    let saved = crate::item::item47::Raw47Slot {
        id: raw.id,
        count: raw.count,
        damage: raw.damage,
        tag: Tag::End,
    }
    .upgrade_at(facts::old_item_name, 1343)?;
    if let Some(Tag::Compound(values)) = saved.get("components") {
        for (key, value) in values {
            let name = key.to_string_lossy();
            if !matches!(name.as_str(), "minecraft:damage" | "minecraft:repair_cost")
                || d::as_int(Some(value), -1) != 0
            {
                return Err(Error::new("fixed recipe needs a component-aware display"));
            }
        }
    }
    let name =
        d::as_str(saved.get("id")).ok_or_else(|| Error::new("recipe item upgrade lost name"))?;
    let count = d::as_int(saved.get("count"), -1);
    if count != i32::from(raw.count) {
        return Err(Error::new("recipe item upgrade changed count"));
    }
    Ok(Stack::new(current_item(&name)?, count))
}
fn entries() -> Result<&'static [p::RecipeBookEntry]> {
    static ALL: OnceLock<std::result::Result<Vec<p::RecipeBookEntry>, String>> = OnceLock::new();
    ALL.get_or_init(|| build().map_err(|e| e.to_string()))
        .as_ref()
        .map(Vec::as_slice)
        .map_err(|e| Error::new(e.clone()))
}
fn build() -> Result<Vec<p::RecipeBookEntry>> {
    let station = p::SlotDisplay::Item(current_item("minecraft:crafting_table")?);
    let mut out = Vec::with_capacity(RECIPES.len());
    for spec in RECIPES {
        let mut ingredients = Vec::new();
        let mut requirements = Vec::new();
        for alternatives in spec.ingredients {
            let mut items = Vec::new();
            for &raw in *alternatives {
                let item = stack(raw)?.item;
                if !items.contains(&item) {
                    items.push(item);
                }
            }
            ingredients.push(match items.as_slice() {
                [] => p::SlotDisplay::Empty,
                [id] => p::SlotDisplay::Item(*id),
                _ => p::SlotDisplay::Composite(
                    items.iter().copied().map(p::SlotDisplay::Item).collect(),
                ),
            });
            if !items.is_empty() {
                requirements.push(p::HolderSet::Items(items));
            }
        }
        let result = p::SlotDisplay::ItemStack(
            stack(spec.result)
                .map_err(|e| Error::new(format!("protocol340 recipe {}: {e}", spec.name)))?,
        );
        let display = if spec.width > 0 {
            if ingredients.len() != (spec.width * spec.height) as usize {
                return Err(Error::new("invalid factual shaped recipe dimensions"));
            }
            p::RecipeDisplay::Shaped {
                width: spec.width,
                height: spec.height,
                ingredients,
                result,
                station: station.clone(),
            }
        } else {
            p::RecipeDisplay::Shapeless {
                ingredients,
                result,
                station: station.clone(),
            }
        };
        out.push(p::RecipeBookEntry {
            id: spec.id,
            display,
            group: spec.group,
            category: 3,
            requirements: Some(requirements),
            flags: 0,
        });
    }
    Ok(out)
}
fn known(id: i32) -> Result<()> {
    if RECIPES.binary_search_by_key(&id, |v| v.id).is_ok() || DYNAMIC.binary_search(&id).is_ok() {
        Ok(())
    } else {
        Err(Error::new("unknown protocol340 recipe registry ID"))
    }
}
fn entry(id: i32) -> Result<Option<p::RecipeBookEntry>> {
    known(id)?;
    let all = entries()?;
    Ok(all
        .binary_search_by_key(&id, |v| v.id)
        .ok()
        .map(|i| all[i].clone()))
}
fn list(r: &mut Reader<'_>) -> Result<Vec<i32>> {
    let n = r.count(65536)?;
    let mut out = Vec::with_capacity(n.min(512));
    for _ in 0..n {
        let id = r.varint()?;
        known(id)?;
        out.push(id);
    }
    Ok(out)
}

#[derive(Default)]
pub struct State {
    unlocked: BTreeSet<i32>,
}
impl State {
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
    ) -> Result<Option<Vec<play::Clientbound>>> {
        let mut r = Reader::new(payload);
        let out = match id {
            0x31 => {
                let action = r.varint()?;
                if !(0..=2).contains(&action) {
                    return Err(Error::new("invalid protocol340 recipe book action"));
                }
                let crafting = (boolean(&mut r)?, boolean(&mut r)?);
                let ids = list(&mut r)?;
                let highlighted = if action == 0 {
                    list(&mut r)?.into_iter().collect()
                } else {
                    ids.iter().copied().collect::<BTreeSet<_>>()
                };
                finish(&r)?;
                let mut out = vec![play::Clientbound::RecipeBookSettings(
                    p::RecipeBookSettings {
                        books: [crafting, (false, false), (false, false), (false, false)],
                    },
                )];
                if action == 2 {
                    for id in &ids {
                        self.unlocked.remove(id);
                    }
                    out.push(play::Clientbound::RecipeBookRemove(p::RecipeBookRemove {
                        ids,
                    }));
                } else {
                    let mut entries = Vec::new();
                    for &id in &ids {
                        if let Some(mut entry) = entry(id)? {
                            entry.flags = i8::from(action == 1)
                                | if highlighted.contains(&id) { 2 } else { 0 };
                            entries.push(entry);
                        }
                    }
                    if action == 0 {
                        self.unlocked.clear();
                    }
                    self.unlocked.extend(ids);
                    out.push(play::Clientbound::RecipeBookAdd(p::RecipeBookAdd {
                        entries,
                        replace: action == 0,
                    }));
                }
                out
            }
            0x2b => {
                let container = i32::from(r.u8()?);
                let recipe = r.varint()?;
                finish(&r)?;
                let entry = entry(recipe)?
                    .ok_or_else(|| Error::new("dynamic recipe has no fixed ghost display"))?;
                vec![play::Clientbound::PlaceGhostRecipe(p::PlaceGhostRecipe {
                    container,
                    display: entry.display,
                })]
            }
            _ => return Ok(None),
        };
        Ok(Some(out))
    }
    pub fn serverbound(&mut self, packet: &play::Serverbound) -> Result<Option<(i32, Vec<u8>)>> {
        let mut w = Writer::new();
        let id = match packet {
            play::Serverbound::PlaceRecipe(p) => {
                if !self.unlocked.contains(&p.recipe) || entry(p.recipe)?.is_none() {
                    return Err(Error::new(
                        "recipe placement is not an unlocked fixed340 recipe",
                    ));
                }
                let container = u8::try_from(p.container)
                    .ok()
                    .filter(|id| *id <= 127)
                    .ok_or_else(|| Error::new("protocol340 recipe container out of range"))?;
                w.u8(container);
                w.varint(p.recipe);
                w.bool(p.use_max);
                0x12
            }
            play::Serverbound::RecipeBookSeenRecipe(p) => {
                known(p.recipe)?;
                w.varint(0);
                w.varint(p.recipe);
                0x17
            }
            play::Serverbound::RecipeBookChangeSettings(p) if p.book_type == 0 => {
                w.varint(1);
                w.bool(p.open);
                w.bool(p.filtering);
                0x17
            }
            _ => return Ok(None),
        };
        Ok(Some((id, w.data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_registry_builds_fixed_displays_and_preserves_original_ids() {
        let _names = crate::adapter::ids::legacy_registries(super::super::names());
        let entries = entries().unwrap();
        assert_eq!(entries.len(), 432);
        for (name, result, count) in [
            ("minecraft:oak_planks", "minecraft:oak_planks", 4),
            ("minecraft:crafting_table", "minecraft:crafting_table", 1),
        ] {
            let spec = RECIPES.iter().find(|v| v.name == name).unwrap();
            let entry = entry(spec.id).unwrap().unwrap();
            let p::SlotDisplay::ItemStack(stack) = entry.display.result() else {
                panic!("result stack")
            };
            assert_eq!(
                (stack.item, stack.count),
                (current_item(result).unwrap(), count)
            );
        }
        for id in DYNAMIC {
            assert!(entry(*id).unwrap().is_none());
        }
    }
}

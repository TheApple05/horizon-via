//! Only pre-flattening properties which require actual neighboring blocks are reconstructed.
use super::{facts, neighbors};
use crate::adapter::v1_8_9::lookup47;
use crate::{Error, Result};
use neighbors::Descriptor;
include!("context_facts.rs");

pub fn base(raw: u16) -> Result<Descriptor> {
    if raw >> 4 != 203 {
        return lookup47::base(raw);
    }
    let id = *facts::STATES
        .get(usize::from(raw))
        .ok_or_else(|| Error::new("invalid purpur stair state"))?;
    let props = PURPUR
        .iter()
        .find(|v| v.1 == id as u32)
        .ok_or_else(|| Error::new("purpur stair base has no pinned descriptor"))?
        .0;
    Ok(Descriptor {
        name: "minecraft:purpur_stairs".into(),
        properties: ["facing", "half", "shape", "waterlogged"]
            .into_iter()
            .zip(props)
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
    })
}
fn resolve(value: &Descriptor) -> Result<i32> {
    if value.name != "minecraft:purpur_stairs" {
        return lookup47::resolve(value).map(|v| v as i32);
    }
    let props = ["facing", "half", "shape", "waterlogged"]
        .map(|key| value.properties.get(key).map(String::as_str));
    PURPUR
        .iter()
        .find(|v| v.0.map(Some) == props)
        .map(|v| v.1 as i32)
        .ok_or_else(|| Error::new("unknown contextual purpur stair properties"))
}
pub fn state(
    raw: u16,
    pos: [i32; 3],
    neighbor: impl Fn([i32; 3]) -> Option<u16>,
) -> Result<Option<i32>> {
    if !neighbors::needs_context(raw) {
        return Ok(None);
    }
    let descriptor = neighbors::derive(raw, pos, neighbor, base)?
        .unwrap_or(neighbors::temporary_base(raw, base)?);
    resolve(&descriptor).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("context_goldens.rs");
    #[test]
    fn actual_vanilla_neighbors_and_models_match_all_5760_cases() {
        for (n, &(raw, neighbors, expected)) in FIXTURES.iter().enumerate() {
            let actual = state(raw, [0, 64, 0], |p| {
                Some(match [p[0], p[1] - 64, p[2]] {
                    [0, 0, -1] => neighbors[0],
                    [1, 0, 0] => neighbors[1],
                    [0, 0, 1] => neighbors[2],
                    [-1, 0, 0] => neighbors[3],
                    [0, 1, 0] => neighbors[4],
                    [0, -1, 0] => neighbors[5],
                    _ => 0,
                })
            });
            assert_eq!(
                actual.unwrap_or_else(|e| panic!("fixture{n},raw{raw}: {e}")),
                Some(expected),
                "fixture{n},raw{raw}"
            );
        }
    }
}

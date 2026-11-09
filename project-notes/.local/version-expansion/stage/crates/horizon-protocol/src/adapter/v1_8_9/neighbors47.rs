//! Original old-neighbor-dependent block-state derivation, with current descriptor output.
//! Caller supplies exact pinned base descriptors and distinguishes unknown neighbors from air.
use crate::{Error, Result};
use std::collections::{BTreeMap, HashSet, VecDeque};

mod connection_data {
    //! Membership facts from independently executed official1.8.9 getActualState.
    pub const CONNECTION_BANKS: [[u64; 64]; 4] = [
        [
            0xffffffffffff0000,
            0xffff0000ffffffff,
            0x0000000000000000,
            0xffffffffffffffff,
            0xffff0000ffffffff,
            0xffffffffffff0000,
            0x00000000ffffffff,
            0x0000000000000000,
            0xffff000000000000,
            0x0000000000000000,
            0xffffffffffff0000,
            0xffff0000ffff0000,
            0x00000000ffffffff,
            0x000000000000ffff,
            0x0000ffffffffffff,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000ffff0000ffff,
            0xffff0000ffffffff,
            0x000000000000ffff,
            0x0000000000000000,
            0xffffffffffff0000,
            0x000000000000ffff,
            0xffff000000000000,
            0x0000ffff00000000,
            0x000000000000ffff,
            0x0000000000000000,
            0xffff0000ffffffff,
            0x00000000ffffffff,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0xffff0000ffffffff,
            0xffffffff00000000,
            0x0000333300000000,
            0x00000000ffff0000,
            0x0000ffff0000ffff,
            0x0000ffffffffffff,
            0xffff000000000000,
            0xffff0000ffff0000,
            0xffffffffffffffff,
            0xffffffffffffffff,
            0x000000000000ffff,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
        ],
        [
            0xffffffffffff0000,
            0xffff0000ffffffff,
            0x0000000000000000,
            0xffffffffffffffff,
            0xffff0000ffffffff,
            0xffffffffffff0000,
            0x00000000ffffffff,
            0x0000000000000000,
            0xffff000000000000,
            0x0000000000000000,
            0xffffffffffff0000,
            0xffff0000ffff0000,
            0x00000000ffffffff,
            0x000000000000ffff,
            0x0000ffffffffffff,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000ffff0000ffff,
            0xffff00000000ffff,
            0x000000000000ffff,
            0x0000000000000000,
            0xffffffffffff0000,
            0x000000000000ffff,
            0xffff000000000000,
            0x0000ffff00000000,
            0x00000000ffffffff,
            0x0000000000000000,
            0xffff0000ffffffff,
            0x00000000ffffffff,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0xffff0000ffffffff,
            0xffffffff00000000,
            0x0000333300000000,
            0x00000000ffff0000,
            0x0000ffff0000ffff,
            0x0000ffffffffffff,
            0xffff000000000000,
            0xffff0000ffff0000,
            0xffffffffffffffff,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
        ],
        [
            0xffffffffffff0000,
            0xffff0000ffffffff,
            0x0000000000000000,
            0xffffffffffffffff,
            0xffffffffffffffff,
            0xffffffffffffffff,
            0x00000000ffffffff,
            0x0000000000000000,
            0xffff000000000000,
            0x0000000000000000,
            0xffffffffffff0000,
            0xffffffffffff0000,
            0x00000000ffffffff,
            0x0000000000000000,
            0x0000ffffffffffff,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000ffff0000ffff,
            0xffffffff0000ffff,
            0xffff0000ffffffff,
            0xffff000000000000,
            0xffffffffffff0000,
            0xffffffffffffffff,
            0x0000000000000000,
            0x0000ffff00000000,
            0x000000000000ffff,
            0x0000000000000000,
            0xffff0000ffff0000,
            0x00000000ffffffff,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0xffff0000ffffffff,
            0xffffffff00000000,
            0x000033333333ffff,
            0x0000000000000000,
            0x0000ffffffffffff,
            0x0000ffffffffffff,
            0xffff000000000000,
            0x00000000ffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
        ],
        [
            0xffffffffffff0000,
            0xffff0000ffffffff,
            0x0000000000000000,
            0xffffffffffffffff,
            0xffff0000ffffffff,
            0xffffffffffff0000,
            0x00000000ffffffff,
            0x0000000000000000,
            0xffff000000000000,
            0x0000000000000000,
            0xffffffffffff0000,
            0xffff0000ffff0000,
            0x00000000ffffffff,
            0x000000000000ffff,
            0x0000ffffffffffff,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000ffffffff0000,
            0x0000000000000000,
            0x0000ffff0000ffff,
            0xffff00000000ffff,
            0x000000000000ffff,
            0x0000000000000000,
            0xffffffffffff0000,
            0x000000000000ffff,
            0xffff000000000000,
            0x0000ffff00000000,
            0x000000000000ffff,
            0x0000000000000000,
            0xffff0000ffffffff,
            0x00000000ffffffff,
            0x00000000ffff0000,
            0x00000000ffff0000,
            0xffff0000ffff0000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0xffff0000ffffffff,
            0xffffffff00000000,
            0x0000333300000000,
            0x00000000ffff0000,
            0x0000ffff0000ffff,
            0x0000ffffffffffff,
            0xffff000000000000,
            0xffff0000ffff0000,
            0xffffffffffffffff,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
        ],
    ];
    pub const CONNECTION_INDEX: [[usize; 4]; 6] = [
        [0, 0, 0, 0],
        [1, 1, 1, 1],
        [2, 2, 2, 2],
        [2, 2, 2, 2],
        [2, 2, 2, 2],
        [3, 3, 3, 3],
    ];
}

use connection_data::{CONNECTION_BANKS, CONNECTION_INDEX};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub name: String,
    pub properties: BTreeMap<String, String>,
}
const SIDES: [[i32; 3]; 4] = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];
const SIDE_NAMES: [&str; 4] = ["north", "east", "south", "west"];

fn step(pos: [i32; 3], delta: [i32; 3]) -> Result<[i32; 3]> {
    Ok([
        pos[0]
            .checked_add(delta[0])
            .ok_or_else(|| Error::new("neighbor x overflow"))?,
        pos[1]
            .checked_add(delta[1])
            .ok_or_else(|| Error::new("neighbor y overflow"))?,
        pos[2]
            .checked_add(delta[2])
            .ok_or_else(|| Error::new("neighbor z overflow"))?,
    ])
}
fn set(out: &mut Descriptor, key: &str, value: impl Into<String>) {
    out.properties.insert(key.into(), value.into());
}
fn copy(out: &mut Descriptor, other: &Descriptor, key: &str) -> Result<()> {
    let value = other
        .properties
        .get(key)
        .ok_or_else(|| Error::new(format!("base descriptor missing {key}")))?;
    set(out, key, value.clone());
    Ok(())
}
fn connection_category(block: u16) -> Option<usize> {
    match block {
        85 | 188..=192 => Some(0),
        113 => Some(1),
        101 => Some(2),
        102 => Some(3),
        160 => Some(4),
        139 => Some(5),
        _ => None,
    }
}
fn connects(category: usize, side: usize, raw: u16) -> Result<bool> {
    if raw >= 4096 {
        return Err(Error::new("neighbor raw state exceeds vanilla47 table"));
    }
    let bank = CONNECTION_INDEX[category][side];
    let raw = usize::from(raw);
    Ok(CONNECTION_BANKS[bank][raw / 64] & (1u64 << (raw % 64)) != 0)
}

/// True for block families this module derives. Leaves' optional distance search
/// is separate because it has a wider refresh radius and is not a visual model key.
pub fn needs_context(raw: u16) -> bool {
    let block = raw >> 4;
    matches!(
        block,
        2 | 54 | 64 | 71 | 107 | 110 | 139 | 146 | 175 | 183..=197 | 101 | 102 | 113 | 160
    ) || (block == 3 && raw & 15 == 2)
        || connection_category(block).is_some()
        || stair(raw).is_some()
}

fn stair(raw: u16) -> Option<(usize, bool)> {
    matches!(
        raw >> 4,
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180
    )
    .then_some(([1, 3, 2, 0][usize::from(raw & 3)], raw & 4 != 0))
}

/// Old-client default for absent context, including metadata aliases whose modern
/// flattening table alone cannot represent the1.8 visual default.
pub fn temporary_base(raw: u16, base: impl Fn(u16) -> Result<Descriptor>) -> Result<Descriptor> {
    let mut out = if raw >> 4 == 175 && raw & 8 != 0 {
        let mut value = base(175 << 4)?;
        set(&mut value, "half", "upper");
        value
    } else {
        base(raw)?
    };
    if matches!(raw >> 4, 101 | 102 | 160) {
        for side in SIDE_NAMES {
            set(&mut out, side, "true");
        }
    }
    Ok(out)
}

/// None means a required neighbor is unknown (or a transient chest pair has not
/// become consistent). Keep the base state until the missing neighbor arrives;
/// never turn an unknown chunk into known air. Resolve the returned complete
/// current descriptor against the pinned factual block-state report.
pub fn derive(
    raw: u16,
    pos: [i32; 3],
    mut neighbor: impl FnMut([i32; 3]) -> Option<u16>,
    base: impl Fn(u16) -> Result<Descriptor>,
) -> Result<Option<Descriptor>> {
    let block = raw >> 4;
    let mut out = temporary_base(raw, &base)?;
    if matches!(block, 64 | 71 | 193..=197) {
        let upper = raw & 8 != 0;
        let Some(other) = neighbor(step(pos, [0, if upper { -1 } else { 1 }, 0])?) else {
            return Ok(None);
        };
        if other >> 4 == block {
            let other = base(other)?;
            for key in if upper {
                ["facing", "open"]
            } else {
                ["hinge", "powered"]
            } {
                copy(&mut out, &other, key)?;
            }
        }
    } else if block == 175 && raw & 8 != 0 {
        let Some(other) = neighbor(step(pos, [0, -1, 0])?) else {
            return Ok(None);
        };
        if other >> 4 == 175 {
            // Upper metadata ignores the low three variant bits in1.8. Invalid
            // lower variant6/7 aliases also decode to sunflower through base().
            out = base(if other & 8 != 0 { 175 << 4 } else { other })?;
            set(&mut out, "half", "upper");
        }
    } else if matches!(block, 2 | 110) || (block == 3 && raw & 15 == 2) {
        let Some(above) = neighbor(step(pos, [0, 1, 0])?) else {
            return Ok(None);
        };
        set(
            &mut out,
            "snowy",
            if matches!(above >> 4, 78 | 80) {
                "true"
            } else {
                "false"
            },
        );
    } else if let Some(category) = connection_category(block) {
        let mut connected = [false; 4];
        for (side, delta) in SIDES.into_iter().enumerate() {
            let Some(other) = neighbor(step(pos, delta)?) else {
                return Ok(None);
            };
            connected[side] = connects(category, side, other)?;
        }
        // The old all-disconnected pane model was a full cross. Current panes
        // need four explicit arms to select that same cross instead of a post.
        if matches!(category, 2..=4) && connected == [false; 4] {
            connected = [true; 4];
        }
        for side in 0..4 {
            set(
                &mut out,
                SIDE_NAMES[side],
                if block == 139 {
                    if connected[side] { "low" } else { "none" }
                } else if connected[side] {
                    "true"
                } else {
                    "false"
                },
            );
        }
        if block == 139 {
            let Some(above) = neighbor(step(pos, [0, 1, 0])?) else {
                return Ok(None);
            };
            let ns = connected == [true, false, true, false];
            let ew = connected == [false, true, false, true];
            set(
                &mut out,
                "up",
                if (!ns && !ew) || above >> 4 != 0 {
                    "true"
                } else {
                    "false"
                },
            );
        }
    } else if matches!(block, 107 | 183..=187) {
        let facing = out
            .properties
            .get("facing")
            .ok_or_else(|| Error::new("gate base has no facing"))?;
        let sides = match facing.as_str() {
            "north" | "south" => [1, 3],
            "east" | "west" => [0, 2],
            _ => return Err(Error::new("invalid gate facing")),
        };
        let mut in_wall = false;
        for side in sides {
            let Some(other) = neighbor(step(pos, SIDES[side])?) else {
                return Ok(None);
            };
            in_wall |= other >> 4 == 139;
        }
        set(&mut out, "in_wall", if in_wall { "true" } else { "false" });
    } else if let Some((facing, top)) = stair(raw) {
        let mut sides = [0; 4];
        for side in 0..4 {
            let Some(other) = neighbor(step(pos, SIDES[side])?) else {
                return Ok(None);
            };
            sides[side] = other;
        }
        let different = |side: usize| stair(sides[side]) != Some((facing, top));
        let mut shape = "straight";
        if let Some((other, other_top)) = stair(sides[facing])
            && top == other_top
            && other % 2 != facing % 2
            && different((other + 2) % 4)
        {
            shape = if other == (facing + 3) % 4 {
                "outer_left"
            } else {
                "outer_right"
            };
        } else if let Some((other, other_top)) = stair(sides[(facing + 2) % 4])
            && top == other_top
            && other % 2 != facing % 2
            && different(other)
        {
            shape = if other == (facing + 3) % 4 {
                "inner_left"
            } else {
                "inner_right"
            };
        }
        set(&mut out, "shape", shape);
    } else if matches!(block, 54 | 146) {
        let facing = out
            .properties
            .get("facing")
            .ok_or_else(|| Error::new("chest base has no facing"))?
            .clone();
        let clockwise = match facing.as_str() {
            "north" => 1,
            "east" => 2,
            "south" => 3,
            "west" => 0,
            _ => return Err(Error::new("invalid chest facing")),
        };
        let mut adjacent = None;
        for (side, delta) in SIDES.into_iter().enumerate() {
            let Some(other) = neighbor(step(pos, delta)?) else {
                return Ok(None);
            };
            if other >> 4 == block {
                let other = base(other)?;
                if other.properties.get("facing") != Some(&facing) {
                    return Ok(None);
                }
                if side != clockwise && side != (clockwise + 2) % 4 {
                    return Ok(None);
                }
                if adjacent.replace(side).is_some() {
                    return Err(Error::new("old chest has multiple pair candidates"));
                }
            }
        }
        set(
            &mut out,
            "type",
            match adjacent {
                Some(side) if side == clockwise => "left",
                Some(_) => "right",
                None => "single",
            },
        );
    }
    Ok(Some(out))
}

/// Optional current leaf distance, breadth-first up to distance6;7 means no nearby
/// old log. A missing explored neighbor returns None unless a nearer known log is
/// already established. Caller can cache this separately from immediate neighbors.
pub fn leaf_distance(
    pos: [i32; 3],
    mut neighbor: impl FnMut([i32; 3]) -> Option<u16>,
) -> Result<Option<u8>> {
    let mut seen = HashSet::from([pos]);
    let mut queue = VecDeque::from([(pos, 0u8)]);
    let mut missing_distance = None;
    while let Some((at, distance)) = queue.pop_front() {
        if distance >= 6 {
            continue;
        }
        for delta in [
            [0, -1, 0],
            [0, 1, 0],
            SIDES[0],
            SIDES[1],
            SIDES[2],
            SIDES[3],
        ] {
            let next = step(at, delta)?;
            if !seen.insert(next) {
                continue;
            }
            let next_distance = distance + 1;
            let Some(raw) = neighbor(next) else {
                missing_distance =
                    Some(missing_distance.map_or(next_distance, |old: u8| old.min(next_distance)));
                continue;
            };
            match raw >> 4 {
                17 | 162 => {
                    return Ok(
                        if missing_distance.is_none_or(|missing| missing >= next_distance) {
                            Some(next_distance)
                        } else {
                            None
                        },
                    );
                }
                18 | 161 => queue.push_back((next, next_distance)),
                _ => {}
            }
        }
    }
    Ok(missing_distance.is_none().then_some(7))
}

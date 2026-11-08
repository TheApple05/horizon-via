//! Protocol754 world effects and connection-local recipe aliases, from vanilla packet writers.
use super::{boolean, finish, read_slot, tables, text, up};
use crate::{
    Error, Result,
    buf::{Codec, Pos, Reader, Writer},
    entity_data::Particle,
    packets::{play, recipes},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn finite(value: f32) -> Result<f32> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| Error::new("nonfinite 1.16.5 effect"))
}
fn double(r: &mut Reader<'_>) -> Result<f64> {
    let value = r.f64()?;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| Error::new("nonfinite 1.16.5 coordinate"))
}
fn registry(kind: &str, name: &str) -> Result<i32> {
    super::names()
        .get(kind)
        .and_then(|ids| ids.id(name))
        .or_else(|| {
            super::super::v1_8_9::bootstrap::names()
                .get(kind)
                .and_then(|ids| ids.id(name))
        })
        .ok_or_else(|| Error::new(format!("unknown 1.16.5 effect registry {kind}/{name}")))
}
fn item_id(name: &str) -> Result<i32> {
    let raw = tables::ITEM_NAMES
        .iter()
        .position(|value| *value == name)
        .ok_or_else(|| Error::new("missing 1.16.5 recipe station"))?;
    up(tables::ITEM_UP, raw as i32)
}
fn seed() -> Result<i64> {
    let mut bytes = [0; 8];
    crate::crypto::random_bytes(&mut bytes)?;
    Ok(i64::from_ne_bytes(bytes))
}
fn source(r: &mut Reader<'_>) -> Result<i32> {
    let value = r.varint()?;
    (0..=9)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| Error::new("invalid old sound source"))
}
fn sound_name(r: &mut Reader<'_>) -> Result<String> {
    let raw = r.varint()?;
    usize::try_from(raw)
        .ok()
        .and_then(|raw| tables::SOUND_NAMES.get(raw))
        .filter(|name| !name.is_empty())
        .map(|name| (*name).into())
        .ok_or_else(|| Error::new("unknown 1.16.5 sound"))
}

#[derive(Clone, Default)]
pub(super) struct State {
    recipes: HashMap<String, Option<recipes::RecipeBookEntry>>,
    keys: Vec<String>,
    ids: HashMap<String, i32>,
    groups: BTreeMap<(i32, String), i32>,
    unlocked: BTreeSet<String>,
    highlighted: BTreeSet<String>,
    received_book: bool,
    smithing_base: BTreeSet<i32>,
    smithing_addition: BTreeSet<i32>,
}

impl State {
    pub(super) fn recipe_key(&self, display: i32) -> Result<&str> {
        let key = usize::try_from(display)
            .ok()
            .and_then(|id| self.keys.get(id))
            .ok_or_else(|| Error::new("unknown 1.16.5 recipe display"))?;
        if self
            .recipes
            .get(key)
            .and_then(|value| value.as_ref())
            .is_none()
        {
            return Err(Error::new("old recipe has no display"));
        }
        Ok(key)
    }

    pub(super) fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let mut out = Vec::new();
        match id {
            6 => out.push(C::AwardStats(stats(&mut r)?)),
            7 => {
                let pos = Pos::read(&mut r)?;
                let state = up(tables::STATE_UP, r.varint()?)?;
                let action = r.varint()?;
                boolean(&mut r)?;
                if !(0..=6).contains(&action) {
                    return Err(Error::new("invalid old dig acknowledgement"));
                }
                out.push(C::BlockUpdate(play::BlockUpdate { pos, state }));
            }
            12 => out.push(C::BossEvent(boss(&mut r)?)),
            15 => {
                let mut w = Writer::new();
                for _ in 0..3 {
                    let value = r.varint()?;
                    if value < 0 {
                        return Err(Error::new("invalid old suggestion range"));
                    }
                    w.varint(value);
                }
                let n = r.count(r.remaining())?;
                w.varint(n as i32);
                for _ in 0..n {
                    w.string(&r.string(32767)?);
                    let tooltip = boolean(&mut r)?;
                    w.bool(tooltip);
                    if tooltip {
                        w.nbt(&text(&mut r)?);
                    }
                }
                out.push(C::Unknown(
                    crate::ids::play::clientbound::COMMAND_SUGGESTIONS,
                    w.data,
                ));
            }
            24 | 81 => {
                let name = if id == 24 {
                    r.string(32767)?
                } else {
                    sound_name(&mut r)?
                };
                let source = source(&mut r)?;
                let (x, y, z) = (r.i32()?, r.i32()?, r.i32()?);
                let (volume, pitch) = (finite(r.f32()?)?, finite(r.f32()?)?);
                if volume < 0. || pitch < 0. {
                    return Err(Error::new("negative old sound value"));
                }
                out.push(C::Sound(play::Sound {
                    sound: (-1, Some((name, None))),
                    source,
                    x,
                    y,
                    z,
                    volume,
                    pitch,
                    seed: seed()?,
                }));
            }
            27 => out.extend(explosion(&mut r)?),
            33 => {
                let mut kind = r.i32()?;
                let pos = Pos::read(&mut r)?;
                let mut data = r.i32()?;
                let global = boolean(&mut r)?;
                if kind == 2001 {
                    data = up(tables::STATE_UP, data)?;
                } else if kind == 1010 && data == 0 {
                    kind = 1011;
                } else if kind == 1010 {
                    let item = usize::try_from(data)
                        .ok()
                        .and_then(|id| tables::ITEM_NAMES.get(id))
                        .copied()
                        .ok_or_else(|| Error::new("unknown old jukebox item"))?;
                    let name = item.strip_prefix("minecraft:").unwrap_or(item);
                    let name = name
                        .strip_prefix("music_disc_")
                        .ok_or_else(|| Error::new("non-disc old jukebox item"))?;
                    data = registry("minecraft:jukebox_song", &format!("minecraft:{name}"))?;
                }
                out.push(C::LevelEvent(play::LevelEvent {
                    kind,
                    pos,
                    data,
                    global,
                }));
            }
            34 => out.push(C::LevelParticles(particles(&mut r)?)),
            37 => out.push(C::MapItemData(map(&mut r)?)),
            47 => {
                let container = i32::from(r.u8()?);
                let key = r.string(32767)?;
                let display = self
                    .recipes
                    .get(&key)
                    .and_then(|entry| entry.as_ref())
                    .map(|entry| entry.display.clone())
                    .ok_or_else(|| Error::new("old ghost recipe has no display"))?;
                out.push(C::PlaceGhostRecipe(recipes::PlaceGhostRecipe {
                    container,
                    display,
                }));
            }
            49 => match r.varint()? {
                0 => {}
                1 => {
                    if r.varint()? < 0 {
                        return Err(Error::new("negative old combat duration"));
                    }
                    r.i32()?;
                }
                2 => {
                    let id = r.varint()?;
                    r.i32()?;
                    out.push(C::PlayerCombatKill(play::PlayerCombatKill {
                        id,
                        message: text(&mut r)?,
                    }));
                }
                _ => return Err(Error::new("unknown old combat event")),
            },
            53 => {
                let mut next = self.clone();
                out.extend(next.book(&mut r)?);
                finish(&r)?;
                *self = next;
                return Ok(Some(out));
            }
            61 => out.push(border(&mut r)?),
            67 => {
                let slot = i32::from(r.i8()?);
                if !(0..=18).contains(&slot) {
                    return Err(Error::new("invalid old scoreboard slot"));
                }
                out.push(C::SetDisplayObjective(play::SetDisplayObjective {
                    slot,
                    objective: r.string(32767)?,
                }));
            }
            74 => out.push(C::SetObjective(objective(&mut r)?)),
            76 => out.push(C::SetPlayerTeam(team(&mut r)?)),
            77 => {
                let owner = r.string(32767)?;
                let action = r.varint()?;
                let objective = r.string(32767)?;
                out.push(match action {
                    0 => C::SetScore(play::SetScore {
                        owner,
                        objective,
                        score: r.varint()?,
                        display: None,
                        number_format: None,
                    }),
                    1 => C::ResetScore(play::ResetScore {
                        owner,
                        objective: (!objective.is_empty()).then_some(objective),
                    }),
                    _ => return Err(Error::new("invalid old score action")),
                });
            }
            79 => out.push(match r.varint()? {
                0 => C::SetTitleText(play::Text {
                    text: text(&mut r)?,
                }),
                1 => C::SetSubtitleText(play::Text {
                    text: text(&mut r)?,
                }),
                2 => C::SetActionBarText(play::Text {
                    text: text(&mut r)?,
                }),
                3 => C::SetTitlesAnimation(play::TitlesAnimation {
                    fade_in: r.i32()?,
                    stay: r.i32()?,
                    fade_out: r.i32()?,
                }),
                4 => C::ClearTitles(play::Bool1 { value: false }),
                5 => C::ClearTitles(play::Bool1 { value: true }),
                _ => return Err(Error::new("unknown old title action")),
            }),
            80 => {
                let sound = (-1, Some((sound_name(&mut r)?, None)));
                let source = source(&mut r)?;
                let id = r.varint()?;
                let volume = finite(r.f32()?)?;
                let pitch = finite(r.f32()?)?;
                if id < 0 || volume < 0. || pitch < 0. {
                    return Err(Error::new("invalid old entity sound"));
                }
                out.push(C::SoundEntity(play::SoundEntity {
                    sound,
                    source,
                    id,
                    volume,
                    pitch,
                    seed: seed()?,
                }));
            }
            82 => {
                let flags = r.u8()?;
                if flags & !3 != 0 {
                    return Err(Error::new("invalid old stop sound flags"));
                }
                let source = if flags & 1 != 0 {
                    Some(source(&mut r)?)
                } else {
                    None
                };
                let sound = if flags & 2 != 0 {
                    Some(r.string(32767)?)
                } else {
                    None
                };
                out.push(C::StopSound(play::StopSound { source, sound }));
            }
            87 => out.push(C::UpdateAdvancements(advancements(&mut r)?)),
            90 => {
                let mut next = self.clone();
                next.declarations(&mut r)?;
                finish(&r)?;
                out.push(next.properties()?);
                if next.received_book {
                    out.push(C::RecipeBookAdd(next.current_book()));
                }
                *self = next;
                return Ok(Some(out));
            }
            _ => return Ok(None),
        }
        finish(&r)?;
        Ok(Some(out))
    }
}

fn stats(r: &mut Reader<'_>) -> Result<play::AwardStats> {
    let n = r.count(r.remaining() / 3)?;
    let mut stats = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let kind = r.varint()?;
        let raw = r.varint()?;
        let value = r.varint()?;
        let name = usize::try_from(kind)
            .ok()
            .and_then(|id| tables::STAT_TYPE_NAMES.get(id))
            .copied()
            .ok_or_else(|| Error::new("unknown old stat kind"))?;
        let table = match name {
            "minecraft:mined" => tables::BLOCK_UP,
            "minecraft:crafted"
            | "minecraft:used"
            | "minecraft:broken"
            | "minecraft:picked_up"
            | "minecraft:dropped" => tables::ITEM_UP,
            "minecraft:killed" | "minecraft:killed_by" => tables::ENTITY_UP,
            "minecraft:custom" => tables::CUSTOM_STAT_UP,
            _ => return Err(Error::new("unknown old stat registry")),
        };
        if value < 0 {
            return Err(Error::new("negative old statistic"));
        }
        let mapped = if name == "minecraft:custom"
            && tables::CUSTOM_STAT_NAMES.get(raw as usize) == Some(&"minecraft:play_one_minute")
        {
            registry("minecraft:custom_stat", "minecraft:play_time")?
        } else {
            up(table, raw)?
        };
        stats.push((up(tables::STAT_TYPE_UP, kind)?, mapped, value));
    }
    Ok(play::AwardStats { stats })
}

fn boss(r: &mut Reader<'_>) -> Result<play::BossEvent> {
    let id = r.uuid()?;
    fn style(r: &mut Reader<'_>) -> Result<(i32, i32)> {
        let a = r.varint()?;
        let b = r.varint()?;
        if !(0..=6).contains(&a) || !(0..=4).contains(&b) {
            return Err(Error::new("invalid old boss style"));
        }
        Ok((a, b))
    }
    fn flags(r: &mut Reader<'_>) -> Result<(bool, bool, bool)> {
        let flags = r.u8()?;
        if flags & !7 != 0 {
            return Err(Error::new("invalid old boss flags"));
        }
        Ok((flags & 1 != 0, flags & 2 != 0, flags & 4 != 0))
    }
    let action = match r.varint()? {
        0 => {
            let name = text(r)?;
            let progress = finite(r.f32()?)?;
            let (color, overlay) = style(r)?;
            let (darken, music, fog) = flags(r)?;
            play::BossAction::Add {
                name,
                progress,
                color,
                overlay,
                darken,
                music,
                fog,
            }
        }
        1 => play::BossAction::Remove,
        2 => play::BossAction::UpdateProgress {
            progress: finite(r.f32()?)?,
        },
        3 => play::BossAction::UpdateName { name: text(r)? },
        4 => {
            let (color, overlay) = style(r)?;
            play::BossAction::UpdateStyle { color, overlay }
        }
        5 => {
            let (darken, music, fog) = flags(r)?;
            play::BossAction::UpdateProperties { darken, music, fog }
        }
        _ => return Err(Error::new("unknown old boss action")),
    };
    Ok(play::BossEvent { id, action })
}

fn explosion(r: &mut Reader<'_>) -> Result<Vec<play::Clientbound>> {
    let center = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let radius = finite(r.f32()?)?;
    if radius < 0. {
        return Err(Error::new("negative old explosion radius"));
    }
    let n = r.i32()?;
    if n < 0 || n as usize > r.remaining().saturating_sub(12) / 3 {
        return Err(Error::new("invalid old explosion size"));
    }
    let origin = center.map(|value| value as i32);
    let mut blocks = Vec::with_capacity((n as usize).min(4096));
    for _ in 0..n {
        blocks.push([
            origin[0].wrapping_add(i32::from(r.i8()?)),
            origin[1].wrapping_add(i32::from(r.i8()?)),
            origin[2].wrapping_add(i32::from(r.i8()?)),
        ]);
    }
    let knockback = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let particle = Particle {
        id: registry(
            "minecraft:particle_type",
            if radius >= 2. {
                "minecraft:explosion_emitter"
            } else {
                "minecraft:explosion"
            },
        )?,
        data: None,
    };
    let mut out = vec![play::Clientbound::Explode(play::Explode {
        center,
        radius,
        block_count: n,
        knockback: Some(knockback),
        particle,
        sound: (-1, Some(("minecraft:entity.generic.explode".into(), None))),
        block_particles: vec![],
        play_sound: true,
    })];
    out.extend(
        blocks
            .into_iter()
            .map(|pos| play::Clientbound::BlockUpdate(play::BlockUpdate { pos, state: 0 })),
    );
    Ok(out)
}

fn particles(r: &mut Reader<'_>) -> Result<play::LevelParticles> {
    let raw = r.i32()?;
    let override_limiter = boolean(r)?;
    let pos = [double(r)?, double(r)?, double(r)?];
    let dist = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?];
    let speed = finite(r.f32()?)?;
    let count = r.i32()?;
    if count < 0 {
        return Err(Error::new("negative old particle count"));
    }
    let mut normalized = Writer::new();
    normalized.varint(raw);
    normalized.bytes(r.rest());
    let mut options = Reader::new(&normalized.data);
    let mut particle = super::entity::particle(&mut options)?;
    finish(&options)?;
    r.bytes(r.remaining())?;
    match tables::PARTICLE_NAMES.get(raw as usize) {
        Some(&"minecraft:entity_effect") => {
            particle.id = crate::entity_data::LEGACY_ENTITY_EFFECT_PARTICLE_ID;
            particle.data = None;
        }
        Some(&"minecraft:ambient_entity_effect") => {
            particle.id = crate::entity_data::LEGACY_AMBIENT_ENTITY_EFFECT_PARTICLE_ID;
            particle.data = None;
        }
        _ => {}
    }
    Ok(play::LevelParticles {
        particle,
        override_limiter,
        always_show: false,
        pos,
        dist,
        max_speed: [speed; 3],
        count,
        randomization: 0,
    })
}

fn map(r: &mut Reader<'_>) -> Result<play::MapItemData> {
    let map_id = r.varint()?;
    let scale = r.i8()?;
    boolean(r)?;
    let locked = boolean(r)?;
    if map_id < 0 || !(0..=4).contains(&scale) {
        return Err(Error::new("invalid old map"));
    }
    let n = r.count(r.remaining() / 5)?;
    let mut decorations = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let kind = r.varint()?;
        if !(0..=26).contains(&kind) {
            return Err(Error::new("unknown old map decoration"));
        }
        let x = r.i8()?;
        let y = r.i8()?;
        let rot = r.i8()?;
        if !(0..=15).contains(&rot) {
            return Err(Error::new("invalid old map rotation"));
        }
        let name = if boolean(r)? { Some(text(r)?) } else { None };
        decorations.push(play::MapDecoration {
            kind,
            x,
            y,
            rot,
            name,
        });
    }
    let width = i32::from(r.u8()?);
    let patch = if width == 0 {
        None
    } else {
        let height = i32::from(r.u8()?);
        let start_x = i32::from(r.u8()?);
        let start_y = i32::from(r.u8()?);
        let colors = r.byte_array(16384)?;
        if width > 128
            || height == 0
            || start_x + width > 128
            || start_y + height > 128
            || colors.len() != (width * height) as usize
        {
            return Err(Error::new("invalid old map patch"));
        }
        Some(play::MapPatch {
            start_x,
            start_y,
            width,
            height,
            colors,
        })
    };
    Ok(play::MapItemData {
        map_id,
        scale,
        locked,
        decorations: Some(decorations),
        patch,
    })
}

fn border(r: &mut Reader<'_>) -> Result<play::Clientbound> {
    use play::Clientbound as C;
    Ok(match r.varint()? {
        0 => C::SetBorderSize(play::SetBorderSize { size: double(r)? }),
        1 => C::SetBorderLerpSize(play::SetBorderLerpSize {
            old_size: double(r)?,
            new_size: double(r)?,
            lerp_time: r.varlong()?,
        }),
        2 => C::SetBorderCenter(play::SetBorderCenter {
            x: double(r)?,
            z: double(r)?,
        }),
        3 => C::InitializeBorder(play::InitializeBorder {
            center_x: double(r)?,
            center_z: double(r)?,
            old_size: double(r)?,
            new_size: double(r)?,
            lerp_time: r.varlong()?,
            absolute_max_size: r.varint()?,
            warning_blocks: r.varint()?,
            warning_time: r.varint()?,
        }),
        4 => C::SetBorderWarningDelay(play::SetBorderWarningDelay { delay: r.varint()? }),
        5 => C::SetBorderWarningDistance(play::SetBorderWarningDistance {
            blocks: r.varint()?,
        }),
        _ => return Err(Error::new("unknown old border action")),
    })
}

fn objective(r: &mut Reader<'_>) -> Result<play::SetObjective> {
    let name = r.string(32767)?;
    let method = r.i8()?;
    let display = match method {
        0 | 2 => {
            let text = text(r)?;
            let render = r.varint()?;
            if !(0..=1).contains(&render) {
                return Err(Error::new("invalid old objective render"));
            }
            Some((text, render, None))
        }
        1 => None,
        _ => return Err(Error::new("invalid old objective method")),
    };
    Ok(play::SetObjective {
        name,
        method,
        display,
    })
}

fn team(r: &mut Reader<'_>) -> Result<play::SetPlayerTeam> {
    let name = r.string(32767)?;
    let method = r.i8()?;
    if !(0..=4).contains(&method) {
        return Err(Error::new("invalid old team method"));
    }
    let parameters = if method == 0 || method == 2 {
        let display_name = text(r)?;
        let options = r.i8()?;
        let name_tag_visibility = match r.string(32767)?.as_str() {
            "always" => 0,
            "never" => 1,
            "hideForOtherTeams" => 2,
            "hideForOwnTeam" => 3,
            _ => return Err(Error::new("invalid old team visibility")),
        };
        let collision_rule = match r.string(32767)?.as_str() {
            "always" => 0,
            "never" => 1,
            "pushOtherTeams" => 2,
            "pushOwnTeam" => 3,
            _ => return Err(Error::new("invalid old team collision rule")),
        };
        let color = r.varint()?;
        if options & !3 != 0 || !(-1..=15).contains(&color) {
            return Err(Error::new("invalid old team color/options"));
        }
        let prefix = text(r)?;
        let suffix = text(r)?;
        Some(play::TeamParameters {
            display_name,
            prefix,
            suffix,
            name_tag_visibility,
            collision_rule,
            color: (color >= 0).then_some(color),
            options,
        })
    } else {
        None
    };
    let players = if matches!(method, 0 | 3 | 4) {
        let n = r.count(r.remaining())?;
        (0..n).map(|_| r.string(32767)).collect::<Result<_>>()?
    } else {
        vec![]
    };
    Ok(play::SetPlayerTeam {
        name,
        method,
        parameters,
        players,
    })
}

fn advancements(r: &mut Reader<'_>) -> Result<play::UpdateAdvancements> {
    let reset = boolean(r)?;
    let n = r.count(r.remaining())?;
    let mut added = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let id = r.string(32767)?;
        let parent = if boolean(r)? {
            Some(r.string(32767)?)
        } else {
            None
        };
        let (mut x, mut y) = (0., 0.);
        let display = if boolean(r)? {
            let title = text(r)?;
            let description = text(r)?;
            let icon = read_slot(r)?.ok_or_else(|| Error::new("empty old advancement icon"))?;
            let frame = r.varint()?;
            let flags = r.i32()?;
            if !(0..=2).contains(&frame) || flags & !7 != 0 {
                return Err(Error::new("invalid old advancement display"));
            }
            let background = if flags & 1 != 0 {
                Some(r.string(32767)?)
            } else {
                None
            };
            x = finite(r.f32()?)?;
            y = finite(r.f32()?)?;
            Some(play::AdvancementDisplay {
                title,
                description,
                icon,
                frame,
                show_toast: flags & 2 != 0,
                hidden: flags & 4 != 0,
                background,
            })
        } else {
            None
        };
        let n = r.count(r.remaining())?;
        for _ in 0..n {
            r.string(32767)?;
        }
        let n = r.count(r.remaining())?;
        let mut requirements = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let count = r.count(r.remaining())?;
            requirements.push((0..count).map(|_| r.string(32767)).collect::<Result<_>>()?);
        }
        added.push(play::Advancement {
            id,
            parent,
            display,
            requirements,
            telemetry: false,
            x,
            y,
        });
    }
    let n = r.count(r.remaining())?;
    let removed = (0..n).map(|_| r.string(32767)).collect::<Result<_>>()?;
    let n = r.count(r.remaining())?;
    let mut progress = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let id = r.string(32767)?;
        let count = r.count(r.remaining())?;
        let mut criteria = Vec::with_capacity(count.min(256));
        for _ in 0..count {
            let name = r.string(32767)?;
            let at = if boolean(r)? { Some(r.i64()?) } else { None };
            criteria.push((name, at));
        }
        progress.push((id, criteria));
    }
    Ok(play::UpdateAdvancements {
        reset,
        added,
        removed,
        progress,
        show: true,
    })
}

fn ingredient(r: &mut Reader<'_>) -> Result<(recipes::SlotDisplay, recipes::HolderSet)> {
    let n = r.count(65536)?;
    let mut values = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        if let Some(item) = read_slot(r)? {
            values.push(item);
        }
    }
    let ids = values
        .iter()
        .map(|item| item.item)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut displays = values
        .into_iter()
        .map(recipes::SlotDisplay::ItemStack)
        .collect::<Vec<_>>();
    let display = match displays.len() {
        0 => recipes::SlotDisplay::Empty,
        1 => displays.remove(0),
        _ => recipes::SlotDisplay::Composite(displays),
    };
    Ok((display, recipes::HolderSet::Items(ids)))
}
fn result(r: &mut Reader<'_>) -> Result<recipes::SlotDisplay> {
    read_slot(r)?
        .map(recipes::SlotDisplay::ItemStack)
        .ok_or_else(|| Error::new("empty old recipe result"))
}

impl State {
    fn alias(&mut self, key: &str) -> Result<i32> {
        if let Some(id) = self.ids.get(key) {
            return Ok(*id);
        }
        if self.keys.len() >= 65536 {
            return Err(Error::new("old recipe alias budget exceeded"));
        }
        let id = self.keys.len() as i32;
        self.keys.push(key.into());
        self.ids.insert(key.into(), id);
        Ok(id)
    }
    fn declarations(&mut self, r: &mut Reader<'_>) -> Result<()> {
        let n = r.count(65536)?;
        self.groups.clear();
        self.smithing_base.clear();
        self.smithing_addition.clear();
        let mut declarations = HashMap::new();
        for _ in 0..n {
            let kind = r.string(32767)?;
            let key = r.string(32767)?;
            if !tables::RECIPE_SERIALIZER_NAMES.contains(&kind.as_str()) {
                return Err(Error::new("unknown old recipe serializer"));
            }
            let id = self.alias(&key)?;
            let mut group = String::new();
            let mut requirements = Vec::new();
            let mut category = 3;
            let display = match kind.as_str() {
                "minecraft:crafting_shaped" | "minecraft:crafting_shapeless" => {
                    let shaped = kind.ends_with("_shaped");
                    let (width, height) = if shaped {
                        (r.varint()?, r.varint()?)
                    } else {
                        (0, 0)
                    };
                    if shaped && (!(1..=3).contains(&width) || !(1..=3).contains(&height)) {
                        return Err(Error::new("invalid old shaped recipe size"));
                    }
                    group = r.string(32767)?;
                    let count = if shaped {
                        (width * height) as usize
                    } else {
                        r.count(9)?
                    };
                    let mut ingredients = Vec::with_capacity(count);
                    for _ in 0..count {
                        let (display, set) = ingredient(r)?;
                        ingredients.push(display);
                        requirements.push(set);
                    }
                    let result = result(r)?;
                    let station = recipes::SlotDisplay::Item(item_id("minecraft:crafting_table")?);
                    Some(if shaped {
                        recipes::RecipeDisplay::Shaped {
                            width,
                            height,
                            ingredients,
                            result,
                            station,
                        }
                    } else {
                        recipes::RecipeDisplay::Shapeless {
                            ingredients,
                            result,
                            station,
                        }
                    })
                }
                "minecraft:smelting"
                | "minecraft:blasting"
                | "minecraft:smoking"
                | "minecraft:campfire_cooking" => {
                    group = r.string(32767)?;
                    let (input, set) = ingredient(r)?;
                    requirements.push(set);
                    let result = result(r)?;
                    let experience = finite(r.f32()?)?;
                    let duration = r.varint()?;
                    if duration < 0 || experience < 0. {
                        return Err(Error::new("invalid old cooking recipe"));
                    }
                    let (station, cat) = match kind.as_str() {
                        "minecraft:smelting" => ("minecraft:furnace", 6),
                        "minecraft:blasting" => ("minecraft:blast_furnace", 8),
                        "minecraft:smoking" => ("minecraft:smoker", 9),
                        _ => ("minecraft:campfire", 6),
                    };
                    category = cat;
                    Some(recipes::RecipeDisplay::Furnace {
                        ingredient: input,
                        fuel: recipes::SlotDisplay::AnyFuel,
                        result,
                        station: recipes::SlotDisplay::Item(item_id(station)?),
                        duration,
                        experience,
                    })
                }
                "minecraft:stonecutting" => {
                    group = r.string(32767)?;
                    let (input, set) = ingredient(r)?;
                    requirements.push(set);
                    let result = result(r)?;
                    category = 10;
                    Some(recipes::RecipeDisplay::Stonecutter {
                        input,
                        result,
                        station: recipes::SlotDisplay::Item(item_id("minecraft:stonecutter")?),
                    })
                }
                "minecraft:smithing" => {
                    let (_, base) = ingredient(r)?;
                    let (_, addition) = ingredient(r)?;
                    result(r)?;
                    if let recipes::HolderSet::Items(items) = base {
                        self.smithing_base.extend(items);
                    }
                    if let recipes::HolderSet::Items(items) = addition {
                        self.smithing_addition.extend(items);
                    }
                    None
                }
                name if name.starts_with("minecraft:crafting_special_") => None,
                _ => return Err(Error::new("unsupported old recipe serializer")),
            };
            let entry = if let Some(display) = display {
                let group_id = if group.is_empty() {
                    0
                } else {
                    let next = self.groups.len() as i32 + 1;
                    *self.groups.entry((category, group)).or_insert(next)
                };
                Some(recipes::RecipeBookEntry {
                    id,
                    display,
                    group: group_id,
                    category,
                    requirements: Some(requirements),
                    flags: 0,
                })
            } else {
                None
            };
            if declarations.insert(key, entry).is_some() {
                return Err(Error::new("duplicate old recipe key"));
            }
        }
        self.recipes = declarations;
        Ok(())
    }
    fn current_book(&self) -> recipes::RecipeBookAdd {
        let entries = self
            .unlocked
            .iter()
            .filter_map(|key| {
                self.recipes
                    .get(key)
                    .and_then(|entry| entry.clone())
                    .map(|mut entry| {
                        entry.flags = if self.highlighted.contains(key) { 3 } else { 0 };
                        entry
                    })
            })
            .collect();
        recipes::RecipeBookAdd {
            entries,
            replace: true,
        }
    }
    fn book(&mut self, r: &mut Reader<'_>) -> Result<Vec<play::Clientbound>> {
        let action = r.varint()?;
        if !(0..=2).contains(&action) {
            return Err(Error::new("invalid old recipe book action"));
        }
        let mut books = [(false, false); 4];
        for pair in &mut books {
            *pair = (boolean(r)?, boolean(r)?);
        }
        let n = r.count(65536)?;
        let keys = (0..n)
            .map(|_| r.string(32767))
            .collect::<Result<Vec<_>>>()?;
        let mut highlighted = BTreeSet::new();
        if action == 0 {
            let n = r.count(65536)?;
            for _ in 0..n {
                highlighted.insert(r.string(32767)?);
            }
        }
        let mut out = vec![play::Clientbound::RecipeBookSettings(
            recipes::RecipeBookSettings { books },
        )];
        if action == 2 {
            let mut ids = Vec::new();
            for key in keys {
                self.unlocked.remove(&key);
                self.highlighted.remove(&key);
                if let Some(id) = self.ids.get(&key) {
                    ids.push(*id);
                }
            }
            out.push(play::Clientbound::RecipeBookRemove(
                recipes::RecipeBookRemove { ids },
            ));
        } else {
            if action == 0 {
                self.unlocked.clear();
                self.highlighted = highlighted;
            }
            let mut entries = Vec::new();
            for key in keys {
                self.unlocked.insert(key.clone());
                if action == 1 {
                    self.highlighted.insert(key.clone());
                }
                if let Some(Some(entry)) = self.recipes.get(&key) {
                    let mut entry = entry.clone();
                    entry.flags = if self.highlighted.contains(&key) {
                        3
                    } else {
                        0
                    };
                    entries.push(entry);
                }
            }
            if self.unlocked.len() > 65536 {
                return Err(Error::new("old recipe book budget exceeded"));
            }
            out.push(play::Clientbound::RecipeBookAdd(recipes::RecipeBookAdd {
                entries,
                replace: action == 0,
            }));
        }
        self.received_book = true;
        Ok(out)
    }
    fn properties(&self) -> Result<play::Clientbound> {
        let mut sets: BTreeMap<&str, BTreeSet<i32>> = [
            "smithing_addition",
            "smithing_base",
            "smithing_template",
            "furnace_input",
            "blast_furnace_input",
            "smoker_input",
            "campfire_input",
            "brewing_input",
            "brewing_reagent",
        ]
        .into_iter()
        .map(|name| (name, BTreeSet::new()))
        .collect();
        sets.insert("smithing_base", self.smithing_base.clone());
        sets.insert("smithing_addition", self.smithing_addition.clone());
        let mut stonecutting = Vec::new();
        let mut keys = self.recipes.keys().collect::<Vec<_>>();
        keys.sort_unstable();
        for key in keys {
            if let Some(entry) = &self.recipes[key] {
                match &entry.display {
                    recipes::RecipeDisplay::Furnace {
                        ingredient,
                        station,
                        ..
                    } => {
                        let name = if *station
                            == recipes::SlotDisplay::Item(item_id("minecraft:blast_furnace")?)
                        {
                            "blast_furnace_input"
                        } else if *station
                            == recipes::SlotDisplay::Item(item_id("minecraft:smoker")?)
                        {
                            "smoker_input"
                        } else if *station
                            == recipes::SlotDisplay::Item(item_id("minecraft:campfire")?)
                        {
                            "campfire_input"
                        } else {
                            "furnace_input"
                        };
                        if let Some(requirements) = &entry.requirements {
                            for set in requirements {
                                if let recipes::HolderSet::Items(items) = set {
                                    sets.entry(name).or_default().extend(items);
                                }
                            }
                        }
                        let _ = ingredient;
                    }
                    recipes::RecipeDisplay::Stonecutter { result, .. } => {
                        if let Some(requirements) = &entry.requirements {
                            if let Some(set) = requirements.first() {
                                stonecutting.push((set, result));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut w = Writer::new();
        w.varint(sets.len() as i32);
        for (name, items) in sets {
            w.string(&format!("minecraft:{name}"));
            w.varint(items.len() as i32);
            for item in items {
                w.varint(item);
            }
        }
        w.varint(stonecutting.len() as i32);
        for (set, result) in stonecutting {
            set.write(&mut w);
            recipes::SlotDisplay::write(result, &mut w);
        }
        Ok(play::Clientbound::Unknown(
            crate::ids::play::clientbound::UPDATE_RECIPES,
            w.data,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod goldens {
        include!("worldfx_goldens.rs");
    }
    fn decode(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn actual754_writers_preserve_effect_values_and_exact_boundaries() {
        let mut state = State::default();
        for &(name, id, hex) in goldens::FIXTURES {
            if matches!(id, 53 | 90) {
                continue;
            }
            let bytes = decode(hex);
            let packets = state
                .clientbound(id, &bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"))
                .unwrap();
            assert!(!packets.is_empty(), "{name}");
            match &packets[0] {
                play::Clientbound::Sound(p) => {
                    assert_eq!((p.x, p.y, p.z), (10, 516, -22));
                    assert_eq!((p.volume, p.pitch), (0.75, 1.25));
                }
                play::Clientbound::SoundEntity(p) => assert_eq!(p.id, 123),
                play::Clientbound::LevelParticles(p) => {
                    assert_eq!(p.pos, [1.25, 64.5, -2.75]);
                    assert_eq!(p.count, 3);
                }
                play::Clientbound::Explode(p) => {
                    assert_eq!(p.center, [3.25, 64.5, -9.75]);
                    assert_eq!(p.knockback, Some([0.125, -0.25, 0.5]));
                }
                play::Clientbound::BossEvent(p) => assert!(matches!(
                    p.action,
                    play::BossAction::UpdateProperties {
                        darken: true,
                        music: false,
                        fog: true
                    }
                )),
                play::Clientbound::UpdateAdvancements(p) => assert!(!p.reset && p.added.is_empty()),
                _ => panic!("wrong effect family: {name}"),
            }
            for end in 0..bytes.len() {
                assert!(
                    state.clientbound(id, &bytes[..end]).is_err(),
                    "{name} truncated at {end}"
                );
            }
            let mut extra = bytes;
            extra.push(0);
            assert!(
                state.clientbound(id, &extra).is_err(),
                "{name} trailing data"
            );
        }
    }
    #[test]
    fn actual754_recipe_writers_keep_keys_ingredients_unlocks_and_packet_atomicity() {
        let _names = crate::adapter::ids::legacy_registries(super::super::names());
        let _items = crate::adapter::ids::legacy_items(std::sync::Arc::new(std::sync::Mutex::new(
            Default::default(),
        )));
        let get = |name| {
            decode(
                goldens::FIXTURES
                    .iter()
                    .find(|row| row.0 == name)
                    .unwrap()
                    .2,
            )
        };
        let mut state = State::default();
        let declarations = get("recipes_five_serializers");
        let packets = state.clientbound(90, &declarations).unwrap().unwrap();
        assert!(
            matches!(&packets[0],play::Clientbound::Unknown(id,_) if *id==crate::ids::play::clientbound::UPDATE_RECIPES)
        );
        assert_eq!(state.recipes.len(), 5);
        let book = get("recipe_book_init");
        let packets = state.clientbound(53, &book).unwrap().unwrap();
        let play::Clientbound::RecipeBookAdd(add) = &packets[1] else {
            panic!()
        };
        assert_eq!(add.entries.len(), 2);
        assert!(add.replace);
        let shaped = add
            .entries
            .iter()
            .find(|entry| state.recipe_key(entry.id).unwrap() == "synthetic:table")
            .unwrap();
        assert!(
            matches!(&shaped.display,recipes::RecipeDisplay::Shaped {width:2,height:2,ingredients,..} if ingredients.len()==4)
        );
        assert_eq!(shaped.flags, 3);
        let iron = add
            .entries
            .iter()
            .find(|entry| state.recipe_key(entry.id).unwrap() == "synthetic:iron")
            .unwrap();
        assert!(
            matches!(&iron.display,recipes::RecipeDisplay::Furnace {duration:200,experience,..} if (*experience-0.7).abs()<f32::EPSILON)
        );
        let groups = state.groups.len();
        let mut reloaded = declarations.clone();
        let group = b"synthetic-cooking";
        let at = reloaded
            .windows(group.len())
            .position(|bytes| bytes == group)
            .unwrap();
        reloaded[at + group.len() - 1] = b'X';
        state.clientbound(90, &reloaded).unwrap();
        assert_eq!(state.groups.len(), groups);
        assert_eq!(state.recipe_key(shaped.id).unwrap(), "synthetic:table");
        let mut ghost = Writer::new();
        ghost.u8(2);
        ghost.string("synthetic:table");
        assert!(
            matches!(&state.clientbound(47, &ghost.data).unwrap().unwrap()[0], play::Clientbound::PlaceGhostRecipe(p) if p.container == 2)
        );
        for (id, bytes) in [(90, declarations), (53, book)] {
            for end in 0..bytes.len() {
                assert!(
                    state.clientbound(id, &bytes[..end]).is_err(),
                    "old recipe packet truncated at {end}"
                );
                assert_eq!(state.recipes.len(), 5);
                assert_eq!(state.unlocked.len(), 2);
            }
            let mut extra = bytes;
            extra.push(0);
            assert!(state.clientbound(id, &extra).is_err());
        }
    }
    #[test]
    fn boss754_keeps_independent_music_and_fog_bits() {
        let mut w = Writer::new();
        w.uuid(17);
        w.varint(5);
        w.u8(5);
        let p = boss(&mut Reader::new(&w.data)).unwrap();
        assert!(matches!(
            p.action,
            play::BossAction::UpdateProperties {
                darken: true,
                music: false,
                fog: true
            }
        ));
    }
    #[test]
    fn old_score_and_title_actions_validate_exact_boundaries() {
        let mut state = State::default();
        let mut w = Writer::new();
        w.string("Probe");
        w.varint(0);
        w.string("objective");
        w.varint(37);
        assert!(
            matches!(&state.clientbound(77,&w.data).unwrap().unwrap()[0],play::Clientbound::SetScore(p) if p.score==37)
        );
        for n in 0..w.data.len() {
            assert!(state.clientbound(77, &w.data[..n]).is_err());
        }
        w.u8(0);
        assert!(state.clientbound(77, &w.data).is_err());
    }
}

#[cfg(test)]
mod smithing_recipe_tests {
    use super::*;
    #[test]
    fn actual_smithing_declarations_publish_all_additions_and_replace_them_on_reload() {
        let _names = crate::adapter::ids::legacy_registries(super::super::names());
        let _items = crate::adapter::ids::legacy_items(std::sync::Arc::new(std::sync::Mutex::new(
            Default::default(),
        )));
        let hex = "09126d696e6563726166743a736d697468696e67206d696e6563726166743a6e65746865726974655f6178655f736d697468696e670101de04010a000003000644616d61676500000000000101c504010001e304010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67226d696e6563726166743a6e65746865726974655f626f6f74735f736d697468696e670101fd04010a000003000644616d61676500000000000101c5040100018505010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67276d696e6563726166743a6e65746865726974655f6368657374706c6174655f736d697468696e670101fb04010a000003000644616d61676500000000000101c5040100018305010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67236d696e6563726166743a6e65746865726974655f68656c6d65745f736d697468696e670101fa04010a000003000644616d61676500000000000101c5040100018205010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67206d696e6563726166743a6e65746865726974655f686f655f736d697468696e670101df04010a000003000644616d61676500000000000101c504010001e404010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67256d696e6563726166743a6e65746865726974655f6c656767696e67735f736d697468696e670101fc04010a000003000644616d61676500000000000101c5040100018405010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67246d696e6563726166743a6e65746865726974655f7069636b6178655f736d697468696e670101dd04010a000003000644616d61676500000000000101c504010001e204010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67236d696e6563726166743a6e65746865726974655f73686f76656c5f736d697468696e670101dc04010a000003000644616d61676500000000000101c504010001e104010a000003000644616d6167650000000000126d696e6563726166743a736d697468696e67226d696e6563726166743a6e65746865726974655f73776f72645f736d697468696e670101db04010a000003000644616d61676500000000000101c504010001e004010a000003000644616d6167650000000000";
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let mut state = State::default();
        let packets = state.clientbound(90, &bytes).unwrap().unwrap();
        let play::Clientbound::Unknown(id, payload) = &packets[0] else {
            panic!()
        };
        assert_eq!(*id, crate::ids::play::clientbound::UPDATE_RECIPES);
        let mut r = Reader::new(payload);
        let mut properties = BTreeMap::new();
        for _ in 0..r.count(32).unwrap() {
            let name = r.string(32767).unwrap();
            let values = (0..r.count(1024).unwrap())
                .map(|_| r.varint().unwrap())
                .collect::<BTreeSet<_>>();
            properties.insert(name, values);
        }
        assert_eq!(
            properties["minecraft:smithing_addition"],
            BTreeSet::from([item_id("minecraft:netherite_ingot").unwrap()])
        );
        assert_eq!(properties["minecraft:smithing_base"].len(), 9);
        assert!(properties["minecraft:smithing_template"].is_empty());
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(r.remaining(), 0);
        for end in 0..bytes.len() {
            assert!(state.clientbound(90, &bytes[..end]).is_err());
            assert_eq!(state.smithing_base.len(), 9);
            assert_eq!(state.smithing_addition.len(), 1);
        }
        state.clientbound(90, &[0]).unwrap();
        assert!(state.smithing_addition.is_empty() && state.smithing_base.is_empty());
    }
}

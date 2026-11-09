//! Original protocol47 visible-world packet candidate. All output values are pinned777.
use crate::{
    Error, Result,
    buf::Reader,
    entity_data::{Particle, Value, serializer as s},
    item::Stack,
    packets::play,
};
use horizon_world::nbt::Tag;
use std::collections::{BTreeMap, BTreeSet};
pub mod flight_math;
pub mod particles;
mod sounds;

pub trait Context {
    fn registry(&self, registry: &str, name: &str) -> Result<i32>;
    fn block(&self, wire: u16) -> Result<i32>;
    fn item(&self, id: i16, damage: i16) -> Result<Stack>;
    /// Root should reuse item::v1_20_5::modifier_id(uuid, "") to preserve vanilla modifier identities.
    fn modifier_id(&self, uuid: u128) -> String;
    fn random_seed(&self) -> i64 {
        0
    }
}
#[derive(Default, Debug)]
pub struct Output {
    pub packets: Vec<play::Clientbound>,
    /// Caller also updates the connection's old-terrain cache so a later partial chunk cannot resurrect blocks.
    pub block_changes: Vec<([i32; 3], u16)>,
    pub known_noop: Option<&'static str>,
}
fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 world packet bytes"))
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
        Err(Error::new("negative entity id"))
    } else {
        Ok(id)
    }
}
fn finite(v: f32) -> Result<f32> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::new("non-finite protocol47 float"))
    }
}
fn double(r: &mut Reader<'_>) -> Result<f64> {
    let v = r.f64()?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::new("non-finite protocol47 double"))
    }
}
fn legacy_pos(v: i64) -> [i32; 3] {
    [
        (v >> 38) as i32,
        ((v << 26) >> 52) as i32,
        ((v << 38) >> 38) as i32,
    ]
}

fn attribute_name(old: &str) -> Option<&'static str> {
    Some(match old {
        "generic.maxHealth" => "minecraft:max_health",
        "generic.followRange" => "minecraft:follow_range",
        "generic.knockbackResistance" => "minecraft:knockback_resistance",
        "generic.movementSpeed" => "minecraft:movement_speed",
        "generic.attackDamage" => "minecraft:attack_damage",
        "horse.jumpStrength" => "minecraft:jump_strength",
        "zombie.spawnReinforcements" => "minecraft:spawn_reinforcements",
        _ => return None,
    })
}
pub fn attributes(payload: &[u8], ctx: &impl Context) -> Result<play::UpdateAttributes> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let count = r.i32()?;
    if count < 0 || count as usize > r.remaining() / 11 {
        return Err(Error::new("invalid protocol47 attribute count"));
    }
    let mut attributes = Vec::new();
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let name = r.string(64)?;
        let canonical = attribute_name(&name)
            .ok_or_else(|| Error::new(format!("unsupported protocol47 attribute {name}")))?;
        let attribute = ctx.registry("minecraft:attribute", canonical)?;
        if !seen.insert(attribute) {
            return Err(Error::new("duplicate protocol47 attribute"));
        }
        let base = double(&mut r)?;
        let count = r.count(r.remaining() / 25)?;
        let mut modifiers = Vec::new();
        let mut ids = BTreeSet::new();
        for _ in 0..count {
            let uuid = r.uuid()?;
            let amount = double(&mut r)?;
            let operation = i32::from(r.u8()?);
            if operation > 2 || !ids.insert(uuid) {
                return Err(Error::new("invalid or duplicate protocol47 modifier"));
            }
            modifiers.push(play::AttributeModifier {
                id: ctx.modifier_id(uuid),
                amount,
                operation,
            });
        }
        attributes.push(play::AttributeSnapshot {
            attribute,
            base,
            modifiers,
        });
    }
    finish(&r)?;
    Ok(play::UpdateAttributes {
        id,
        values: attributes,
    })
}
pub fn explosion(payload: &[u8], ctx: &impl Context) -> Result<Output> {
    let mut r = Reader::new(payload);
    let center = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let radius = finite(r.f32()?)?;
    if radius < 0. {
        return Err(Error::new("negative protocol47 explosion radius"));
    }
    let count = r.i32()?;
    if count < 0 || count as usize > r.remaining().saturating_sub(12) / 3 {
        return Err(Error::new("invalid protocol47 explosion count"));
    }
    let origin = center.map(|v| v as i32); // Vanilla47 writer/reader use Java (int), truncating negative coordinates toward zero.
    let mut positions = Vec::with_capacity((count as usize).min(256));
    for _ in 0..count {
        positions.push([
            origin[0].wrapping_add(i32::from(r.i8()?)),
            origin[1].wrapping_add(i32::from(r.i8()?)),
            origin[2].wrapping_add(i32::from(r.i8()?)),
        ]);
    }
    let knockback = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    finish(&r)?;
    let particle = Particle {
        id: ctx.registry(
            "minecraft:particle_type",
            if radius >= 2. {
                "minecraft:explosion_emitter"
            } else {
                "minecraft:explosion"
            },
        )?,
        data: None,
    };
    let mut packets = vec![play::Clientbound::Explode(play::Explode {
        center,
        radius,
        block_count: count,
        knockback: Some(knockback),
        particle,
        sound: (-1, Some(("minecraft:entity.generic.explode".into(), None))),
        block_particles: Vec::new(),
        play_sound: true,
    })];
    packets.extend(
        positions
            .iter()
            .map(|&pos| play::Clientbound::BlockUpdate(play::BlockUpdate { pos, state: 0 })),
    );
    Ok(Output {
        packets,
        block_changes: positions.into_iter().map(|pos| (pos, 0)).collect(),
        ..Default::default()
    })
}
pub fn sound(payload: &[u8], ctx: &impl Context) -> Result<play::Sound> {
    let mut r = Reader::new(payload);
    let name = r.string(256)?;
    let x = r.i32()?;
    let y = r.i32()?;
    let z = r.i32()?;
    let volume = finite(r.f32()?)?;
    let pitch = f32::from(r.u8()?) / 63.;
    finish(&r)?;
    if volume < 0. {
        return Err(Error::new("negative protocol47 sound volume"));
    }
    let old = name.strip_prefix("minecraft:").unwrap_or(&name);
    let (_, current, source) = sounds::ALIASES
        .iter()
        .find(|row| row.0 == old)
        .ok_or_else(|| Error::new(format!("unsupported protocol47 sound {name}")))?;
    Ok(play::Sound {
        sound: (-1, Some(((*current).into(), None))),
        source: *source,
        x,
        y,
        z,
        volume,
        pitch,
        seed: ctx.random_seed(),
    })
}
/// Old auxiliary sound numbers are not current LevelEvent numbers. Emit the actual old sound choice.
pub fn auxiliary_event(payload: &[u8], ctx: &impl Context) -> Result<Output> {
    let mut r = Reader::new(payload);
    let old = r.i32()?;
    let pos = legacy_pos(r.i64()?);
    r.i32()?;
    let global = boolean(&mut r)?;
    finish(&r)?;
    if global {
        if matches!(old, 1013 | 1018) {
            return Err(Error::new(
                "protocol47 global boss sound requires listener-relative coordinates",
            ));
        }
        if !matches!(old, 1000..=1012 | 1014..=1017 | 1020..=1022 | 2000..=2005) {
            return Err(Error::new(format!(
                "unsupported protocol47 global event {old}"
            )));
        }
        return Ok(Output {
            known_noop: Some("vanilla47 global event handler only recognizes1013/1018"),
            ..Default::default()
        });
    }
    // These ids belong exclusively to the old global handler; the local handler ignores them.
    if matches!(old, 1013 | 1018) {
        return Ok(Output {
            known_noop: Some("vanilla47 local event handler ignores global boss sound ids"),
            ..Default::default()
        });
    }
    let seed = ctx.random_seed();
    let first = (seed as u64 & 0xff_ffff) as f32 / 16_777_216.;
    let second = ((seed as u64 >> 24) & 0xff_ffff) as f32 / 16_777_216.;
    let random = 1. + (first - second) * 0.2;
    let slight = 0.9 + first * 0.1;
    let (name, volume, pitch) = match old {
        1000 => ("random.click", 1., 1.),
        1001 => ("random.click", 1., 1.2),
        1002 => ("random.bow", 1., 1.2),
        1003 => ("random.door_open", 1., slight),
        1004 => ("random.fizz", 0.5, 2.6 + (first - second) * 0.8),
        1006 => ("random.door_close", 1., slight),
        1007 => ("mob.ghast.charge", 10., random),
        1008 => ("mob.ghast.fireball", 10., random),
        1009 => ("mob.ghast.fireball", 2., random),
        1010 => ("mob.zombie.wood", 2., random),
        1011 => ("mob.zombie.metal", 2., random),
        1012 => ("mob.zombie.woodbreak", 2., random),
        1014 => ("mob.wither.shoot", 2., random),
        1015 => ("mob.bat.takeoff", 0.05, random),
        1016 => ("mob.zombie.infect", 2., random),
        1017 => ("mob.zombie.unfect", 2., random),
        1020 => ("random.anvil_break", 1., slight),
        1021 => ("random.anvil_use", 1., slight),
        1022 => ("random.anvil_land", 0.3, slight),
        _ => {
            return Ok(Output {
                packets: vec![play::Clientbound::LevelEvent(level_event(payload, ctx)?)],
                ..Default::default()
            });
        }
    };
    let (_, name, source) = sounds::ALIASES
        .iter()
        .find(|row| row.0 == name)
        .ok_or_else(|| Error::new("missing protocol47 auxiliary sound alias"))?;
    Ok(Output {
        packets: vec![play::Clientbound::Sound(play::Sound {
            sound: (-1, Some(((*name).into(), None))),
            source: *source,
            x: pos[0] * 8 + 4,
            y: pos[1] * 8 + 4,
            z: pos[2] * 8 + 4,
            volume,
            pitch,
            seed,
        })],
        ..Default::default()
    })
}
pub fn level_event(payload: &[u8], ctx: &impl Context) -> Result<play::LevelEvent> {
    let mut r = Reader::new(payload);
    let old = r.i32()?;
    let pos = legacy_pos(r.i64()?);
    let data = r.i32()?;
    let global = boolean(&mut r)?;
    finish(&r)?;
    let data = if old == 2001 {
        let state = data & 0xffff;
        ctx.block((((state & 4095) << 4) | ((state >> 12) & 15)) as u16)?
    } else {
        data
    };
    // These ids retain their primary-client behavior. Old jukebox/potion/smoke data needs separate conversion.
    if !matches!(old,1000..=1002|2001|2003..=2004) {
        return Err(Error::new(format!(
            "unsupported protocol47 level event {old}"
        )));
    }
    Ok(play::LevelEvent {
        kind: old,
        pos,
        data,
        global,
    })
}
pub fn lightning(payload: &[u8], uuid: u128, ctx: &impl Context) -> Result<play::AddEntity> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    if r.u8()? != 1 {
        return Err(Error::new("unsupported protocol47 global entity"));
    }
    let position = [r.i32()?, r.i32()?, r.i32()?].map(|v| f64::from(v) / 32.);
    finish(&r)?;
    Ok(play::AddEntity {
        id,
        uuid,
        kind: ctx.registry("minecraft:entity_type", "minecraft:lightning_bolt")?,
        position,
        velocity: [0.; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}
pub fn map(payload: &[u8], ctx: &impl Context) -> Result<play::MapItemData> {
    let mut r = Reader::new(payload);
    let map_id = r.varint()?;
    if map_id < 0 {
        return Err(Error::new("negative protocol47 map id"));
    }
    let scale = r.i8()?;
    if !(0..=4).contains(&scale) {
        return Err(Error::new("invalid protocol47 map scale"));
    }
    let count = r.count(r.remaining() / 3)?;
    let mut decorations = Vec::new();
    for _ in 0..count {
        let packed = r.u8()?;
        let name = match packed >> 4 {
            0 => "player",
            1 => "frame",
            2 => "red_marker",
            3 => "blue_marker",
            4 => "target_x",
            5 => "target_point",
            6 => "player_off_map",
            7 => "player_off_limits",
            _ => return Err(Error::new("unsupported protocol47 map decoration")),
        };
        let kind = ctx.registry(
            "minecraft:map_decoration_type",
            &format!("minecraft:{name}"),
        )?;
        decorations.push(play::MapDecoration {
            kind,
            x: r.i8()?,
            y: r.i8()?,
            rot: (packed & 15) as i8,
            name: None,
        });
    }
    let width = i32::from(r.u8()?);
    let patch = if width == 0 {
        None
    } else {
        let height = i32::from(r.u8()?);
        let start_x = i32::from(r.u8()?);
        let start_y = i32::from(r.u8()?);
        if height <= 0 || width + start_x > 128 || height + start_y > 128 {
            return Err(Error::new("invalid protocol47 map patch bounds"));
        }
        let colors = r.byte_array(16384)?;
        if colors.len() != (width * height) as usize {
            return Err(Error::new(
                "protocol47 map color count disagrees with bounds",
            ));
        }
        Some(play::MapPatch {
            start_x,
            start_y,
            width,
            height,
            colors,
        })
    };
    finish(&r)?;
    Ok(play::MapItemData {
        map_id,
        scale,
        locked: false,
        decorations: Some(decorations),
        patch,
    })
}

#[derive(Default)]
pub struct State {
    sleeping: BTreeMap<i32, [i32; 3]>,
}
impl State {
    pub fn clear(&mut self) {
        self.sleeping.clear();
    }
    /// Run after entity47 output: a metadata-only crouch update must not wake a sleeping player.
    pub fn filter_entity_packet(&mut self, packet: &mut play::Clientbound) {
        match packet {
            play::Clientbound::RemoveEntities(data) => {
                for id in &data.ids {
                    self.sleeping.remove(id);
                }
            }
            play::Clientbound::SetEntityData(data) => {
                if let Some(&pos) = self.sleeping.get(&data.id) {
                    data.values.retain(|row| !matches!(row.0, 6 | 14));
                    data.values.extend([
                        (6, s::POSE, Value::Id(2)),
                        (14, s::OPTIONAL_BLOCK_POS, Value::OptPos(Some(pos))),
                    ]);
                }
            }
            _ => {}
        }
    }
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        ctx: &impl Context,
        mut uuid: impl FnMut(i32) -> u128,
    ) -> Result<Option<Output>> {
        let output = match id {
            10 => {
                let mut r = Reader::new(payload);
                let id = entity_id(&mut r)?;
                let pos = legacy_pos(r.i64()?);
                finish(&r)?;
                self.sleeping.insert(id, pos);
                Output {
                    packets: vec![play::Clientbound::SetEntityData(play::SetEntityData {
                        id,
                        values: vec![
                            (6, s::POSE, Value::Id(2)),
                            (14, s::OPTIONAL_BLOCK_POS, Value::OptPos(Some(pos))),
                        ],
                    })],
                    ..Default::default()
                }
            }
            11 => {
                let mut r = Reader::new(payload);
                let id = entity_id(&mut r)?;
                let action = r.u8()?;
                finish(&r)?;
                match action {
                    1 => Output {
                        packets: vec![play::Clientbound::HurtAnimation(play::HurtAnimation {
                            id,
                            yaw: 0.,
                        })],
                        ..Default::default()
                    },
                    2 => {
                        self.sleeping.remove(&id);
                        Output {
                            packets: vec![
                                play::Clientbound::SetEntityData(play::SetEntityData {
                                    id,
                                    values: vec![
                                        (6, s::POSE, Value::Id(0)),
                                        (14, s::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
                                    ],
                                }),
                                play::Clientbound::Animate(play::Animate { id, action: 2 }),
                            ],
                            ..Default::default()
                        }
                    }
                    0 | 4 | 5 => Output {
                        packets: vec![play::Clientbound::Animate(play::Animate { id, action })],
                        ..Default::default()
                    },
                    3 => Output {
                        known_noop: Some("vanilla47 client ignores animation3"),
                        ..Default::default()
                    },
                    _ => return Err(Error::new("unsupported protocol47 animation")),
                }
            }
            32 => Output {
                packets: vec![play::Clientbound::UpdateAttributes(attributes(
                    payload, ctx,
                )?)],
                ..Default::default()
            },
            39 => explosion(payload, ctx)?,
            40 => auxiliary_event(payload, ctx)?,
            41 => Output {
                packets: vec![play::Clientbound::Sound(sound(payload, ctx)?)],
                ..Default::default()
            },
            42 => Output {
                packets: vec![play::Clientbound::LevelParticles(particles::read(
                    payload, ctx,
                )?)],
                ..Default::default()
            },
            44 => {
                let mut r = Reader::new(payload);
                let id = entity_id(&mut r)?;
                Output {
                    packets: vec![play::Clientbound::AddEntity(lightning(
                        payload,
                        uuid(id),
                        ctx,
                    )?)],
                    ..Default::default()
                }
            }
            52 => Output {
                packets: vec![play::Clientbound::MapItemData(map(payload, ctx)?)],
                ..Default::default()
            },
            70 => {
                let mut r = Reader::new(payload);
                let threshold = r.varint()?;
                finish(&r)?;
                return Err(Error::new(format!(
                    "protocol47 play compression requires connection directive threshold{threshold}"
                )));
            }
            73 => {
                let mut r = Reader::new(payload);
                entity_id(&mut r)?;
                let tag = r.named_nbt()?;
                if !matches!(tag, Tag::End | Tag::Compound(_)) {
                    return Err(Error::new("invalid protocol47 entity NBT"));
                }
                finish(&r)?;
                Output {
                    known_noop: Some(
                        "vanilla47 Entity.g(NBT) is a no-op, with no overrides across121 vanilla entity classes",
                    ),
                    ..Default::default()
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests;

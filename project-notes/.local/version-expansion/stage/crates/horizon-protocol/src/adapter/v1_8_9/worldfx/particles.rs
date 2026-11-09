use super::{Context, Error, Result, boolean, finish, finite};
use crate::{
    buf::Reader,
    entity_data::{Particle, ParticleData},
    packets::play,
};

fn named(id: i32) -> Option<&'static str> {
    Some(match id {
        0 => "poof",
        1 => "explosion",
        2 => "explosion_emitter",
        3 => "firework",
        4 => "bubble",
        5 => "splash",
        6 => "fishing",
        7 => "underwater",
        9 => "crit",
        10 => "enchanted_hit",
        11 => "smoke",
        12 => "large_smoke",
        13 => "effect",
        14 => "instant_effect",
        15 | 16 => "entity_effect",
        17 => "witch",
        18 => "dripping_water",
        19 => "dripping_lava",
        20 => "angry_villager",
        21 => "happy_villager",
        22 => "mycelium",
        23 => "note",
        24 => "portal",
        25 => "enchant",
        26 => "flame",
        27 => "lava",
        29 => "cloud",
        30 => "dust",
        31 => "item_snowball",
        33 => "item_slime",
        34 => "heart",
        35 => "block_marker",
        36 => "item",
        37 => "block",
        38 => "falling_dust",
        39 => "rain",
        41 => "elder_guardian",
        _ => return None,
    })
}
fn color(rgb: [f32; 3]) -> i32 {
    rgb.into_iter().fold(0, |value, c| {
        (value << 8) | ((c.clamp(0., 1.) * 255.).round() as i32)
    })
}
pub fn read(payload: &[u8], ctx: &impl Context) -> Result<play::LevelParticles> {
    let mut r = Reader::new(payload);
    let old = r.i32()?;
    if !(0..=41).contains(&old) {
        return Err(Error::new("unknown protocol47 particle id"));
    }
    let override_limiter = boolean(&mut r)?;
    let pos = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let mut dist = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?];
    let speed = finite(r.f32()?)?;
    let count = r.i32()?;
    if count < 0 {
        return Err(Error::new("negative protocol47 particle count"));
    }
    let arguments = match old {
        36 => vec![r.varint()?, r.varint()?],
        37 | 38 => vec![r.varint()?],
        _ => Vec::new(),
    };
    finish(&r)?;
    let name = named(old)
        .ok_or_else(|| Error::new(format!("unsupported removed protocol47 particle {old}")))?;
    let mut max_speed = [speed; 3];
    let data = match old {
        13 | 14 => Some(ParticleData::ColorPower(-1, 1.)),
        15 | 16 | 30 => {
            if count != 0 {
                return Err(Error::new(
                    "protocol47 per-particle randomized colors require explicit expansion",
                ));
            }
            let mut rgb = dist.map(|value| value * speed);
            if old == 30 && rgb[0] == 0. {
                rgb[0] = 1.;
            }
            let packed = color(rgb);
            max_speed = [0.; 3];
            dist = [0.; 3];
            if old == 30 {
                Some(ParticleData::ColorScale(packed, 1.))
            } else {
                Some(ParticleData::Color(
                    packed | ((if old == 16 { 0x26u32 } else { 0xffu32 }) << 24) as i32,
                ))
            }
        }
        35 => Some(ParticleData::State(ctx.block(166 << 4)?)),
        36 => {
            let id = i16::try_from(arguments[0])
                .map_err(|_| Error::new("invalid old item particle id"))?;
            let damage = i16::try_from(arguments[1])
                .map_err(|_| Error::new("invalid old item particle damage"))?;
            if id < 0 {
                return Err(Error::new("negative old item particle id"));
            }
            Some(ParticleData::Item(ctx.item(id, damage)?))
        }
        37 | 38 => {
            let packed = arguments[0];
            if !(0..=65535).contains(&packed) {
                return Err(Error::new("invalid old block particle state"));
            }
            Some(ParticleData::State(ctx.block(
                (((packed & 4095) << 4) | ((packed >> 12) & 15)) as u16,
            )?))
        }
        _ => None,
    };
    Ok(play::LevelParticles {
        particle: Particle {
            id: ctx.registry("minecraft:particle_type", &format!("minecraft:{name}"))?,
            data,
        },
        override_limiter,
        always_show: false,
        pos,
        dist,
        max_speed,
        count,
        randomization: 0,
    })
}

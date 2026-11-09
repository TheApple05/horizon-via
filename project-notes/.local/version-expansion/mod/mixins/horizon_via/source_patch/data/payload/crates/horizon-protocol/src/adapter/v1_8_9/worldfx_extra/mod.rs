//! Supplemental protocol47 visuals, pinned777 output. Dispatch before frozen worldfx47.
use super::worldfx as base;
use crate::{
    Error, Result,
    buf::Reader,
    entity_data::{Particle, ParticleData},
    packets::play,
};
use horizon_core::random::{LegacyRandom, RandomSource};
pub mod bounds;
mod potion;

pub trait Context: base::Context {
    /// The old terrain cache's exact (numeric block id <<4)|metadata.
    fn raw_block(&self, pos: [i32; 3]) -> Result<u16>;
    /// Camera entity's base position (not eye height); old global sounds use this.
    fn listener_position(&self) -> Result<[f64; 3]>;
}
#[derive(Default, Debug)]
pub struct Output {
    pub packets: Vec<play::Clientbound>,
    pub known_noop: Option<&'static str>,
    /// Explicitly recognized visual effect with no faithful current equivalent/context.
    pub omitted_cosmetic: Option<&'static str>,
}
fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 extra effect bytes"))
    }
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid protocol47 extra effect boolean")),
    }
}
fn finite(v: f32) -> Result<f32> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::new("non-finite protocol47 extra effect float"))
    }
}
fn position(v: i64) -> [i32; 3] {
    [
        (v >> 38) as i32,
        ((v << 26) >> 52) as i32,
        ((v << 38) >> 38) as i32,
    ]
}
fn particle(
    ctx: &impl Context,
    name: &str,
    data: Option<ParticleData>,
    pos: [f64; 3],
    velocity: [f64; 3],
    override_limiter: bool,
) -> Result<play::Clientbound> {
    Ok(play::Clientbound::LevelParticles(play::LevelParticles {
        particle: Particle {
            id: ctx.registry("minecraft:particle_type", &format!("minecraft:{name}"))?,
            data,
        },
        override_limiter,
        always_show: false,
        pos,
        dist: velocity.map(|v| v as f32),
        max_speed: [1.; 3],
        count: 0,
        randomization: 0,
    }))
}
fn sound(
    ctx: &impl Context,
    name: &str,
    source: i32,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
) -> Result<play::Clientbound> {
    if position
        .iter()
        .any(|v| !v.is_finite() || v * 8. < f64::from(i32::MIN) || v * 8. > f64::from(i32::MAX))
    {
        return Err(Error::new(
            "legacy sound position exceeds pinned coordinates",
        ));
    }
    let [x, y, z] = position.map(|v| (v * 8.) as i32);
    Ok(play::Clientbound::Sound(play::Sound {
        sound: (-1, Some((name.into(), None))),
        source,
        x,
        y,
        z,
        volume,
        pitch,
        seed: ctx.random_seed(),
    }))
}
fn rgb(color: i32, brightness: f32) -> i32 {
    [16, 8, 0].into_iter().fold(0, |out, shift| {
        (out << 8)
            | ((((color >> shift) & 255) as f32 * brightness)
                .round()
                .clamp(0., 255.) as i32)
    })
}
pub fn potion_color(damage: i32) -> (i32, bool) {
    potion::COLORS[(damage & 15) as usize]
}
pub fn clientbound(id: i32, payload: &[u8], ctx: &impl Context) -> Result<Option<Output>> {
    match id {
        40 => event(payload, ctx),
        42 => old_particle(payload, ctx),
        _ => Ok(None),
    }
}
pub fn event(payload: &[u8], ctx: &impl Context) -> Result<Option<Output>> {
    let mut r = Reader::new(payload);
    let old = r.i32()?;
    let pos = position(r.i64()?);
    let data = r.i32()?;
    let global = boolean(&mut r)?;
    finish(&r)?;
    if global {
        if !matches!(old, 1013 | 1018) {
            return Ok(None);
        }
        let listener = ctx.listener_position()?;
        if listener.iter().any(|v| !v.is_finite()) {
            return Err(Error::new("non-finite legacy listener position"));
        }
        let direction = std::array::from_fn::<_, 3, _>(|k| f64::from(pos[k]) - listener[k]);
        let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
        let at = if length > 0. {
            std::array::from_fn(|k| listener[k] + direction[k] / length * 2.)
        } else {
            listener
        };
        return Ok(Some(Output {
            packets: vec![sound(
                ctx,
                if old == 1013 {
                    "minecraft:entity.wither.spawn"
                } else {
                    "minecraft:entity.ender_dragon.death"
                },
                5,
                at,
                if old == 1013 { 1. } else { 5. },
                1.,
            )?],
            ..Default::default()
        }));
    }
    let mut out = Output::default();
    let mut random = LegacyRandom::new(ctx.random_seed());
    match old {
        1005 => {
            const RECORDS: [&str; 12] = [
                "13", "cat", "blocks", "chirp", "far", "mall", "mellohi", "stal", "strad", "ward",
                "11", "wait",
            ];
            let record = data
                .checked_sub(2256)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| RECORDS.get(i));
            let (kind, data) = if let Some(record) = record {
                (
                    1010,
                    ctx.registry("minecraft:jukebox_song", &format!("minecraft:{record}"))?,
                )
            } else {
                (1011, 0)
            };
            out.packets
                .push(play::Clientbound::LevelEvent(play::LevelEvent {
                    kind,
                    pos,
                    data,
                    global: false,
                }));
        }
        2000 => {
            let dx = f64::from(data % 3 - 1);
            let dz = f64::from(data / 3 % 3 - 1);
            let origin = [
                f64::from(pos[0]) + dx * 0.6 + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + dz * 0.6 + 0.5,
            ];
            for _ in 0..10 {
                let impulse = random.next_double() * 0.2 + 0.01;
                let at = [
                    origin[0] + dx * 0.01 + (random.next_double() - 0.5) * dz * 0.5,
                    origin[1] + (random.next_double() - 0.5) * 0.5,
                    origin[2] + dz * 0.01 + (random.next_double() - 0.5) * dx * 0.5,
                ];
                let velocity = [
                    dx * impulse + random.next_gaussian() * 0.01,
                    -0.03 + random.next_gaussian() * 0.01,
                    dz * impulse + random.next_gaussian() * 0.01,
                ];
                out.packets
                    .push(particle(ctx, "smoke", None, at, velocity, false)?);
            }
        }
        2002 => {
            let at = pos.map(f64::from);
            let damage = i16::try_from(data).map_err(|_| {
                Error::new("legacy potion crumb damage exceeds numeric slot context")
            })?;
            let crumb = ParticleData::Item(ctx.item(373, damage)?);
            for _ in 0..8 {
                let v = [
                    random.next_gaussian() * 0.15,
                    random.next_double() * 0.2,
                    random.next_gaussian() * 0.15,
                ];
                out.packets
                    .push(particle(ctx, "item", Some(crumb.clone()), at, v, false)?);
            }
            let (color, instant) = potion_color(data);
            for _ in 0..100 {
                let radius = random.next_double() * 4.;
                let angle = random.next_double() * std::f64::consts::PI * 2.;
                let velocity = [
                    angle.cos() * radius,
                    0.01 + random.next_double() * 0.5,
                    angle.sin() * radius,
                ];
                let at = [
                    at[0] + velocity[0] * 0.1,
                    at[1] + 0.3,
                    at[2] + velocity[2] * 0.1,
                ];
                let brightness = 0.75_f32 + random.next_float() * 0.25_f32;
                out.packets.push(particle(
                    ctx,
                    if instant { "instant_effect" } else { "effect" },
                    Some(ParticleData::ColorPower(
                        rgb(color, brightness),
                        radius as f32,
                    )),
                    at,
                    velocity,
                    false,
                )?);
            }
            let pitch = random.next_float() * 0.1_f32 + 0.9_f32;
            out.packets.push(sound(
                ctx,
                "minecraft:entity.splash_potion.break",
                7,
                pos.map(|v| f64::from(v) + 0.5),
                1.,
                pitch,
            )?);
        }
        2005 => {
            let count = if data == 0 { 15 } else { data };
            if count > 4096 {
                return Err(Error::new(
                    "legacy bonemeal count exceeds explicit4096 limit",
                ));
            }
            let raw = ctx.raw_block(pos)?;
            let bounds = bounds::lookup(raw)?;
            if bounds.air || count < 0 {
                out.known_noop =
                    Some("vanilla47 bonemeal emits no particles for air or negative count");
            } else if bounds.requires_neighbors {
                out.omitted_cosmetic = Some(
                    "legacy bonemeal bounds depend on neighbours; explicit context is unavailable",
                );
            } else {
                for _ in 0..count {
                    let velocity = [
                        random.next_gaussian() * 0.02,
                        random.next_gaussian() * 0.02,
                        random.next_gaussian() * 0.02,
                    ];
                    let at = [
                        f64::from(pos[0] as f32 + random.next_float()),
                        f64::from(pos[1]) + f64::from(random.next_float()) * bounds.height,
                        f64::from(pos[2] as f32 + random.next_float()),
                    ];
                    out.packets
                        .push(particle(ctx, "happy_villager", None, at, velocity, false)?);
                }
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(out))
}
fn old_particle(payload: &[u8], ctx: &impl Context) -> Result<Option<Output>> {
    let mut r = Reader::new(payload);
    let kind = r.i32()?;
    if !matches!(kind, 8 | 15 | 16 | 28 | 30 | 32 | 40) {
        return Ok(None);
    }
    let override_limiter = boolean(&mut r)?;
    let pos = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let dist = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?];
    let speed = finite(r.f32()?)?;
    let count = r.i32()?;
    finish(&r)?;
    if count < 0 {
        return Err(Error::new("negative legacy particle count"));
    }
    let mut out = Output::default();
    match kind {
        8 => {
            let mut aliased = payload.to_vec();
            aliased[..4].copy_from_slice(&22i32.to_be_bytes());
            out.packets
                .push(play::Clientbound::LevelParticles(base::particles::read(
                    &aliased, ctx,
                )?));
        }
        28 | 32 => {
            out.omitted_cosmetic = Some(if kind == 28 {
                "legacy footprint has no pinned777 particle equivalent"
            } else {
                "legacy snowshovel has no faithful pinned777 particle equivalent"
            })
        }
        40 => out.known_noop = Some("vanilla47 ParticleManager registers no factory for take40"),
        15 | 16 | 30 => {
            if count == 0 {
                return Ok(None);
            }
            if count > 4096 {
                return Err(Error::new(
                    "legacy colored particle count exceeds explicit4096 limit",
                ));
            }
            let mut random = LegacyRandom::new(ctx.random_seed());
            for _ in 0..count {
                let at =
                    std::array::from_fn(|k| pos[k] + random.next_gaussian() * f64::from(dist[k]));
                let mut color =
                    std::array::from_fn::<_, 3, _>(|_| random.next_gaussian() * f64::from(speed));
                if kind == 30 && color[0] == 0. {
                    color[0] = 1.;
                }
                let packed = color.into_iter().fold(0, |out, c| {
                    (out << 8) | ((c.clamp(0., 1.) * 255.).round() as i32)
                });
                let data = if kind == 30 {
                    ParticleData::ColorScale(packed, 1.)
                } else {
                    ParticleData::Color(
                        packed | ((if kind == 16 { 0x26u32 } else { 0xffu32 }) << 24) as i32,
                    )
                };
                out.packets.push(particle(
                    ctx,
                    if kind == 30 { "dust" } else { "entity_effect" },
                    Some(data),
                    at,
                    [0.; 3],
                    override_limiter,
                )?);
            }
        }
        _ => unreachable!(),
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests;

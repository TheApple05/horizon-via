//! Protocol340 effects and advancement layouts; output uses pinned consumer IDs.
use super::{boolean, facts, finish};
use crate::{
    Error, Result,
    adapter::v1_8_9::{worldfx as old, worldfx_extra},
    buf::Reader,
    entity_data::{Particle, ParticleData},
    item::Stack,
    packets::{play, recipes},
};
use std::collections::BTreeSet;

struct Context(i64);
impl old::Context for Context {
    fn registry(&self, kind: &str, name: &str) -> Result<i32> {
        registry(kind, name)
    }
    fn block(&self, raw: u16) -> Result<i32> {
        facts::STATES
            .get(usize::from(raw))
            .copied()
            .filter(|id| *id >= 0)
            .ok_or_else(|| Error::new("unmapped protocol340 effect block"))
    }
    fn item(&self, id: i16, damage: i16) -> Result<Stack> {
        crate::item::v1_20_3::from_numeric_slot_at(
            crate::item::item47::Raw47Slot {
                id,
                damage,
                count: 1,
                tag: horizon_world::nbt::Tag::End,
            },
            facts::old_item_name,
            1343,
        )
    }
    fn modifier_id(&self, uuid: u128) -> String {
        crate::item::v1_20_5::modifier_id(uuid, "")
    }
    fn random_seed(&self) -> i64 {
        self.0
    }
}
impl worldfx_extra::Context for Context {
    fn raw_block(&self, _: [i32; 3]) -> Result<u16> {
        Err(Error::new("effect requires terrain context"))
    }
    fn listener_position(&self) -> Result<[f64; 3]> {
        Err(Error::new("effect requires listener context"))
    }
}
fn registry(kind: &str, name: &str) -> Result<i32> {
    super::names()
        .get(kind)
        .and_then(|ids| ids.id(name))
        .ok_or_else(|| Error::new(format!("missing effect registry value {kind}/{name}")))
}
fn finite(value: f32) -> Result<f32> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| Error::new("non-finite protocol340 effect value"))
}

pub fn clientbound(id: i32, payload: &[u8]) -> Result<Option<old::Output>> {
    if !matches!(
        id,
        0x0c | 0x19 | 0x1c | 0x21 | 0x22 | 0x31 | 0x49 | 0x4d | 0x4e
    ) {
        return Ok(None);
    }
    let mut seed = [0; 8];
    crate::crypto::random_bytes(&mut seed)?;
    let ctx = Context(i64::from_ne_bytes(seed));
    let mut out = old::Output::default();
    let packet = match id {
        0x1c => return old::explosion(payload, &ctx).map(Some),
        0x4e => play::Clientbound::UpdateAttributes(attributes(payload, &ctx)?),
        0x19 | 0x49 => play::Clientbound::Sound(sound(payload, id == 0x19, ctx.0)?),
        0x22 => {
            out.packets = particles(payload, &ctx)?;
            return Ok(Some(out));
        }
        0x0c => play::Clientbound::BossEvent(boss(payload)?),
        0x21 => play::Clientbound::LevelEvent(level_event(payload, &ctx)?),
        0x4d => play::Clientbound::UpdateAdvancements(advancements(payload)?),
        0x31 => {
            out.known_noop = Some("protocol340 numeric recipe display mapping is unavailable");
            play::Clientbound::RecipeBookSettings(recipe_settings(payload)?)
        }
        _ => unreachable!(),
    };
    out.packets.push(packet);
    Ok(Some(out))
}

fn attributes(payload: &[u8], ctx: &Context) -> Result<play::UpdateAttributes> {
    use old::Context as _;
    let mut r = Reader::new(payload);
    let id = r.varint()?;
    let count = r.i32()?;
    if id < 0 || count < 0 || count as usize > r.remaining() / 11 {
        return Err(Error::new("invalid protocol340 attribute count/entity"));
    }
    let mut values = Vec::new();
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let name = r.string(64)?;
        let name = match name.as_str() {
            "generic.maxHealth" => "minecraft:max_health",
            "generic.followRange" => "minecraft:follow_range",
            "generic.knockbackResistance" => "minecraft:knockback_resistance",
            "generic.movementSpeed" => "minecraft:movement_speed",
            "generic.flyingSpeed" => "minecraft:flying_speed",
            "generic.attackDamage" => "minecraft:attack_damage",
            "generic.attackSpeed" => "minecraft:attack_speed",
            "generic.armor" => "minecraft:armor",
            "generic.armorToughness" => "minecraft:armor_toughness",
            "generic.luck" => "minecraft:luck",
            "horse.jumpStrength" => "minecraft:jump_strength",
            "zombie.spawnReinforcements" => "minecraft:spawn_reinforcements",
            _ => {
                return Err(Error::new(format!(
                    "unsupported protocol340 attribute {name}"
                )));
            }
        };
        let attribute = registry("minecraft:attribute", name)?;
        let base = r.f64()?;
        if !base.is_finite() || !seen.insert(attribute) {
            return Err(Error::new("invalid or duplicate protocol340 attribute"));
        }
        let count = r.count(r.remaining() / 25)?;
        let mut modifiers = Vec::new();
        let mut uuids = BTreeSet::new();
        for _ in 0..count {
            let uuid = r.uuid()?;
            let amount = r.f64()?;
            let operation = i32::from(r.u8()?);
            if !amount.is_finite() || operation > 2 || !uuids.insert(uuid) {
                return Err(Error::new("invalid or duplicate protocol340 modifier"));
            }
            modifiers.push(play::AttributeModifier {
                id: ctx.modifier_id(uuid),
                amount,
                operation,
            });
        }
        values.push(play::AttributeSnapshot {
            attribute,
            base,
            modifiers,
        });
    }
    finish(&r)?;
    Ok(play::UpdateAttributes { id, values })
}

fn sound(payload: &[u8], named: bool, seed: i64) -> Result<play::Sound> {
    let mut r = Reader::new(payload);
    let name = if named {
        r.string(256)?
    } else {
        facts::lookup(facts::SOUNDS, r.varint()?)
            .ok_or_else(|| Error::new("unknown protocol340 sound"))?
            .into()
    };
    let source = r.varint()?;
    let x = r.i32()?;
    let y = r.i32()?;
    let z = r.i32()?;
    let volume = finite(r.f32()?)?;
    let pitch = finite(r.f32()?)?;
    finish(&r)?;
    if !(0..=9).contains(&source) || volume < 0. || pitch < 0. {
        return Err(Error::new("invalid protocol340 sound fields"));
    }
    // Pre-flattening names changed for the small set of renamed vanilla entities.
    let name = name
        .replace("entity.zombie_pig.", "entity.zombified_piglin.")
        .replace("entity.illusion_illager.", "entity.illusioner.")
        .replace("entity.evocation_illager.", "entity.evoker.")
        .replace("entity.vindication_illager.", "entity.vindicator.")
        .replace("entity.snowman.", "entity.snow_golem.");
    Ok(play::Sound {
        sound: (-1, Some((name, None))),
        source,
        x,
        y,
        z,
        volume,
        pitch,
        seed,
    })
}

fn particles(payload: &[u8], ctx: &Context) -> Result<Vec<play::Clientbound>> {
    let mut r = Reader::new(payload);
    let kind = r.i32()?;
    if !(0..=48).contains(&kind) {
        return Err(Error::new("unknown protocol340 particle"));
    }
    if kind <= 41 {
        if let Some(extra) = worldfx_extra::clientbound(42, payload, ctx)? {
            return Ok(extra.packets);
        }
        let mut normalized = payload.to_vec();
        if kind == 38 {
            normalized[..4].copy_from_slice(&37i32.to_be_bytes());
        }
        return Ok(vec![play::Clientbound::LevelParticles(
            old::particles::read(&normalized, ctx)?,
        )]);
    }
    if kind == 46 {
        let mut normalized = payload.to_vec();
        normalized[..4].copy_from_slice(&38i32.to_be_bytes());
        return Ok(vec![play::Clientbound::LevelParticles(
            old::particles::read(&normalized, ctx)?,
        )]);
    }
    let override_limiter = boolean(&mut r)?;
    let pos = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?].map(f64::from);
    let dist = [finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?];
    let speed = finite(r.f32()?)?;
    let count = r.i32()?;
    finish(&r)?;
    if count < 0 {
        return Err(Error::new("negative protocol340 particle count"));
    }
    let name = match kind {
        42 => "dragon_breath",
        43 => "end_rod",
        44 => "damage_indicator",
        45 => "sweep_attack",
        47 => "totem_of_undying",
        48 => "spit",
        _ => unreachable!(),
    };
    Ok(vec![play::Clientbound::LevelParticles(
        play::LevelParticles {
            particle: Particle {
                id: registry("minecraft:particle_type", &format!("minecraft:{name}"))?,
                data: (kind == 42).then_some(ParticleData::Scalar(1.)),
            },
            override_limiter,
            always_show: false,
            pos,
            dist,
            max_speed: [speed; 3],
            count,
            randomization: 0,
        },
    )])
}

fn level_event(payload: &[u8], ctx: &Context) -> Result<play::LevelEvent> {
    use old::Context as _;
    let mut r = Reader::new(payload);
    let mut kind = r.i32()?;
    let packed = r.i64()?;
    let pos = [
        (packed >> 38) as i32,
        ((packed << 26) >> 52) as i32,
        ((packed << 38) >> 38) as i32,
    ];
    let mut data = r.i32()?;
    let global = boolean(&mut r)?;
    finish(&r)?;
    if kind == 2001 {
        if !(0..=65535).contains(&data) {
            return Err(Error::new("invalid protocol340 block event state"));
        }
        data = ctx.block((((data & 4095) << 4) | ((data >> 12) & 15)) as u16)?;
    } else if kind == 1010 {
        if data == 0 {
            kind = 1011;
        } else {
            let name = facts::lookup(facts::ITEMS, data)
                .and_then(|name| name.strip_prefix("minecraft:record_"))
                .ok_or_else(|| Error::new("unknown protocol340 music record"))?;
            data = registry("minecraft:jukebox_song", &format!("minecraft:{name}"))?;
        }
    }
    Ok(play::LevelEvent {
        kind,
        pos,
        data,
        global,
    })
}

fn boss(payload: &[u8]) -> Result<play::BossEvent> {
    let mut r = Reader::new(payload);
    let id = r.uuid()?;
    let action = match r.varint()? {
        0 => {
            let name = crate::adapter::read_legacy_component(&mut r)?;
            let progress = finite(r.f32()?)?;
            let (color, overlay) = style(&mut r)?;
            let (darken, music, fog) = flags(&mut r)?;
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
        3 => play::BossAction::UpdateName {
            name: crate::adapter::read_legacy_component(&mut r)?,
        },
        4 => {
            let (color, overlay) = style(&mut r)?;
            play::BossAction::UpdateStyle { color, overlay }
        }
        5 => {
            let (darken, music, fog) = flags(&mut r)?;
            play::BossAction::UpdateProperties { darken, music, fog }
        }
        _ => return Err(Error::new("unknown protocol340 boss operation")),
    };
    finish(&r)?;
    Ok(play::BossEvent { id, action })
}
fn style(r: &mut Reader<'_>) -> Result<(i32, i32)> {
    let color = r.varint()?;
    let overlay = r.varint()?;
    if !(0..=6).contains(&color) || !(0..=4).contains(&overlay) {
        return Err(Error::new("invalid old boss style"));
    }
    Ok((color, overlay))
}
fn flags(r: &mut Reader<'_>) -> Result<(bool, bool, bool)> {
    let flags = r.u8()?;
    if flags & !3 != 0 {
        return Err(Error::new("invalid old boss flags"));
    }
    // Vanilla340 uses bit2 for both music and fog, in both its reader and writer.
    Ok((flags & 1 != 0, flags & 2 != 0, flags & 2 != 0))
}

fn recipe_settings(payload: &[u8]) -> Result<recipes::RecipeBookSettings> {
    let mut r = Reader::new(payload);
    let action = r.varint()?;
    if !(0..=2).contains(&action) {
        return Err(Error::new("invalid protocol340 recipe action"));
    }
    let crafting = (boolean(&mut r)?, boolean(&mut r)?);
    for _ in 0..if action == 0 { 2 } else { 1 } {
        let n = r.count(65536)?;
        for _ in 0..n {
            if r.varint()? < 0 {
                return Err(Error::new("negative old recipe ID"));
            }
        }
    }
    finish(&r)?;
    Ok(recipes::RecipeBookSettings {
        books: [crafting, (false, false), (false, false), (false, false)],
    })
}

fn advancements(payload: &[u8]) -> Result<play::UpdateAdvancements> {
    let mut r = Reader::new(payload);
    let reset = boolean(&mut r)?;
    let n = r.count(r.remaining())?;
    let mut added = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let id = r.string(32767)?;
        let parent = if boolean(&mut r)? {
            Some(r.string(32767)?)
        } else {
            None
        };
        let mut x = 0.;
        let mut y = 0.;
        let display = if boolean(&mut r)? {
            let title = crate::adapter::read_legacy_component(&mut r)?;
            let description = crate::adapter::read_legacy_component(&mut r)?;
            let raw = crate::item::item47::read(&mut r)?
                .ok_or_else(|| Error::new("empty advancement icon"))?;
            let icon = crate::item::v1_20_3::from_numeric_slot_at(raw, facts::old_item_name, 1343)?;
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
        let criteria = r.count(r.remaining())?;
        for _ in 0..criteria {
            r.string(32767)?;
        }
        let groups = r.count(r.remaining())?;
        let mut requirements = Vec::with_capacity(groups.min(256));
        for _ in 0..groups {
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
    let count = r.count(r.remaining())?;
    let removed = (0..count).map(|_| r.string(32767)).collect::<Result<_>>()?;
    let count = r.count(r.remaining())?;
    let mut progress = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let id = r.string(32767)?;
        let count = r.count(r.remaining())?;
        let mut criteria = Vec::with_capacity(count.min(256));
        for _ in 0..count {
            let name = r.string(32767)?;
            let at = if boolean(&mut r)? {
                Some(r.i64()?)
            } else {
                None
            };
            criteria.push((name, at));
        }
        progress.push((id, criteria));
    }
    finish(&r)?;
    Ok(play::UpdateAdvancements {
        reset,
        added,
        removed,
        progress,
        show: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::Writer;

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
    fn actual_vanilla_writers_cover_all_particle_ids_and_packet_boundaries() {
        let _names = crate::adapter::ids::legacy_registries(super::super::names());
        let _items = crate::adapter::ids::legacy_items(std::sync::Arc::new(std::sync::Mutex::new(
            Default::default(),
        )));
        for &(name, id, hex) in goldens::FIXTURES {
            let bytes = decode(hex);
            let out = clientbound(id, &bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"))
                .unwrap();
            if !matches!(
                name,
                "particle_footstep" | "particle_snow_shovel" | "particle_item_take"
            ) {
                assert!(!out.packets.is_empty(), "{name}");
            }
            for end in 0..bytes.len() {
                assert!(
                    clientbound(id, &bytes[..end]).is_err(),
                    "{name}, truncated at {end}"
                );
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(clientbound(id, &extra).is_err(), "{name} trailing bytes");
        }
    }

    #[test]
    fn actual_attribute_sound_and_explosion_values_survive_translation() {
        let get = |name| {
            decode(
                goldens::FIXTURES
                    .iter()
                    .find(|row| row.0 == name)
                    .unwrap()
                    .2,
            )
        };
        let attrs = attributes(&get("attributes_two"), &Context(1)).unwrap();
        assert_eq!(attrs.id, 123);
        assert_eq!(attrs.values.len(), 2);
        assert_eq!(attrs.values[0].base, 40.);
        assert_eq!(attrs.values[0].modifiers[0].amount, 2.5);
        assert_eq!(attrs.values[0].modifiers[1].operation, 2);
        let sound = sound(&get("named_sound_custom"), true, 71).unwrap();
        assert_eq!(sound.sound, (-1, Some(("synthetic:chime".into(), None))));
        assert_eq!((sound.x, sound.y, sound.z), (10, 516, -22));
        assert_eq!((sound.volume, sound.pitch, sound.source), (0.75, 1.25, 4));
        let explosion = old::explosion(&get("explosion_two_blocks"), &Context(1)).unwrap();
        assert_eq!(
            explosion.block_changes,
            vec![([4, 65, -9], 0), ([2, 64, -10], 0)]
        );
        assert!(
            matches!(&explosion.packets[0], play::Clientbound::Explode(p) if p.knockback == Some([0.125, -0.25, 0.5]))
        );
    }

    #[test]
    fn actual_rich_advancement_keeps_requirements_coordinates_and_progress() {
        let _names = crate::adapter::ids::legacy_registries(super::super::names());
        let _items = crate::adapter::ids::legacy_items(std::sync::Arc::new(std::sync::Mutex::new(
            Default::default(),
        )));
        let row = goldens::FIXTURES
            .iter()
            .find(|row| row.0 == "advancements_rich")
            .unwrap();
        let packet = advancements(&decode(row.2)).unwrap();
        assert!(packet.reset);
        let advancement = &packet.added[0];
        assert_eq!(advancement.id, "synthetic:root");
        assert_eq!((advancement.x, advancement.y), (1.5, -2.25));
        assert_eq!(
            advancement.requirements,
            vec![vec!["first", "second"], vec!["second"]]
        );
        assert_eq!(
            packet.progress[0].1,
            vec![
                ("first".into(), Some(1_234_567_890_000)),
                ("second".into(), None)
            ]
        );
        assert_eq!(packet.removed, vec!["synthetic:removed"]);
        assert_eq!(advancement.display.as_ref().unwrap().icon.count, 3);
    }

    #[test]
    fn boss_json_and_style_preserve_boundaries_and_reject_trailing_data() {
        let mut w = Writer::new();
        w.uuid(17);
        w.varint(0);
        w.string("{\"text\":\"Old boss\"}");
        w.f32(0.75);
        w.varint(4);
        w.varint(2);
        w.u8(3);
        let p = boss(&w.data).unwrap();
        assert_eq!(p.id, 17);
        assert!(matches!(
            p.action,
            play::BossAction::Add {
                progress: 0.75,
                color: 4,
                overlay: 2,
                darken: true,
                music: true,
                fog: true,
                ..
            }
        ));
        for n in 0..w.data.len() {
            assert!(boss(&w.data[..n]).is_err());
        }
        w.u8(0);
        assert!(boss(&w.data).is_err());
    }

    #[test]
    fn recipe_lists_are_consumed_with_action_specific_boundaries() {
        let mut w = Writer::new();
        w.varint(0);
        w.bool(true);
        w.bool(false);
        w.varint(2);
        w.varint(3);
        w.varint(700);
        w.varint(1);
        w.varint(700);
        assert_eq!(recipe_settings(&w.data).unwrap().books[0], (true, false));
        w.u8(0);
        assert!(recipe_settings(&w.data).is_err());
        assert!(recipe_settings(&[3]).is_err());
    }
}

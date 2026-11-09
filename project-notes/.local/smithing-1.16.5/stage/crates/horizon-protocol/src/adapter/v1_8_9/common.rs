//! Original protocol47 common packet bridge. Output values use pinned777 types/ids.
//! Call outside older-protocol id/layout guards and do not translate these packets twice.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
    packets::play,
};
use horizon_world::{
    datafix::{self, dynamic as d, schema::Ty},
    nbt::Tag,
};
use std::collections::BTreeMap;

/// Crate-private text conversion and connection-local mappings stay with the integrator.
pub trait Context {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag>;
    fn dimension(&self) -> &str;
    fn block_state(&self, raw: i32, pos: [i32; 3]) -> Result<i32>;
    fn block(&self, raw: i32) -> Result<i32>;
    fn block_entity_kind(&self, name: &str) -> Result<i32>;
    /// Armor stands use old INTERACT_AT. Ordinary right-click targets use INTERACT.
    fn interact_at(&self, entity: i32) -> Result<bool>;
    /// Flower pots/skull types can also change the contextual block state. Handle them
    /// before generic NBT conversion; None selects the ordinary block-entity path.
    fn contextual_block_entity(
        &self,
        _pos: [i32; 3],
        _action: u8,
        _tag: &Tag,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        Ok(None)
    }
}

// S33 permits null empty lines from the old component serializer. Keep this
// exception local to sign text; other packet components remain strict.
fn sign_line(r: &mut Reader<'_>, cx: &impl Context) -> Result<Tag> {
    let start = r.position();
    // Official 1.8.9 hw.a -> em.d -> em.c(32767).
    let text = r.string(32767)?;
    if text.trim_matches([' ', '\t', '\r', '\n']).eq("null") {
        return Ok(d::string(""));
    }
    let mut component = Reader::new(r.since(start));
    let value = cx.text(&mut component)?;
    finish(&component)?;
    Ok(value)
}

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 common packet bytes"))
    }
}
fn finite(value: f32) -> Result<f32> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::new("nonfinite protocol47 float"))
    }
}
fn entity(value: i32) -> Result<i32> {
    if value >= 0 {
        Ok(value)
    } else {
        Err(Error::new("negative protocol47 entity id"))
    }
}
pub fn position_up(raw: i64) -> [i32; 3] {
    [
        (raw >> 38) as i32,
        ((raw << 26) >> 52) as i32,
        ((raw << 38) >> 38) as i32,
    ]
}
fn position_down(pos: [i32; 3]) -> Result<i64> {
    let [x, y, z] = pos;
    if !(-33_554_432..33_554_432).contains(&x)
        || !(-33_554_432..33_554_432).contains(&z)
        || !(-2048..2048).contains(&y)
    {
        return Err(Error::new("position exceeds protocol47 packed range"));
    }
    Ok(((i64::from(x) & 0x3ff_ffff) << 38)
        | ((i64::from(y) & 0xfff) << 26)
        | (i64::from(z) & 0x3ff_ffff))
}

#[derive(Clone, Debug, PartialEq)]
pub struct OldPacket {
    pub id: i32,
    pub payload: Vec<u8>,
}
fn packet(id: i32, w: Writer) -> OldPacket {
    OldPacket {
        id,
        payload: w.data,
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub player: i32,
    sneaking: bool,
    ability_flags: u8,
    flying_speed: f32,
    walking_speed: f32,
}
impl State {
    pub fn new(player: i32) -> Self {
        Self {
            player,
            sneaking: false,
            ability_flags: 0,
            flying_speed: 0.05,
            walking_speed: 0.1,
        }
    }

    /// None means another translator owns this old packet id. Some(empty) is a
    /// deliberately consumed old packet; every handled payload is fully validated.
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        cx: &impl Context,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let packets = match id {
            0x02 => {
                let content = cx.text(&mut r)?;
                let position = r.u8()?;
                if position > 2 {
                    return Err(Error::new("unknown protocol47 chat position"));
                }
                vec![C::SystemChat(play::SystemChat {
                    content,
                    overlay: position == 2,
                })]
            }
            0x03 => {
                let game_time = r.i64()?;
                let day = r.i64()?;
                let total_ticks = day
                    .checked_abs()
                    .ok_or_else(|| Error::new("invalid protocol47 day time"))?;
                vec![C::SetTime(play::SetTime {
                    game_time,
                    clocks: vec![play::ClockState {
                        clock: if cx.dimension() == "minecraft:the_end" {
                            1
                        } else {
                            0
                        },
                        total_ticks,
                        partial_tick: 0.0,
                        rate: if day >= 0 { 1.0 } else { 0.0 },
                    }],
                })]
            }
            0x05 => vec![C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
                pos: play::GlobalPos {
                    dimension: cx.dimension().into(),
                    pos: position_up(r.i64()?),
                },
                yaw: 0.0,
                pitch: 0.0,
            })],
            0x06 => vec![C::SetHealth(play::SetHealth {
                health: finite(r.f32()?)?,
                food: r.varint()?,
                saturation: finite(r.f32()?)?,
            })],
            0x09 => {
                let slot = i32::from(r.u8()?);
                if slot > 8 {
                    return Err(Error::new("invalid protocol47 hotbar slot"));
                }
                vec![C::SetHeldSlot(play::SetHeldSlot { slot })]
            }
            0x1f => vec![C::SetExperience(play::SetExperience {
                progress: finite(r.f32()?)?,
                level: r.varint()?,
                total: r.varint()?,
            })],
            0x22 => {
                let chunk = [r.i32()?, r.i32()?];
                let count = r.count(65536)?;
                if count > r.remaining() / 3 {
                    return Err(Error::new("impossible protocol47 multi-block count"));
                }
                let mut sections: BTreeMap<i32, Vec<i64>> = BTreeMap::new();
                for _ in 0..count {
                    let packed = r.u16()?;
                    let raw = r.varint()?;
                    let x = i32::from(packed >> 12);
                    let z = i32::from((packed >> 8) & 15);
                    let y = i32::from(packed & 255);
                    let world_x = chunk[0]
                        .checked_mul(16)
                        .and_then(|v| v.checked_add(x))
                        .ok_or_else(|| Error::new("protocol47 chunk x overflow"))?;
                    let world_z = chunk[1]
                        .checked_mul(16)
                        .and_then(|v| v.checked_add(z))
                        .ok_or_else(|| Error::new("protocol47 chunk z overflow"))?;
                    let state = cx.block_state(raw, [world_x, y, world_z])?;
                    if state < 0 {
                        return Err(Error::new("negative mapped protocol47 block state"));
                    }
                    sections
                        .entry(y >> 4)
                        .or_default()
                        .push((i64::from(state) << 12) | i64::from((x << 8) | (z << 4) | (y & 15)));
                }
                sections
                    .into_iter()
                    .map(|(y, changes)| {
                        C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                            section: [chunk[0], y, chunk[1]],
                            changes,
                        })
                    })
                    .collect()
            }
            0x23 => {
                let pos = position_up(r.i64()?);
                let state = cx.block_state(r.varint()?, pos)?;
                if state < 0 {
                    return Err(Error::new("negative mapped protocol47 block state"));
                }
                vec![C::BlockUpdate(play::BlockUpdate { pos, state })]
            }
            0x24 => {
                let pos = position_up(r.i64()?);
                let b0 = r.u8()?;
                let b1 = r.u8()?;
                let block = cx.block(r.varint()?)?;
                if block < 0 {
                    return Err(Error::new("negative mapped protocol47 block id"));
                }
                vec![C::BlockEvent(play::BlockEvent { pos, b0, b1, block })]
            }
            0x25 => vec![C::BlockDestruction(play::BlockDestruction {
                id: r.varint()?,
                pos: position_up(r.i64()?),
                progress: r.u8()?,
            })],
            0x2b => {
                let event = r.u8()?;
                let param = finite(r.f32()?)?;
                if event > 10 {
                    return Err(Error::new("unknown protocol47 game event"));
                }
                vec![C::GameEvent(play::GameEvent { event, param })]
            }
            0x33 => {
                let pos = position_up(r.i64()?);
                let lines = [
                    sign_line(&mut r, cx)?,
                    sign_line(&mut r, cx)?,
                    sign_line(&mut r, cx)?,
                    sign_line(&mut r, cx)?,
                ];
                let side = |messages: Vec<Tag>| {
                    Tag::compound([
                        ("messages", Tag::List(messages)),
                        ("color", d::string("black")),
                        ("has_glowing_text", Tag::Byte(0)),
                    ])
                };
                let tag = Tag::compound([
                    ("id", d::string("minecraft:sign")),
                    ("x", Tag::Int(pos[0])),
                    ("y", Tag::Int(pos[1])),
                    ("z", Tag::Int(pos[2])),
                    ("front_text", side(lines.into())),
                    ("back_text", side(vec![d::string(""); 4])),
                    ("is_waxed", Tag::Byte(0)),
                ]);
                vec![C::BlockEntityData(play::BlockEntityData {
                    pos,
                    kind: cx.block_entity_kind("minecraft:sign")?,
                    tag,
                })]
            }
            0x35 => {
                let pos = position_up(r.i64()?);
                let action = r.u8()?;
                let tag = r.named_nbt()?;
                if !matches!(tag, Tag::Compound(_)) {
                    return Err(Error::new("protocol47 block entity tag is not compound"));
                }
                validate_block_entity_type(action, &tag)?;
                // The hook may mutate the connection's skull/pot state cache.
                finish(&r)?;
                if let Some(packets) = cx.contextual_block_entity(pos, action, &tag)? {
                    packets
                } else {
                    let mut tag = upgrade_block_entity(pos, action, tag)?;
                    let name = d::as_str(tag.get("id"))
                        .ok_or_else(|| Error::new("missing upgraded block entity name"))?;
                    let kind = cx.block_entity_kind(&name)?;
                    d::set(&mut tag, "x", Tag::Int(pos[0]));
                    d::set(&mut tag, "y", Tag::Int(pos[1]));
                    d::set(&mut tag, "z", Tag::Int(pos[2]));
                    vec![C::BlockEntityData(play::BlockEntityData { pos, kind, tag })]
                }
            }
            0x36 => vec![C::OpenSignEditor(play::OpenSignEditor {
                pos: position_up(r.i64()?),
                front: true,
            })],
            0x39 => {
                let flags = r.u8()?;
                let flying_speed = finite(r.f32()?)?;
                let walking_speed = finite(r.f32()?)?;
                if flags & !15 != 0 || flying_speed < 0.0 || walking_speed < 0.0 {
                    return Err(Error::new("invalid protocol47 abilities"));
                }
                // Commit state only after the entire packet is known to be valid.
                finish(&r)?;
                self.ability_flags = flags;
                self.flying_speed = flying_speed;
                self.walking_speed = walking_speed;
                vec![C::PlayerAbilities(play::PlayerAbilities {
                    flags: flags as i8,
                    flying_speed,
                    walking_speed,
                })]
            }
            0x40 => vec![C::Disconnect(crate::packets::configuration::Disconnect {
                reason: cx.text(&mut r)?,
            })],
            0x41 => {
                let difficulty = i32::from(r.u8()?);
                if difficulty > 3 {
                    return Err(Error::new("invalid protocol47 difficulty"));
                }
                vec![C::ChangeDifficulty(play::ChangeDifficulty {
                    difficulty,
                    locked: false,
                })]
            }
            0x43 => vec![C::SetCamera(play::SetCamera {
                camera_id: entity(r.varint()?)?,
            })],
            0x45 => match r.varint()? {
                0 => vec![C::SetTitleText(play::Text {
                    text: cx.text(&mut r)?,
                })],
                1 => vec![C::SetSubtitleText(play::Text {
                    text: cx.text(&mut r)?,
                })],
                2 => vec![C::SetTitlesAnimation(play::TitlesAnimation {
                    fade_in: r.i32()?,
                    stay: r.i32()?,
                    fade_out: r.i32()?,
                })],
                3 => vec![C::ClearTitles(play::Bool1 { value: false })],
                4 => vec![C::ClearTitles(play::Bool1 { value: true })],
                _ => return Err(Error::new("unknown protocol47 title action")),
            },
            0x47 => vec![C::TabList(play::TabList {
                header: cx.text(&mut r)?,
                footer: cx.text(&mut r)?,
            })],
            _ => return Ok(None),
        };
        finish(&r)?;
        Ok(Some(packets))
    }

    /// Direct pinned777 -> protocol47 packets. None means unhandled by this helper;
    /// Some(empty) is locally consumed. Item-bearing packets have a separate fallible path.
    pub fn serverbound(
        &mut self,
        p: &play::Serverbound,
        cx: &impl Context,
    ) -> Result<Option<Vec<OldPacket>>> {
        use play::Serverbound as S;
        let mut w = Writer::new();
        let id = match p {
            S::MovePlayerPos(p) => {
                movement_flags(p.flags)?;
                coordinates([p.x, p.y, p.z])?;
                w.f64(p.x);
                w.f64(p.y);
                w.f64(p.z);
                w.bool(p.flags & 1 != 0);
                0x04
            }
            S::MovePlayerPosRot(p) => {
                movement_flags(p.flags)?;
                coordinates([p.x, p.y, p.z])?;
                finite(p.yaw)?;
                finite(p.pitch)?;
                w.f64(p.x);
                w.f64(p.y);
                w.f64(p.z);
                w.f32(p.yaw);
                w.f32(p.pitch);
                w.bool(p.flags & 1 != 0);
                0x06
            }
            S::MovePlayerRot(p) => {
                movement_flags(p.flags)?;
                finite(p.yaw)?;
                finite(p.pitch)?;
                w.f32(p.yaw);
                w.f32(p.pitch);
                w.bool(p.flags & 1 != 0);
                0x05
            }
            S::MovePlayerStatus(p) => {
                movement_flags(p.flags)?;
                w.bool(p.flags & 1 != 0);
                0x03
            }
            S::Punch(_) => 0x0a,
            S::SetCarriedItem(p) => {
                if !(0..=8).contains(&p.slot) {
                    return Err(Error::new("invalid protocol47 hotbar slot"));
                }
                w.i16(p.slot);
                0x09
            }
            S::Attack(p) => {
                w.varint(entity(p.id)?);
                w.varint(1);
                0x02
            }
            S::Interact(p) => {
                if p.hand != 0 {
                    return Err(Error::new("protocol47 has no offhand interaction"));
                }
                w.varint(entity(p.id)?);
                if cx.interact_at(p.id)? {
                    coordinates(p.location)?;
                    w.varint(2);
                    for value in p.location {
                        let value = value as f32;
                        finite(value)?;
                        w.f32(value);
                    }
                } else {
                    w.varint(0);
                }
                // Old crouch state travels via PlayerInput/C0B, not the UseEntity payload.
                0x02
            }
            S::PlayerAction(p) => {
                if p.direction > 5 {
                    return Err(Error::new("invalid protocol47 block face"));
                }
                let pos = position_down(p.pos)?;
                // Current vanilla only changes its tracked mining face for action1.
                // Old vanilla does not track that face after START; consume the update
                // locally, preserving the ongoing old break transaction.
                if p.action == 1 {
                    return Ok(Some(Vec::new()));
                }
                // Offhand swap and stab have no old representation.
                let action = match p.action {
                    0 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 4,
                    6 => 5,
                    _ => return Err(Error::new("player action has no protocol47 representation")),
                };
                w.u8(action);
                w.i64(pos);
                w.u8(p.direction);
                0x07
            }
            S::PlayerCommand(p) => {
                if p.id != self.player {
                    return Err(Error::new(
                        "protocol47 player command id is not local player",
                    ));
                }
                let action = match p.action {
                    0 => 2,
                    1 => 3,
                    2 => 4,
                    3 => 5,
                    4 => return Ok(Some(Vec::new())),
                    5 => 6,
                    _ => {
                        return Err(Error::new(
                            "player command has no protocol47 representation",
                        ));
                    }
                };
                if !(0..=100).contains(&p.data) {
                    return Err(Error::new("invalid protocol47 player command data"));
                }
                w.varint(entity(self.player)?);
                w.varint(action);
                w.varint(p.data);
                0x0b
            }
            S::PlayerInput(p) => {
                if p.flags & 128 != 0 {
                    return Err(Error::new("invalid current player input flags"));
                }
                let sneaking = p.flags & 32 != 0;
                let mut packets = Vec::new();
                if sneaking != self.sneaking {
                    packets.push(self.command(if sneaking { 0 } else { 1 }, 0)?);
                    self.sneaking = sneaking;
                }
                let strafe = i32::from(p.flags & 4 != 0) - i32::from(p.flags & 8 != 0);
                let forward = i32::from(p.flags & 1 != 0) - i32::from(p.flags & 2 != 0);
                w.f32(strafe as f32);
                w.f32(forward as f32);
                w.u8(u8::from(p.flags & 16 != 0) | u8::from(sneaking) << 1);
                packets.push(packet(0x0c, w));
                return Ok(Some(packets));
            }
            S::PlayerAbilities(p) => {
                if p.flags & !2 != 0 {
                    return Err(Error::new("invalid current player abilities flags"));
                }
                let flags = (self.ability_flags & !2) | (p.flags as u8 & 2);
                w.u8(flags);
                w.f32(self.flying_speed);
                w.f32(self.walking_speed);
                0x13
            }
            S::ClientInformation(p) => {
                if p.language.encode_utf16().count() > 7 || p.language.len() > 28 {
                    return Err(Error::new(
                        "locale exceeds protocol47 seven-character limit",
                    ));
                }
                if !(0..=2).contains(&p.chat_visibility) || p.model_customisation & 128 != 0 {
                    return Err(Error::new("invalid protocol47 client settings"));
                }
                w.string(&p.language);
                w.i8(p.view_distance);
                w.u8(p.chat_visibility as u8);
                w.bool(p.chat_colors);
                w.u8(p.model_customisation);
                0x15
            }
            S::ClientCommand(p) => {
                if !(0..=1).contains(&p.action) {
                    return Err(Error::new(
                        "client command has no protocol47 representation",
                    ));
                }
                w.varint(p.action);
                0x16
            }
            S::Chat(p) => {
                chat(&p.message, &mut w)?;
                0x01
            }
            S::ChatCommand(p) => {
                chat(&format!("/{}", p.command), &mut w)?;
                0x01
            }
            S::ChatCommandSigned(p) => {
                chat(&format!("/{}", p.command), &mut w)?;
                0x01
            }
            S::SignUpdate(p) => {
                if !p.front {
                    return Err(Error::new("protocol47 has no sign back side"));
                }
                w.i64(position_down(p.pos)?);
                for line in &p.lines {
                    // Vanilla1.8's component serializer writes an unstyled literal as
                    // a JSON string primitive, still distinct from a raw text string.
                    let text = datafix::json::Json::string(line);
                    let text = String::from_utf16_lossy(&datafix::json::to_stable_string(&text));
                    if text.encode_utf16().count() > 384 {
                        return Err(Error::new(
                            "serialized sign component exceeds protocol47 limit",
                        ));
                    }
                    w.string(&text);
                }
                0x12
            }
            S::TeleportToEntity(p) => {
                w.uuid(p.uuid);
                0x18
            }
            // These modern bookkeeping signals have no corresponding old server
            // state:47 has no tick-end/batch pacing, recipe book or advancements.
            S::ChatAck(_)
            | S::ChatSessionUpdate(_)
            | S::PlayerLoaded(_)
            | S::ClientTickEnd(_)
            | S::ChunkBatchReceived(_)
            | S::RecipeBookChangeSettings(_)
            | S::RecipeBookSeenRecipe(_)
            | S::SeenAdvancements(_) => {
                return Ok(Some(Vec::new()));
            }
            _ => return Ok(None),
        };
        Ok(Some(vec![packet(id, w)]))
    }

    fn command(&self, action: i32, data: i32) -> Result<OldPacket> {
        let mut w = Writer::new();
        w.varint(entity(self.player)?);
        w.varint(action);
        w.varint(data);
        Ok(packet(0x0b, w))
    }
}

fn coordinates(values: [f64; 3]) -> Result<()> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(Error::new(
            "nonfinite current movement/interaction position",
        ))
    }
}
fn movement_flags(flags: u8) -> Result<()> {
    if flags & !3 == 0 {
        Ok(())
    } else {
        Err(Error::new("invalid current movement flags"))
    }
}
fn chat(message: &str, w: &mut Writer) -> Result<()> {
    if message.encode_utf16().count() > 100 {
        return Err(Error::new("chat exceeds protocol47 100 UTF16-unit limit"));
    }
    w.string(message);
    Ok(())
}

/// The old block-entity action is an independent type hint. Unknown/contextual
/// actions must not be silently converted to an unrelated current block entity.
pub fn upgrade_block_entity(pos: [i32; 3], action: u8, mut tag: Tag) -> Result<Tag> {
    validate_block_entity_type(action, &tag)?;
    let expected = match action {
        1 => "MobSpawner",
        2 => "Control",
        3 => "Beacon",
        4 => "Skull",
        5 => {
            return Err(Error::new(
                "protocol47 flower pot NBT needs contextual block-state conversion",
            ));
        }
        6 => "Banner",
        _ => return Err(Error::new("unknown protocol47 block entity action")),
    };
    if !matches!(tag, Tag::Compound(_)) {
        return Err(Error::new("protocol47 block entity NBT is not compound"));
    }
    if tag.get("id").is_none() {
        d::set(&mut tag, "id", d::string(expected));
    }
    d::set(&mut tag, "x", Tag::Int(pos[0]));
    d::set(&mut tag, "y", Tag::Int(pos[1]));
    d::set(&mut tag, "z", Tag::Int(pos[2]));
    datafix::update(Ty::BlockEntity, &mut tag, -1, datafix::CURRENT);
    Ok(tag)
}

fn validate_block_entity_type(action: u8, tag: &Tag) -> Result<()> {
    let (old, named) = match action {
        1 => ("MobSpawner", "minecraft:mob_spawner"),
        2 => ("Control", "minecraft:command_block"),
        3 => ("Beacon", "minecraft:beacon"),
        4 => ("Skull", "minecraft:skull"),
        5 => ("FlowerPot", "minecraft:flower_pot"),
        6 => ("Banner", "minecraft:banner"),
        _ => return Err(Error::new("unknown protocol47 block entity action")),
    };
    if let Some(value) = tag.get("id") {
        let id = d::as_str(Some(value))
            .ok_or_else(|| Error::new("non-string protocol47 block entity id"))?;
        if id != old && id != named {
            return Err(Error::new(
                "protocol47 block entity id conflicts with action",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "sign_null_tests.rs"]
mod sign_null_tests;

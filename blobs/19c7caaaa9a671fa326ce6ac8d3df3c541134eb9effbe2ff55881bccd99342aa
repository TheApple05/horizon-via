//! Original protocol47 multiplayer presentation bridge. Outputs are pinned777.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
    packets::{login::Property, play},
};
use horizon_world::{
    datafix::{self, dynamic as d, schema::Ty},
    nbt::Tag,
};
use std::collections::VecDeque;

pub trait Context {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag>;
    fn registry_id(&self, registry: &str, name: &str) -> Result<i32> {
        Err(Error::new(format!(
            "missing protocol47 presentation mapping {registry}/{name}"
        )))
    }
    fn completion_target(&self) -> Option<[i32; 3]> {
        None
    }
}
fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 presentation bytes"))
    }
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid protocol47 boolean")),
    }
}
fn number(r: &mut Reader<'_>) -> Result<f64> {
    let n = r.f64()?;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(Error::new("nonfinite protocol47 border value"))
    }
}
fn size(r: &mut Reader<'_>) -> Result<f64> {
    let n = number(r)?;
    if n >= 0.0 {
        Ok(n)
    } else {
        Err(Error::new("negative protocol47 border size"))
    }
}
fn nonnegative(n: i32) -> Result<i32> {
    if n >= 0 {
        Ok(n)
    } else {
        Err(Error::new("negative protocol47 presentation field"))
    }
}
fn duration(r: &mut Reader<'_>) -> Result<i64> {
    let n = r.varlong()?;
    if n >= 0 {
        Ok(n)
    } else {
        Err(Error::new("negative protocol47 border duration"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OldPacket {
    pub id: i32,
    pub payload: Vec<u8>,
}
#[derive(Debug)]
struct Completion {
    id: i32,
    start: i32,
    length: i32,
    strip_result_slash: bool,
    packet: OldPacket,
}
#[derive(Debug, Default)]
pub struct State {
    completion: Option<Completion>,
    queued: VecDeque<Completion>,
    outgoing: Vec<OldPacket>,
}
impl State {
    /// Drain after every clientbound conversion: a completed old request can release
    /// the next queued request. Old replies contain no transaction id, so only one
    /// request may be in flight at a time.
    pub fn take_outgoing(&mut self) -> Vec<OldPacket> {
        std::mem::take(&mut self.outgoing)
    }

    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        cx: &impl Context,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let packets = match id {
            0x38 => player_info(&mut r, cx)?,
            0x3a => {
                let count = r.count(32767)?;
                if count > r.remaining() {
                    return Err(Error::new("impossible protocol47 completion count"));
                }
                let mut matches = Vec::with_capacity(count.min(256));
                for _ in 0..count {
                    matches.push(r.string(32767)?);
                }
                finish(&r)?;
                let request = self
                    .completion
                    .as_ref()
                    .ok_or_else(|| Error::new("unsolicited protocol47 completion reply"))?;
                let mut w = Writer::new();
                w.varint(request.id);
                w.varint(request.start);
                w.varint(request.length);
                w.varint(matches.len() as i32);
                for value in matches {
                    let value = if request.strip_result_slash {
                        value.strip_prefix('/').unwrap_or(&value)
                    } else {
                        &value
                    };
                    w.string(value);
                    w.bool(false);
                }
                self.completion = self.queued.pop_front();
                if let Some(next) = &self.completion {
                    self.outgoing.push(next.packet.clone());
                }
                vec![C::Unknown(
                    crate::ids::play::clientbound::COMMAND_SUGGESTIONS,
                    w.data,
                )]
            }
            0x3b => {
                let name = r.string(16)?;
                let method = r.i8()?;
                let display = match method {
                    0 | 2 => {
                        let text = legacy_text(&r.string(32)?);
                        let render = match r.string(16)?.as_str() {
                            "integer" => 0,
                            "hearts" => 1,
                            _ => {
                                return Err(Error::new("unknown protocol47 objective render type"));
                            }
                        };
                        Some((text, render, None))
                    }
                    1 => None,
                    _ => return Err(Error::new("unknown protocol47 objective method")),
                };
                vec![C::SetObjective(play::SetObjective {
                    name,
                    method,
                    display,
                })]
            }
            0x3c => {
                let owner = r.string(40)?;
                let method = r.varint()?;
                let objective = r.string(16)?;
                vec![match method {
                    0 => {
                        if objective.is_empty() {
                            return Err(Error::new("protocol47 score has no objective"));
                        }
                        C::SetScore(play::SetScore {
                            owner,
                            objective,
                            score: r.varint()?,
                            display: None,
                            number_format: None,
                        })
                    }
                    1 => C::ResetScore(play::ResetScore {
                        owner,
                        objective: (!objective.is_empty()).then_some(objective),
                    }),
                    _ => return Err(Error::new("unknown protocol47 score method")),
                }]
            }
            0x3d => {
                let slot = i32::from(r.u8()?);
                if slot > 18 {
                    return Err(Error::new("unknown protocol47 display slot"));
                }
                vec![C::SetDisplayObjective(play::SetDisplayObjective {
                    slot,
                    objective: r.string(16)?,
                })]
            }
            0x3e => vec![C::SetPlayerTeam(team(&mut r)?)],
            0x44 => vec![match r.varint()? {
                0 => C::SetBorderSize(play::SetBorderSize {
                    size: size(&mut r)?,
                }),
                1 => C::SetBorderLerpSize(play::SetBorderLerpSize {
                    old_size: size(&mut r)?,
                    new_size: size(&mut r)?,
                    lerp_time: duration(&mut r)?,
                }),
                2 => C::SetBorderCenter(play::SetBorderCenter {
                    x: number(&mut r)?,
                    z: number(&mut r)?,
                }),
                3 => C::InitializeBorder(play::InitializeBorder {
                    center_x: number(&mut r)?,
                    center_z: number(&mut r)?,
                    old_size: size(&mut r)?,
                    new_size: size(&mut r)?,
                    lerp_time: duration(&mut r)?,
                    absolute_max_size: nonnegative(r.varint()?)?,
                    warning_blocks: nonnegative(r.varint()?)?,
                    warning_time: nonnegative(r.varint()?)?,
                }),
                4 => C::SetBorderWarningDelay(play::SetBorderWarningDelay {
                    delay: nonnegative(r.varint()?)?,
                }),
                5 => C::SetBorderWarningDistance(play::SetBorderWarningDistance {
                    blocks: nonnegative(r.varint()?)?,
                }),
                _ => return Err(Error::new("unknown protocol47 border action")),
            }],
            0x42 => match r.varint()? {
                0 => vec![C::Unknown(
                    crate::ids::play::clientbound::PLAYER_COMBAT_ENTER,
                    Vec::new(),
                )],
                1 => {
                    let time = nonnegative(r.varint()?)?;
                    r.i32()?;
                    let mut w = Writer::new();
                    w.varint(time);
                    vec![C::Unknown(
                        crate::ids::play::clientbound::PLAYER_COMBAT_END,
                        w.data,
                    )]
                }
                2 => {
                    let id = r.varint()?;
                    r.i32()?;
                    vec![C::PlayerCombatKill(play::PlayerCombatKill {
                        id,
                        message: legacy_text(&r.string(32767)?),
                    })]
                }
                _ => return Err(Error::new("unknown protocol47 combat action")),
            },
            0x37 => vec![C::AwardStats(stats(&mut r, cx)?)],
            _ => return Ok(None),
        };
        finish(&r)?;
        Ok(Some(packets))
    }

    pub fn serverbound(
        &mut self,
        p: &play::Serverbound,
        cx: &impl Context,
    ) -> Result<Option<Vec<OldPacket>>> {
        let play::Serverbound::Unknown(id, payload) = p else {
            return Ok(None);
        };
        if *id != crate::ids::play::serverbound::COMMAND_SUGGESTION {
            return Ok(None);
        }
        let mut r = Reader::new(payload);
        let id = r.varint()?;
        let command = r.string(32500)?;
        finish(&r)?;
        if self.queued.len() + usize::from(self.completion.is_some()) >= 32 {
            return Err(Error::new("too many pending protocol47 completions"));
        }
        let boundary = command.rfind(' ').map_or(0, |index| index + 1);
        let start = command[..boundary].encode_utf16().count() as i32;
        let length = command[boundary..].encode_utf16().count() as i32;
        let mut w = Writer::new();
        // Current command boxes accept a command without '/', while old TabComplete
        // interprets that as chat/name completion. Keep the current replacement range
        // and remove only the synthetic slash from command-name results.
        let bare = !command.starts_with('/');
        let old_command = if bare {
            format!("/{command}")
        } else {
            command.clone()
        };
        w.string(&old_command);
        if let Some([x, y, z]) = cx.completion_target() {
            if !(-33_554_432..33_554_432).contains(&x)
                || !(-33_554_432..33_554_432).contains(&z)
                || !(-2048..2048).contains(&y)
            {
                return Err(Error::new(
                    "completion target outside protocol47 position range",
                ));
            }
            w.bool(true);
            w.i64(
                ((i64::from(x) & 0x3ff_ffff) << 38)
                    | ((i64::from(y) & 0xfff) << 26)
                    | (i64::from(z) & 0x3ff_ffff),
            );
        } else {
            w.bool(false);
        }
        let request = Completion {
            id,
            start,
            length,
            strip_result_slash: bare && start == 0,
            packet: OldPacket {
                id: 0x14,
                payload: w.data,
            },
        };
        if self.completion.is_none() {
            let packet = request.packet.clone();
            self.completion = Some(request);
            Ok(Some(vec![packet]))
        } else {
            self.queued.push_back(request);
            Ok(Some(Vec::new()))
        }
    }
}

fn player_info(r: &mut Reader<'_>, cx: &impl Context) -> Result<Vec<play::Clientbound>> {
    let action = r.varint()?;
    if !(0..=4).contains(&action) {
        return Err(Error::new("unknown protocol47 player info action"));
    }
    let n = r.count(4096)?;
    if n > r.remaining() / 16 {
        return Err(Error::new("impossible protocol47 player info count"));
    }
    let mut entries = Vec::with_capacity(n.min(256));
    let mut uuids = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let uuid = r.uuid()?;
        if action == 4 {
            uuids.push(uuid);
            continue;
        }
        let mut entry = play::PlayerInfo {
            uuid,
            ..Default::default()
        };
        match action {
            0 => {
                entry.name = Some(r.string(16)?);
                let count = r.count(16)?;
                for _ in 0..count {
                    let name = r.string(64)?;
                    let value = r.string(32767)?;
                    let signature = if boolean(r)? {
                        Some(r.string(1024)?)
                    } else {
                        None
                    };
                    entry.properties.push(Property {
                        name,
                        value,
                        signature,
                    });
                }
                entry.game_mode = Some(mode(r.varint()?)?);
                entry.listed = Some(true);
                entry.latency = Some(r.varint()?);
                entry.display_name = Some(if boolean(r)? { Some(cx.text(r)?) } else { None });
                // Defaults reproduce pre-list-order and pre-hat-toggle clients, without
                // claiming or inventing a modern secure-chat session.
                entry.list_order = Some(0);
                entry.show_hat = Some(true);
            }
            1 => entry.game_mode = Some(mode(r.varint()?)?),
            2 => entry.latency = Some(r.varint()?),
            3 => entry.display_name = Some(if boolean(r)? { Some(cx.text(r)?) } else { None }),
            _ => unreachable!(),
        }
        entries.push(entry);
    }
    Ok(if action == 4 {
        vec![play::Clientbound::PlayerInfoRemove(
            play::PlayerInfoRemove { uuids },
        )]
    } else {
        vec![play::Clientbound::PlayerInfoUpdate(
            play::PlayerInfoUpdate {
                actions: match action {
                    0 => 253,
                    1 => 4,
                    2 => 16,
                    3 => 32,
                    _ => unreachable!(),
                },
                entries,
            },
        )]
    })
}
fn mode(mode: i32) -> Result<i32> {
    if (-1..=3).contains(&mode) {
        Ok(mode)
    } else {
        Err(Error::new("unknown protocol47 player game mode"))
    }
}

fn team(r: &mut Reader<'_>) -> Result<play::SetPlayerTeam> {
    let name = r.string(16)?;
    let method = r.i8()?;
    if !(0..=4).contains(&method) {
        return Err(Error::new("unknown protocol47 team method"));
    }
    let parameters = if method == 0 || method == 2 {
        let display_name = legacy_text(&r.string(32)?);
        let prefix = legacy_text(&r.string(16)?);
        let suffix = legacy_text(&r.string(16)?);
        let options = r.i8()?;
        if options & !3 != 0 {
            return Err(Error::new("unknown protocol47 team options"));
        }
        let name_tag_visibility = match r.string(32)?.as_str() {
            "always" => 0,
            "never" => 1,
            "hideForOtherTeams" => 2,
            "hideForOwnTeam" => 3,
            _ => return Err(Error::new("unknown protocol47 name tag visibility")),
        };
        let color = r.i8()?;
        let color = match color {
            -1 => None,
            0..=15 => Some(i32::from(color)),
            _ => return Err(Error::new("unknown protocol47 team color")),
        };
        // Old teams had no collision-rule field. Use the current saved-team reader's
        // default ALWAYS; no server-supplied collision restriction is invented.
        Some(play::TeamParameters {
            display_name,
            prefix,
            suffix,
            name_tag_visibility,
            collision_rule: 0,
            color,
            options,
        })
    } else {
        None
    };
    let mut players = Vec::new();
    if matches!(method, 0 | 3 | 4) {
        let count = r.count(65536)?;
        if count > r.remaining() {
            return Err(Error::new("impossible protocol47 team member count"));
        }
        for _ in 0..count {
            players.push(r.string(40)?);
        }
    }
    Ok(play::SetPlayerTeam {
        name,
        method,
        parameters,
        players,
    })
}

fn stats(r: &mut Reader<'_>, cx: &impl Context) -> Result<play::AwardStats> {
    let count = r.count(65536)?;
    if count > r.remaining() / 2 {
        return Err(Error::new("impossible protocol47 statistic count"));
    }
    let mut legacy = d::empty_map();
    for _ in 0..count {
        let name = r.string(32767)?;
        d::set(&mut legacy, &name, Tag::Int(r.varint()?));
    }
    datafix::update(Ty::Stats, &mut legacy, -1, datafix::CURRENT);
    let mut stats = Vec::new();
    if let Some(Tag::Compound(categories)) = legacy.get("stats") {
        for (category, values) in categories {
            let category = category.to_string_lossy();
            let registry = match category.as_str() {
                "minecraft:mined" => "minecraft:block",
                "minecraft:crafted"
                | "minecraft:used"
                | "minecraft:broken"
                | "minecraft:picked_up"
                | "minecraft:dropped" => "minecraft:item",
                "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                "minecraft:custom" => "minecraft:custom_stat",
                _ => return Err(Error::new("unknown upgraded protocol47 statistic category")),
            };
            let kind = cx.registry_id("minecraft:stat_type", &category)?;
            let Tag::Compound(values) = values else {
                return Err(Error::new(
                    "upgraded protocol47 statistics are not compound",
                ));
            };
            for (name, value) in values {
                let value = d::as_int(Some(value), 0);
                stats.push((
                    kind,
                    cx.registry_id(registry, &name.to_string_lossy())?,
                    value,
                ));
            }
        }
    }
    Ok(play::AwardStats { stats })
}

/// Decode valid old section-code formatting into independent current component spans.
/// Score/team identifiers themselves must keep their exact original strings.
pub fn legacy_text(text: &str) -> Tag {
    const COLORS: [&str; 16] = [
        "black",
        "dark_blue",
        "dark_green",
        "dark_aqua",
        "dark_red",
        "dark_purple",
        "gold",
        "gray",
        "dark_gray",
        "blue",
        "green",
        "aqua",
        "red",
        "light_purple",
        "yellow",
        "white",
    ];
    let mut color = None;
    let mut flags = [false; 5];
    let mut buffer = String::new();
    let mut spans = Vec::new();
    let mut chars = text.chars().peekable();
    fn flush(buffer: &mut String, spans: &mut Vec<Tag>, color: Option<&str>, flags: [bool; 5]) {
        if buffer.is_empty() {
            return;
        }
        let mut tag = Tag::compound([("text", d::string(buffer))]);
        if let Some(color) = color {
            d::set(&mut tag, "color", d::string(color));
        }
        for (name, on) in [
            "obfuscated",
            "bold",
            "strikethrough",
            "underlined",
            "italic",
        ]
        .into_iter()
        .zip(flags)
        {
            d::set(&mut tag, name, Tag::Byte(i8::from(on)));
        }
        spans.push(tag);
        buffer.clear();
    }
    while let Some(ch) = chars.next() {
        if ch == '§'
            && let Some(&code) = chars.peek()
        {
            let code = code.to_ascii_lowercase();
            if let Some(index) = "0123456789abcdefklmnor".find(code) {
                chars.next();
                flush(&mut buffer, &mut spans, color, flags);
                match index {
                    0..=15 => {
                        color = Some(COLORS[index]);
                        flags = [false; 5];
                    }
                    16..=20 => flags[index - 16] = true,
                    _ => {
                        color = None;
                        flags = [false; 5];
                    }
                }
                continue;
            }
        }
        buffer.push(ch);
    }
    flush(&mut buffer, &mut spans, color, flags);
    if spans.is_empty() {
        d::string("")
    } else {
        Tag::compound([("text", d::string("")), ("extra", Tag::List(spans))])
    }
}

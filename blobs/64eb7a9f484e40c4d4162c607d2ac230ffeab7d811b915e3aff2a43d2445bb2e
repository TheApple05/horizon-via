//! Native Java1.8.9(protocol47) adaptation to the pinned777 consumer.
//! Wire expectations are checked against independently executed vanilla packet writers.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use std::collections::BTreeSet;

pub mod bootstrap;
pub mod bridge;
pub mod common;
pub mod entity;
pub mod facts;
pub mod ids;
pub mod inventory;
pub mod lookup47;
pub mod metadata;
pub mod neighbors47;
pub mod presentation;
pub mod terrain;
pub mod world;
pub mod worldfx;
pub mod worldfx_extra;

/// Connection-local vanilla47 lighting facts for the native terrain workers.
pub fn lighting_properties() -> &'static [(u32, u8, u8)] {
    lookup47::LIGHT_OVERRIDES
}

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() != 0 {
        return Err(Error::new("trailing legacy packet bytes"));
    }
    Ok(())
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid legacy boolean")),
    }
}

/// Legacy positions place Y in the middle12bits;1.14+ places Y at the bottom.
pub fn position_up(value: i64) -> i64 {
    let x = value >> 38;
    let y = (value << 26) >> 52;
    let z = (value << 38) >> 38;
    ((x & 0x3ff_ffff) << 38) | ((z & 0x3ff_ffff) << 12) | (y & 0xfff)
}
pub fn position_down(value: i64) -> i64 {
    let x = value >> 38;
    let z = (value << 26) >> 38;
    let y = (value << 52) >> 52;
    ((x & 0x3ff_ffff) << 38) | ((y & 0xfff) << 26) | (z & 0x3ff_ffff)
}

#[derive(Debug, PartialEq)]
pub struct Join {
    pub entity: i32,
    pub hardcore: bool,
    pub mode: u8,
    pub dimension: i8,
    pub difficulty: u8,
    pub max_players: u8,
    pub level_type: String,
    pub reduced_debug: bool,
}
pub fn join(payload: &[u8]) -> Result<Join> {
    let mut r = Reader::new(payload);
    let entity = r.i32()?;
    let flags = r.u8()?;
    if flags & !0x0b != 0 {
        return Err(Error::new("invalid legacy gamemode"));
    }
    let dimension = r.i8()?;
    if ![-1, 0, 1].contains(&dimension) {
        return Err(Error::new("unrecognized legacy dimension"));
    }
    let difficulty = r.u8()?;
    if difficulty > 3 {
        return Err(Error::new("invalid legacy difficulty"));
    }
    let max_players = r.u8()?;
    let level_type = r.string(16)?;
    let reduced_debug = boolean(&mut r)?;
    finish(&r)?;
    Ok(Join {
        entity,
        hardcore: flags & 8 != 0,
        mode: flags & 3,
        dimension,
        difficulty,
        max_players,
        level_type,
        reduced_debug,
    })
}

pub fn keep_alive_up(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let n = r.varint()?;
    finish(&r)?;
    let mut w = Writer::new();
    w.i64(i64::from(n));
    Ok(w.data)
}
pub fn keep_alive_down(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let n = r.i64()?;
    finish(&r)?;
    let mut w = Writer::new();
    w.varint(i32::try_from(n).map_err(|_| Error::new("keepalive exceeds legacy integer"))?);
    Ok(w.data)
}

/// Old login-success carries textual UUID;763 carries a binary UUID and properties.
pub fn login_finished(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let uuid = r.string(36)?;
    let name = r.string(16)?;
    finish(&r)?;
    let hex = if uuid.len() == 36 {
        if [8, 13, 18, 23].iter().any(|&i| uuid.as_bytes()[i] != b'-') {
            return Err(Error::new("invalid legacy UUID"));
        }
        uuid.chars().filter(|&c| c != '-').collect::<String>()
    } else {
        uuid
    };
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new("invalid legacy UUID"));
    }
    let uuid = u128::from_str_radix(&hex, 16).map_err(|_| Error::new("invalid legacy UUID"))?;
    let mut w = Writer::new();
    w.uuid(uuid);
    w.string(&name);
    w.varint(0);
    Ok(w.data)
}

#[derive(Default)]
pub struct State {
    next_teleport: i32,
    pending_teleports: BTreeSet<i32>,
}
impl State {
    /// Generate consumer-only IDs. A1.8server has no teleport-confirm packet.
    pub fn player_position(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        if self.pending_teleports.len() >= 128 {
            return Err(Error::new("too many unacknowledged legacy teleports"));
        }
        let mut r = Reader::new(payload);
        let xyz = [r.f64()?, r.f64()?, r.f64()?];
        let angles = [r.f32()?, r.f32()?];
        let flags = r.u8()?;
        finish(&r)?;
        if flags & !31 != 0
            || xyz.iter().any(|x| !x.is_finite())
            || angles.iter().any(|x| !x.is_finite())
        {
            return Err(Error::new("invalid legacy player position"));
        }
        let id = self.next_teleport;
        self.next_teleport = self
            .next_teleport
            .checked_add(1)
            .ok_or_else(|| Error::new("legacy teleport counter exhausted"))?;
        self.pending_teleports.insert(id);
        let mut w = Writer::new();
        xyz.into_iter().for_each(|v| w.f64(v));
        angles.into_iter().for_each(|v| w.f32(v));
        w.u8(flags);
        w.varint(id);
        w.bool(false);
        Ok(w.data)
    }
    /// Consume an acknowledgement locally rather than emitting a packet the old server cannot read.
    pub fn acknowledge_teleport(&mut self, payload: &[u8]) -> Result<()> {
        let mut r = Reader::new(payload);
        let id = r.varint()?;
        finish(&r)?;
        if !self.pending_teleports.remove(&id) {
            return Err(Error::new("unknown legacy teleport acknowledgement"));
        }
        Ok(())
    }
}

/// Both old protocols have one pack slot. Native replies carry the request identity
/// so delayed feedback for a superseded request cannot acquire the replacement's hash.
#[derive(Default)]
pub(super) struct LegacyPackSlot {
    current: Option<(u128, String)>,
}

impl LegacyPackSlot {
    pub(super) fn request(
        &mut self,
        url: String,
        hash: String,
        echo_hash: bool,
    ) -> std::result::Result<crate::packets::configuration::ResourcePackPush, Vec<u8>> {
        if !super::valid_resource_pack_url(&url) {
            // The native client rejects these schemes before changing its prompt or active pack.
            let mut w = Writer::new();
            if echo_hash {
                w.string(&hash);
            }
            w.varint(2);
            return Err(w.data);
        }
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
        self.current = Some((id, hash.clone()));
        Ok(crate::packets::configuration::ResourcePackPush {
            id,
            url,
            hash,
            required: false,
            prompt: None,
        })
    }

    pub(super) fn status(
        &mut self,
        id: u128,
        action: i32,
        echo_hash: bool,
    ) -> Result<Option<Vec<u8>>> {
        let Some((current_id, hash)) = &self.current else {
            return Ok(None);
        };
        if *current_id != id {
            return Ok(None);
        }
        let status = match action {
            0..=3 => action,
            5 | 6 => 2, // invalid URL or failed reload is a failed download in the old protocol
            4 | 7 => return Ok(None), // downloaded and discarded have no wire representation
            _ => return Err(Error::new("invalid resource pack action")),
        };
        let mut w = Writer::new();
        if echo_hash {
            w.string(hash)
        }
        w.varint(status);
        if status != 3 {
            self.current = None;
        }
        Ok(Some(w.data))
    }
}

pub fn client_settings(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let locale = r.string(16)?;
    let view = r.i8()?;
    let chat = r.varint()?;
    let colors = boolean(&mut r)?;
    let skin = r.u8()?;
    let hand = r.varint()?;
    let _filtering = boolean(&mut r)?;
    let _listed = boolean(&mut r)?;
    finish(&r)?;
    if !(0..=2).contains(&chat) || !(0..=1).contains(&hand) {
        return Err(Error::new("invalid legacy client settings"));
    }
    let mut w = Writer::new();
    w.string(&locale);
    w.i8(view);
    w.u8(chat as u8);
    w.bool(colors);
    w.u8(skin);
    Ok(w.data)
}
pub fn custom_payload(payload: &[u8], up: bool) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let name = r.string(32767)?;
    let brand = if up { "minecraft:brand" } else { "MC|Brand" };
    let expected = if up { "MC|Brand" } else { "minecraft:brand" };
    if name != expected {
        return Err(Error::new("legacy plugin channel needs explicit mapping"));
    }
    let mut w = Writer::new();
    w.string(brand);
    w.bytes(r.rest());
    Ok(w.data)
}
pub fn chat_message(payload: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let message = r.string(256)?;
    finish(&r)?;
    if message.encode_utf16().count() > 100 {
        return Err(Error::new("chat exceeds legacy100UTF16units"));
    }
    let mut w = Writer::new();
    w.string(&message);
    Ok(w.data)
}
pub fn entity_move(payload: &[u8], look: bool) -> Result<Vec<u8>> {
    let mut r = Reader::new(payload);
    let entity = r.varint()?;
    let xyz = [r.i8()?, r.i8()?, r.i8()?];
    let angles = if look { Some([r.u8()?, r.u8()?]) } else { None };
    let ground = boolean(&mut r)?;
    finish(&r)?;
    let mut w = Writer::new();
    w.varint(entity);
    for v in xyz {
        w.i16(i16::from(v) * 128);
    }
    if let Some([yaw, pitch]) = angles {
        w.u8(yaw);
        w.u8(pitch);
    }
    w.bool(ground);
    Ok(w.data)
}

#[cfg(test)]
mod pack_tests {
    use super::*;

    #[test]
    fn superseded_requests_never_write_statuses_for_the_replacement() {
        for echo_hash in [true, false] {
            let mut slot = LegacyPackSlot::default();
            let a = slot
                .request("https://packs/a".into(), "hash-a".into(), echo_hash)
                .unwrap();
            let b = slot
                .request("https://packs/b".into(), "hash-b".into(), echo_hash)
                .unwrap();
            let again = slot
                .request(a.url.clone(), a.hash.clone(), echo_hash)
                .unwrap();
            let invalid = slot
                .request("ftp://test/pack".into(), "bad-hash".into(), echo_hash)
                .unwrap_err();
            let mut invalid = Reader::new(&invalid);
            if echo_hash {
                assert_eq!(invalid.string(40).unwrap(), "bad-hash");
            }
            assert_eq!(invalid.varint().unwrap(), 2);
            assert_eq!(invalid.remaining(), 0);
            assert_ne!(again.id, a.id);
            for previous in [a.id, b.id] {
                for status in [3, 4, 0, 2, 5, 6, 7] {
                    assert_eq!(slot.status(previous, status, echo_hash).unwrap(), None);
                }
            }
            assert_eq!(slot.status(again.id, 7, echo_hash).unwrap(), None);
            for (native, wire) in [(3, 3), (5, 2)] {
                let raw = slot.status(again.id, native, echo_hash).unwrap().unwrap();
                let mut r = Reader::new(&raw);
                if echo_hash {
                    assert_eq!(r.string(40).unwrap(), "hash-a");
                }
                assert_eq!(r.varint().unwrap(), wire);
                assert_eq!(r.remaining(), 0);
            }
            assert_eq!(slot.status(again.id, 0, echo_hash).unwrap(), None);
        }
    }
}

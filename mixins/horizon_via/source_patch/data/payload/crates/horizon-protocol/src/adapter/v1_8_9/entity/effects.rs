//! Protocol47 effect ids1..23 are the same named order as pinned777 ids0..22.
use super::{Error, Result, boolean, entity_id, finish};
use crate::{buf::Reader, packets::play};
fn effect(r: &mut Reader<'_>) -> Result<i32> {
    let effect = r.u8()?;
    if !(1..=23).contains(&effect) {
        return Err(Error::new("unknown protocol47 effect id"));
    }
    Ok(i32::from(effect) - 1)
}
pub fn update(payload: &[u8]) -> Result<play::UpdateMobEffect> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let effect = effect(&mut r)?;
    let amplifier = i32::from(r.u8()?);
    let duration = r.varint()?;
    let visible = boolean(&mut r)?;
    finish(&r)?;
    if duration < 0 {
        return Err(Error::new("negative protocol47 effect duration"));
    }
    // Old S1D sends only ShowParticles, not modern ambient/blending/icon flags.
    Ok(play::UpdateMobEffect {
        id,
        effect,
        amplifier,
        duration,
        flags: if visible { 6 } else { 4 },
    })
}
pub fn remove(payload: &[u8]) -> Result<play::RemoveMobEffect> {
    let mut r = Reader::new(payload);
    let id = entity_id(&mut r)?;
    let effect = effect(&mut r)?;
    finish(&r)?;
    Ok(play::RemoveMobEffect { id, effect })
}

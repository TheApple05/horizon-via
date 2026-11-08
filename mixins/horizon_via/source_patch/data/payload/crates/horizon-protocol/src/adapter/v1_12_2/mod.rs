//! Native Java 1.12.2 (protocol 340) connection adapter.
use crate::{Error, Result, buf::Reader};

pub mod bridge;
pub use bridge::Bridge;
mod contextual;
mod entities;
pub mod facts;
mod inventory;
mod neighbors;
mod recipes;
mod terrain;
pub mod worldfx;

pub fn names() -> std::sync::Arc<super::ids::LegacyRegistries> {
    super::v1_8_9::bootstrap::names()
}

fn finish(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol340 packet bytes"))
    }
}

fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid protocol340 boolean")),
    }
}

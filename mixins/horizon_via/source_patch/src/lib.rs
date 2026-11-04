//! Build-time source overlay support for the Horizon Via native mod.
//!
//! The runtime library deliberately has no code or dependencies. The build script
//! applies a hash-checked overlay only inside Horizon's isolated native-mod stage.

#[cfg(test)]
mod overlay;

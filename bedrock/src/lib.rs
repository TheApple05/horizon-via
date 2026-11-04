// SPDX-License-Identifier: MIT

//! Native Bedrock foundation, not yet connected to Horizon gameplay.
//! No game assets, accounts, downloads or server discovery are owned here.

pub mod blocks;
pub mod catalog;
pub mod limits;
pub mod storage;
pub mod translation;

#[cfg(feature = "nethernet")]
pub mod negotiation;

#[cfg(feature = "nethernet")]
pub mod startup;

//! API v1 wire contract. This module does not start a server or own engine state.
//! `docs/IPC.md` is the maintained contract; shared client/server examples live in
//! `contracts/ipc/v1`. The legacy IPC transport remains independent.
#![allow(dead_code, unused_imports)] // Contract types/reexports precede their consumers.

pub mod events;
pub mod objects;
pub mod ops;
pub mod request;
pub mod response;
pub mod scalar;
pub mod server;
pub mod validation;

pub use events::*;
pub use objects::*;
pub use request::*;
pub use response::*;
pub use scalar::*;

pub const PROTO: u64 = 1;
pub const LINE_IN: usize = 65_536;
pub const LINE_OUT: usize = 1_048_576;
pub const MAX_HASHES: usize = 256;
pub const MAX_SUBSCRIPTIONS: usize = 16;
pub const MAX_CHUNK_ITEMS: usize = 256;

#[cfg(test)]
mod fixtures;

/// Legacy engine.sock adapter (P1-10); removed in P4-09.
pub mod legacy;

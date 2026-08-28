//! diffpane's Rust backend. The browser UI and `docs/contract.md` are frozen:
//! this crate has to produce the same JSON the TypeScript did, which is what
//! the parity harness asserts.

pub mod classify;
pub mod diff;
pub mod model;

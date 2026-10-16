//! diffpane's backend. The browser UI and the JSON between them are frozen:
//! this crate has to produce the same JSON the TypeScript did, which is what
//! the parity harness asserts.

pub mod args;
pub mod assets;
pub mod browser;
pub mod classify;
pub mod cli;
pub mod diff;
pub mod model;
pub mod report;
pub mod scope;
pub mod server;
pub mod session;
pub mod skill;
pub mod validate;
pub mod wait;

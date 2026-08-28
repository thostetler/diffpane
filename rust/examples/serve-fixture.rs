//! Serves an existing session directory, for the browser suite.
//!
//!   serve-fixture <session dir>
//!
//! Prints `{"url":…,"token":…}` on stdout, then serves until it is killed. The
//! CLI builds its session from a real diff, so there is no supported way to
//! point it at a fixture; this is that, and it is an example rather than a
//! `src/bin` so `cargo install` does not carry it.

use std::sync::Arc;

use anyhow::{Context, Result};
use diffpane::assets::Assets;
use diffpane::server::{AppState, bind, generate_token, serve};
use diffpane::session::Session;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
  let dir = std::env::args().nth(1).context("usage: serve-fixture <session dir>")?;
  let token = generate_token();
  // Held, not dropped: a submit signals on this channel, and a closed one
  // would make the browser's submit fail on the way out.
  let (submit_tx, _submitted) = mpsc::channel(1);

  let listener = bind(0).await?;
  let port = listener.local_addr()?.port();
  let state = Arc::new(AppState::new(
    Session::new(dir.into()),
    token.clone(),
    Assets::from_env(),
    submit_tx,
    port,
  ));
  println!("{{\"url\":\"http://127.0.0.1:{port}/?t={token}\",\"token\":\"{token}\"}}");

  serve(listener, state, std::future::pending()).await
}

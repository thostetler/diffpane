//! Waiting for the human.
//!
//! Three ways a review ends, and the caller needs to know which: the report is
//! written either way, but only a submitted review can be approved (a timeout
//! and a Ctrl+C are both `abandoned`, which is why item 21 was about the flush
//! and not about exit codes).

use std::time::Duration;

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
  Submitted,
  Timeout,
  Interrupt,
  /// The server task returned before anything asked it to. Not produced here —
  /// the wait cannot see it — but the caller has no other way to say so, and
  /// calling it an interrupt blamed the human for a crash.
  ServerStopped,
}

/// Resolves when the review is submitted, the timeout fires, or the user quits.
/// `timeout_seconds` of 0 means never.
///
/// Unlike the TypeScript, there is no grace period for an in-flight submit
/// here: the caller shuts the server down gracefully, so a submit already on
/// the wire finishes on its own. See `server`'s module docs.
pub async fn wait_for_ending(submitted: &mut mpsc::Receiver<()>, timeout_seconds: u64) -> Ending {
  tokio::select! {
    signal = wait_for_submit(submitted) => signal,
    _ = tokio::signal::ctrl_c() => {
      eprintln!();
      Ending::Interrupt
    }
    () = sleep_or_never(timeout_seconds) => Ending::Timeout,
  }
}

/// A closed channel means the server dropped its state, which only happens if
/// the server itself stopped. That is the caller's problem to report, not an
/// ending, so this waits rather than inventing one.
async fn wait_for_submit(submitted: &mut mpsc::Receiver<()>) -> Ending {
  match submitted.recv().await {
    Some(()) => Ending::Submitted,
    None => std::future::pending().await,
  }
}

async fn sleep_or_never(seconds: u64) {
  if seconds == 0 {
    std::future::pending::<()>().await;
  }
  tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn a_submit_ends_the_wait() {
    let (sender, mut receiver) = mpsc::channel(1);
    sender.send(()).await.expect("send");
    assert_eq!(wait_for_ending(&mut receiver, 60).await, Ending::Submitted);
  }

  #[tokio::test]
  async fn the_timeout_ends_the_wait() {
    let (_sender, mut receiver) = mpsc::channel(1);
    tokio::time::pause();
    let waiting = tokio::spawn(async move { wait_for_ending(&mut receiver, 3600).await });
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert_eq!(waiting.await.expect("join"), Ending::Timeout);
  }

  #[tokio::test]
  async fn a_timeout_of_zero_never_fires() {
    let (sender, mut receiver) = mpsc::channel(1);
    let elapsed = tokio::time::timeout(Duration::from_millis(20), async {
      wait_for_ending(&mut receiver, 0).await
    })
    .await;
    assert!(elapsed.is_err(), "waited forever, as asked");
    drop(sender);
  }

  #[tokio::test]
  async fn a_closed_channel_is_not_an_ending() {
    let (sender, mut receiver) = mpsc::channel(1);
    drop(sender);
    let waited =
      tokio::time::timeout(Duration::from_millis(20), wait_for_ending(&mut receiver, 0)).await;
    assert!(waited.is_err(), "a dead server is the caller's to report");
  }
}

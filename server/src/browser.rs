//! Opening the review in a browser.

use std::process::{Command, Stdio};

fn launcher() -> (&'static str, &'static [&'static str]) {
  if cfg!(target_os = "macos") {
    ("open", &[])
  } else if cfg!(target_os = "windows") {
    ("cmd", &["/c", "start", ""])
  } else {
    ("xdg-open", &[])
  }
}

/// Best effort: a headless box or a missing xdg-open is not a reason to fail
/// the review, since the URL is printed either way.
pub fn open(url: &str) {
  let (command, args) = launcher();
  let spawned = Command::new(command)
    .args(args)
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn();
  // The launcher exits as soon as it has handed the URL over, but nobody was
  // reaping it, so it sat as a zombie for the review's whole hour. Waiting on
  // a thread keeps `open` non-blocking.
  if let Ok(mut child) = spawned {
    std::thread::spawn(move || {
      let _ = child.wait();
    });
  }
}

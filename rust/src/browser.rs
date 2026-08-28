//! Opening the review in a browser, ported from `src/open-browser.ts`.

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
  let _ = Command::new(command)
    .args(args)
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn();
}

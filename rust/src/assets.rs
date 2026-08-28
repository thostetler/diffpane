//! The browser assets, compiled into the binary.
//!
//! Same call as `skill.rs`: the binary ships alone, so finding `ui/` relative
//! to the executable was one more thing to get wrong on a user's machine — and
//! it got it wrong quietly, falling back to a CWD-relative `ui` and 404ing
//! every asset into a blank page.
//!
//! `DIFFPANE_UI_DIR` serves from a directory instead, for iterating on `ui/`
//! without a rebuild and for the browser suite.

use std::path::{Path, PathBuf};

macro_rules! embedded {
  ($($name:literal),* $(,)?) => {
    &[$(($name, include_bytes!(concat!("../../ui/", $name)) as &[u8])),*]
  };
}

/// Every file the page can ask for. `ui/` also holds the browser suite, which
/// is not an asset and has no business in the binary, so the set is named
/// rather than globbed.
const EMBEDDED: &[(&str, &[u8])] = embedded![
  "index.html",
  "app.css",
  "app.js",
  "fixture.json",
  "favicon.ico",
  "favicon.svg",
  "logo.png",
  "apple-touch-icon.png",
];

/// Where `serve_page`, `serve_favicon` and `serve_asset` read from.
pub enum Assets {
  Embedded,
  Dir(PathBuf),
}

impl Assets {
  pub fn from_env() -> Self {
    Self::from_override(std::env::var("DIFFPANE_UI_DIR").ok().as_deref())
  }

  /// Split out so the tests do not have to mutate the environment, which is
  /// process-wide and racy under a threaded test runner.
  fn from_override(dir: Option<&str>) -> Self {
    match dir {
      Some(dir) if !dir.is_empty() => Self::Dir(PathBuf::from(dir)),
      _ => Self::Embedded,
    }
  }

  /// `None` is the caller's 404. A name that escapes the directory reads as
  /// missing rather than as an error, so the two sources answer alike.
  pub fn read(&self, name: &str) -> Option<Vec<u8>> {
    if name.is_empty() {
      return None;
    }
    match self {
      Self::Embedded => {
        EMBEDDED.iter().find(|(asset, _)| *asset == name).map(|(_, bytes)| bytes.to_vec())
      }
      Self::Dir(dir) => read_under(dir, name),
    }
  }
}

/// Reads a file from a directory, refusing anything that escapes it.
fn read_under(dir: &Path, name: &str) -> Option<Vec<u8>> {
  let root = std::fs::canonicalize(dir).ok()?;
  let target = std::fs::canonicalize(root.join(name)).ok()?;
  if !target.starts_with(&root) || !target.is_file() {
    return None;
  }
  std::fs::read(&target).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ui() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").join("ui")
  }

  #[test]
  fn the_embedded_assets_are_the_ones_on_disk() {
    for (name, bytes) in EMBEDDED {
      assert_eq!(&std::fs::read(ui().join(name)).expect(name), bytes, "{name} is stale");
    }
  }

  /// The page is unreachable without this one, and a typo in the list would
  /// otherwise only show up as a blank browser tab.
  #[test]
  fn the_page_itself_is_embedded() {
    let page = Assets::Embedded.read("index.html").expect("index.html");
    assert!(String::from_utf8_lossy(&page).contains("<html"));
  }

  #[test]
  fn the_browser_suite_is_not_embedded() {
    assert!(ui().join("smoke.test.ts").is_file(), "the suite moved; update this test");
    assert!(Assets::Embedded.read("smoke.test.ts").is_none());
  }

  #[test]
  fn a_directory_serves_what_it_holds_and_nothing_above_it() {
    let assets = Assets::Dir(ui());
    assert!(assets.read("app.css").is_some());
    assert!(assets.read("../Cargo.toml").is_none());
    assert!(assets.read("").is_none());
  }

  #[test]
  fn the_override_only_counts_when_it_says_something() {
    assert!(
      matches!(Assets::from_override(Some("/tmp/diffpane-ui")), Assets::Dir(dir) if dir == Path::new("/tmp/diffpane-ui"))
    );
    assert!(matches!(Assets::from_override(Some("")), Assets::Embedded));
    assert!(matches!(Assets::from_override(None), Assets::Embedded));
  }
}

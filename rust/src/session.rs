//! The on-disk session: a cache directory holding `meta.json`, `hunks.json`,
//! the agent's `review.json`, and the human's `comments.json`.
//!
//! The layout is the TypeScript's, verbatim, including the repo-root hash in
//! the directory name — two checkouts can share a basename.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::model::{Hunks, Meta, Review, ReviewState};

const SLUG_MAX: usize = 48;

pub fn cache_root() -> PathBuf {
  let base = match std::env::var("XDG_CACHE_HOME") {
    Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
    _ => home_dir().join(".cache"),
  };
  base.join("diffpane")
}

fn home_dir() -> PathBuf {
  match std::env::var("HOME") {
    Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
    _ => PathBuf::from("/"),
  }
}

/// Second-precision UTC, matching the TypeScript. The UI compares this string
/// against its own, and the millisecond form once exposed a faked success
/// screen — keep the shape.
pub fn now_iso() -> String {
  Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn slugify(text: &str) -> String {
  let mut slug = String::with_capacity(text.len());
  for ch in text.chars() {
    if ch.is_ascii_alphanumeric() {
      slug.push(ch.to_ascii_lowercase());
    } else if !slug.ends_with('-') {
      slug.push('-');
    }
  }
  let trimmed: &str = slug.trim_matches('-');
  let capped = &trimmed[..trimmed.len().min(SLUG_MAX)];
  if capped.is_empty() { "review".to_string() } else { capped.to_string() }
}

/// Atomic so a crash mid-review cannot truncate the comments file.
pub fn write_json<T: Serialize>(path: &Path, payload: &T) -> Result<()> {
  let parent = path.parent().unwrap_or(Path::new("."));
  fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
  let mut body = serde_json::to_string_pretty(payload)?;
  body.push('\n');

  let temp = path.with_extension(format!("{:08x}.tmp", fastrand::u32(..)));
  match fs::write(&temp, body).and_then(|()| fs::rename(&temp, path)) {
    Ok(()) => Ok(()),
    Err(error) => {
      let _ = fs::remove_file(&temp);
      Err(error).with_context(|| format!("write {}", path.display()))
    }
  }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
  let body = match fs::read_to_string(path) {
    Ok(body) => body,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
  };
  let parsed = serde_json::from_str(&body).with_context(|| format!("parse {}", path.display()))?;
  Ok(Some(parsed))
}

pub struct Session {
  pub dir: PathBuf,
}

impl Session {
  pub fn new(dir: PathBuf) -> Self {
    Self { dir }
  }

  pub fn create(repo_root: &Path, slug: &str) -> Result<Self> {
    Self::create_in(&cache_root(), repo_root, slug)
  }

  pub fn create_in(cache: &Path, repo_root: &Path, slug: &str) -> Result<Self> {
    let dir = cache.join(fingerprint(repo_root)).join(slug);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(Self::new(dir))
  }

  pub fn meta_path(&self) -> PathBuf {
    self.dir.join("meta.json")
  }

  pub fn hunks_path(&self) -> PathBuf {
    self.dir.join("hunks.json")
  }

  pub fn review_path(&self) -> PathBuf {
    self.dir.join("review.json")
  }

  pub fn state_path(&self) -> PathBuf {
    self.dir.join("comments.json")
  }

  pub fn meta(&self) -> Result<Meta> {
    match read_json(&self.meta_path())? {
      Some(meta) => Ok(meta),
      None => bail!("session has no meta.json: {}", self.dir.display()),
    }
  }

  pub fn hunks(&self) -> Result<Hunks> {
    Ok(read_json(&self.hunks_path())?.unwrap_or_default())
  }

  pub fn review(&self) -> Result<Option<Review>> {
    read_json(&self.review_path())
  }

  pub fn state(&self) -> Result<ReviewState> {
    if let Some(state) = read_json(&self.state_path())? {
      return Ok(state);
    }
    let fresh = ReviewState::default();
    self.save_state(&fresh)?;
    Ok(fresh)
  }

  pub fn save_state(&self, state: &ReviewState) -> Result<()> {
    write_json(&self.state_path(), state)
  }
}

/// `<basename>-<sha256(path)[..8]>`, the TypeScript's naming exactly.
fn fingerprint(repo_root: &Path) -> String {
  let path = repo_root.to_string_lossy();
  let digest = Sha256::digest(path.as_bytes());
  let name = repo_root.file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into());
  format!("{name}-{}", &hex(&digest)[..8])
}

fn hex(bytes: &[u8]) -> String {
  bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
    use std::fmt::Write;
    let _ = write!(out, "{byte:02x}");
    out
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn slugifies_like_the_typescript() {
    assert_eq!(slugify("Fix the SearchBar"), "fix-the-searchbar");
    assert_eq!(slugify("feat/scix-123_thing"), "feat-scix-123-thing");
    assert_eq!(slugify("--wrapped--"), "wrapped");
    assert_eq!(slugify("!!!"), "review");
    assert_eq!(slugify(""), "review");
  }

  #[test]
  fn caps_the_slug_at_48_characters() {
    let slug = slugify(&"a".repeat(60));
    assert_eq!(slug.len(), SLUG_MAX);
  }

  #[test]
  fn stamps_seconds_without_milliseconds() {
    let stamp = now_iso();
    assert_eq!(stamp.len(), 20, "{stamp}");
    assert!(stamp.ends_with('Z'), "{stamp}");
    assert!(!stamp.contains('.'), "{stamp}");
  }

  #[test]
  fn hashes_the_repo_root_into_the_directory_name() {
    // The hash is sha256 of the path, truncated — same value the TypeScript
    // produces, so a session written by either implementation is found by the
    // other while both exist.
    assert_eq!(fingerprint(Path::new("/home/tim/code/diffpane")), "diffpane-2541a5e3");
    assert_ne!(fingerprint(Path::new("/tmp/diffpane")), "diffpane-2541a5e3");
  }

  #[test]
  fn round_trips_state_and_seeds_a_missing_file() {
    let temp = tempfile::tempdir().unwrap();
    let session =
      Session::create_in(temp.path(), Path::new("/repo/thing"), "2026-08-28-x").unwrap();

    assert!(!session.state_path().exists());
    let seeded = session.state().unwrap();
    assert!(!seeded.submitted);
    assert!(session.state_path().exists());

    let mut state = session.state().unwrap();
    state.submitted = true;
    state.submitted_at = Some(now_iso());
    session.save_state(&state).unwrap();
    assert!(session.state().unwrap().submitted);
  }

  #[test]
  fn reports_a_session_with_no_meta() {
    let temp = tempfile::tempdir().unwrap();
    let session = Session::new(temp.path().to_path_buf());
    let message = session.meta().unwrap_err().to_string();
    assert!(message.contains("no meta.json"), "{message}");
    assert!(session.review().unwrap().is_none());
    assert!(session.hunks().unwrap().files.is_empty());
  }

  #[test]
  fn leaves_no_temp_file_behind() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested").join("hunks.json");
    write_json(&path, &Hunks::default()).unwrap();

    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
      .unwrap()
      .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
      .collect();
    assert_eq!(names, ["hunks.json"]);
  }
}

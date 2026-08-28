//! Structured diff extraction: `gix` supplies the changed-file set, `hunks`
//! turns each file's content into diffpane's hunk shape.
//!
//! The file list never comes from patch text. Binary, rename-only and mode-only
//! changes have no `---`/`+++` lines at all, and a patch-driven list drops them
//! silently — once reported as "no changes to review" and exited 0.

pub mod hunks;
pub mod staged;
pub mod tree;
pub mod worktree;

use anyhow::Result;
use gix::bstr::{BString, ByteSlice};
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::object::tree::EntryKind;
use gix::worktree::stack::state::attributes::Source;

use crate::classify::{is_noise, language_of};
use crate::model::{FileDiff, FileStatus};

const MAX_LINES_PER_FILE: usize = 2000;
const MAX_LINES_PER_NOISE_FILE: usize = 40;

/// One changed file, before its content has been diffed.
pub struct Change {
  pub path: BString,
  pub old_path: Option<BString>,
  pub status: FileStatus,
  pub old: Option<(gix::ObjectId, EntryKind)>,
  pub new: Option<(gix::ObjectId, EntryKind)>,
}

/// What to diff. Resolved from the CLI scope by `crate::scope`.
pub enum Plan {
  /// Index versus worktree, i.e. bare `git diff`.
  Working,
  /// HEAD tree versus index, i.e. `git diff --cached`.
  Staged,
  /// Tree versus tree, i.e. `git diff <base> <head>`.
  Trees { base: String, head: String },
}

pub fn entry_kind(mode: gix::index::entry::Mode) -> EntryKind {
  use gix::index::entry::Mode;
  match mode {
    Mode::SYMLINK => EntryKind::Link,
    m if m.contains(Mode::FILE_EXECUTABLE) => EntryKind::BlobExecutable,
    _ => EntryKind::Blob,
  }
}

/// git applies a pathspec before rename detection, so a rewrite with only one
/// side inside the spec cannot be paired and comes back out as a plain addition
/// or deletion. Detection has already run by the time we get here, so undo the
/// pairing rather than reporting a rename whose other half is filtered out.
fn split_rewrite(change: Change, new_matches: bool, old_matches: bool) -> Option<Change> {
  let copied = change.status == FileStatus::Copied;
  match (new_matches, old_matches) {
    (true, true) => Some(change),
    (true, false) => {
      Some(Change { old_path: None, status: FileStatus::Added, old: None, ..change })
    }
    // A copy leaves its source in place, so a source-only match has nothing to
    // show; a rename's source is genuinely gone.
    (false, true) if !copied => Some(Change {
      path: change.old_path?,
      old_path: None,
      status: FileStatus::Deleted,
      old: change.old,
      new: None,
    }),
    _ => None,
  }
}

fn apply_pathspec(
  repo: &gix::Repository,
  changes: Vec<Change>,
  paths: &[String],
) -> Result<Vec<Change>> {
  if paths.is_empty() {
    return Ok(changes);
  }
  let index = repo.index_or_empty()?;
  // Attributes come from the index, not the worktree: `:(attr:...)` magic is
  // rare and reading it from disk would make a bare repo a special case.
  let mut spec =
    repo.pathspec(true, paths.iter().map(String::as_str), true, &index, Source::IdMapping)?;

  let mut kept = Vec::with_capacity(changes.len());
  for change in changes {
    let new_matches = spec.is_included(change.path.as_bstr(), Some(false));
    let old_matches =
      change.old_path.as_ref().is_some_and(|path| spec.is_included(path.as_bstr(), Some(false)));
    let kept_change = if change.old_path.is_some() {
      split_rewrite(change, new_matches, old_matches)
    } else if new_matches {
      Some(change)
    } else {
      None
    };
    kept.extend(kept_change);
  }
  Ok(kept)
}

pub fn files(repo: &gix::Repository, plan: &Plan, paths: &[String]) -> Result<Vec<FileDiff>> {
  let changes = match plan {
    Plan::Working => worktree::changes(repo)?,
    Plan::Staged => staged::changes(repo)?,
    Plan::Trees { base, head } => tree::changes(repo, base, head)?,
  };
  let mut changes = apply_pathspec(repo, changes, paths)?;
  // git orders by path, and hunk ids are positional (`f<file>h<hunk>`), so the
  // order is part of the contract: review.json refers to these ids.
  changes.sort_by(|a, b| a.path.cmp(&b.path));

  // Worktree content is read from disk, so the blob pipeline needs a root to
  // read it from; the other plans have neither side on disk.
  let roots = match plan {
    Plan::Working => {
      WorktreeRoots { old_root: None, new_root: repo.workdir().map(std::path::Path::to_path_buf) }
    }
    Plan::Staged | Plan::Trees { .. } => WorktreeRoots::default(),
  };
  let mut cache = repo.diff_resource_cache(Mode::ToGit, roots)?;

  let mut files = Vec::with_capacity(changes.len());
  for (index, change) in changes.iter().enumerate() {
    let path = change.path.to_str_lossy().into_owned();
    let noise = is_noise(&path);
    let cap = if noise { MAX_LINES_PER_NOISE_FILE } else { MAX_LINES_PER_FILE };
    let content = hunks::content(repo, &mut cache, change, cap)?;
    let mut hunks = content.hunks;
    for (hunk_index, hunk) in hunks.iter_mut().enumerate() {
      hunk.id = format!("f{index}h{hunk_index}");
    }
    files.push(FileDiff {
      id: format!("f{index}"),
      old_path: change
        .old_path
        .as_ref()
        .map_or_else(|| path.clone(), |old| old.to_str_lossy().into_owned()),
      language: language_of(&path).map(str::to_owned),
      path,
      status: change.status,
      additions: content.additions,
      deletions: content.deletions,
      binary: content.binary,
      noise,
      truncated: content.truncated,
      hunks: if content.binary { Vec::new() } else { hunks },
    });
  }
  Ok(files)
}

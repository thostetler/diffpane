//! Structured diff extraction: `gix` supplies the changed-file set, `hunks`
//! turns each file's content into diffpane's hunk shape.
//!
//! The file list never comes from patch text. Binary, rename-only and mode-only
//! changes have no `---`/`+++` lines at all, and a patch-driven list drops them
//! silently — once reported as "no changes to review" and exited 0.

pub mod hunks;
pub mod tree;
pub mod worktree;

use anyhow::Result;
use gix::bstr::{BString, ByteSlice};
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::object::tree::EntryKind;

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
  /// Tree versus tree, i.e. `git diff <base> <head>`.
  Trees { base: String, head: String },
}

pub fn files(repo: &gix::Repository, plan: &Plan) -> Result<Vec<FileDiff>> {
  let mut changes = match plan {
    Plan::Working => worktree::changes(repo)?,
    Plan::Trees { base, head } => tree::changes(repo, base, head)?,
  };
  // git orders by path, and hunk ids are positional (`f<file>h<hunk>`), so the
  // order is part of the contract: review.json refers to these ids.
  changes.sort_by(|a, b| a.path.cmp(&b.path));

  // Worktree content is read from disk, so the blob pipeline needs a root to
  // read it from; a tree-to-tree diff has neither side on disk.
  let roots = match plan {
    Plan::Working => {
      WorktreeRoots { old_root: None, new_root: repo.workdir().map(std::path::Path::to_path_buf) }
    }
    Plan::Trees { .. } => WorktreeRoots::default(),
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

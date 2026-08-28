//! Index versus worktree: bare `git diff`.
//!
//! Untracked files never appear, which matches `git diff` exactly. It is still
//! a product gap for "review what the agent just wrote" — the TypeScript has
//! the same hole, and closing it is a separate decision.

use anyhow::Result;
use gix::object::tree::EntryKind;
use gix::status::index_worktree::Item;
use gix::status::plumbing::index_as_worktree::{Change as WtChange, EntryStatus};

use super::{Change, entry_kind};
use crate::model::FileStatus;

pub fn changes(repo: &gix::Repository) -> Result<Vec<Change>> {
  let mut changes = Vec::new();
  let iter = repo
    .status(gix::progress::Discard)?
    .untracked_files(gix::status::UntrackedFiles::None)
    .index_worktree_submodules(None)
    .into_index_worktree_iter(Vec::new())?;

  for item in iter {
    let Item::Modification { entry, rela_path, status, .. } = item? else { continue };
    let old = Some((entry.id, entry_kind(entry.mode)));
    // A null id tells the blob pipeline to read the worktree file instead.
    let worktree = |mode| Some((gix::ObjectId::null(repo.object_hash()), entry_kind(mode)));
    let (status, new) = match status {
      EntryStatus::Change(WtChange::Removed) => (FileStatus::Deleted, None),
      EntryStatus::Change(WtChange::Modification { .. }) => {
        (FileStatus::Modified, worktree(entry.mode))
      }
      EntryStatus::Change(WtChange::Type { worktree_mode }) => {
        (FileStatus::Modified, worktree(worktree_mode))
      }
      EntryStatus::IntentToAdd => {
        (FileStatus::Added, Some((gix::ObjectId::null(repo.object_hash()), EntryKind::Blob)))
      }
      _ => continue,
    };
    changes.push(Change { path: rela_path, old_path: None, status, old, new });
  }
  Ok(changes)
}

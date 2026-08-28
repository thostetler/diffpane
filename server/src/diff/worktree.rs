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

/// Index of stage 2 ("ours") in a conflict's stage 1..3 entry array.
const OURS_STAGE: usize = 1;

pub fn changes(repo: &gix::Repository) -> Result<Vec<Change>> {
  let mut changes = Vec::new();
  let iter = repo
    .status(gix::progress::Discard)?
    .untracked_files(gix::status::UntrackedFiles::None)
    .index_worktree_submodules(None)
    .into_index_worktree_iter(Vec::new())?;

  for item in iter {
    let Item::Modification { entry, rela_path, status, .. } = item? else { continue };
    let indexed = Some((entry.id, entry_kind(entry.mode)));
    // A null id tells the blob pipeline to read the worktree file instead.
    let worktree = |mode| Some((gix::ObjectId::null(repo.object_hash()), entry_kind(mode)));
    let (status, old, new) = match status {
      EntryStatus::Change(WtChange::Removed) => (FileStatus::Deleted, indexed, None),
      EntryStatus::Change(WtChange::Modification { .. }) => {
        (FileStatus::Modified, indexed, worktree(entry.mode))
      }
      EntryStatus::Change(WtChange::Type { worktree_mode }) => {
        (FileStatus::Modified, indexed, worktree(worktree_mode))
      }
      EntryStatus::IntentToAdd => (
        FileStatus::Added,
        indexed,
        Some((gix::ObjectId::null(repo.object_hash()), EntryKind::Blob)),
      ),
      // Conflicted paths have no stage 0, so dropping them would let a review
      // of a mid-merge worktree report "no changes" and exit 0 — silent
      // approval of exactly the files that need looking at. git shows a
      // combined diff here, which the hunk model cannot represent; "ours"
      // against the worktree can, and it carries what the reviewer needs: the
      // conflict markers, plus whatever resolution has happened so far.
      EntryStatus::Conflict { entries, .. } => match &entries[OURS_STAGE] {
        Some(ours) => {
          (FileStatus::Modified, Some((ours.id, entry_kind(ours.mode))), worktree(ours.mode))
        }
        None => (FileStatus::Added, None, worktree(entry.mode)),
      },
      // Submodule reporting is switched off above, so that arm is unreachable;
      // matching it by name rather than `_` keeps a new gix variant a compile
      // error instead of a silently dropped file.
      EntryStatus::Change(WtChange::SubmoduleModification(_)) | EntryStatus::NeedsUpdate(_) => {
        continue;
      }
    };
    changes.push(Change { path: rela_path, old_path: None, status, old, new });
  }
  Ok(changes)
}

#[cfg(test)]
mod tests {
  use std::path::Path;
  use std::process::Command;

  fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
      .current_dir(repo)
      .args(args)
      .env("GIT_AUTHOR_NAME", "t")
      .env("GIT_AUTHOR_EMAIL", "t@example.com")
      .env("GIT_COMMITTER_NAME", "t")
      .env("GIT_COMMITTER_EMAIL", "t@example.com")
      .output()
      .expect("git runs")
  }

  fn write(repo: &Path, name: &str, body: &str) {
    std::fs::write(repo.join(name), body).unwrap();
  }

  /// A merge left mid-conflict: `main` and `other` both rewrote `a.txt`.
  fn conflicted_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    git(repo, &["init", "-q", "-b", "main"]);
    write(repo, "a.txt", "base\n");
    git(repo, &["add", "a.txt"]);
    git(repo, &["commit", "-qm", "base"]);
    git(repo, &["checkout", "-q", "-b", "other"]);
    write(repo, "a.txt", "theirs\n");
    git(repo, &["commit", "-qam", "theirs"]);
    git(repo, &["checkout", "-q", "main"]);
    write(repo, "a.txt", "ours\n");
    git(repo, &["commit", "-qam", "ours"]);
    let merge = git(repo, &["merge", "other"]);
    assert!(!merge.status.success(), "the merge is supposed to conflict");
    temp
  }

  #[test]
  fn a_conflicted_file_is_still_a_change_to_review() {
    let temp = conflicted_repo();
    let repo = gix::open(temp.path()).unwrap();
    let changes = super::changes(&repo).unwrap();
    let paths: Vec<String> = changes.iter().map(|change| change.path.to_string()).collect();
    assert_eq!(paths, ["a.txt"], "dropping it would report a clean worktree and exit 0");
  }

  #[test]
  fn a_conflicted_file_diffs_ours_against_the_markers() {
    let temp = conflicted_repo();
    let repo = gix::open(temp.path()).unwrap();
    let changes = super::changes(&repo).unwrap();
    let change = &changes[0];
    assert_eq!(change.status, crate::model::FileStatus::Modified);
    let ours = change.old.expect("stage 2 is the old side").0;
    let blob = repo.find_object(ours).unwrap().detach().data;
    assert_eq!(blob, b"ours\n");
    assert!(change.new.expect("the worktree is the new side").0.is_null());
  }
}

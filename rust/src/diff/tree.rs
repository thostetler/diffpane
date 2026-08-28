//! Tree versus tree, with rename tracking: `git diff <base> <head>`.

use anyhow::Result;
use gix::diff::tree_with_rewrites::Change as TreeChange;

use super::Change;
use crate::model::FileStatus;

/// git's `diff.renames` defaults, pinned rather than read from the user's
/// config so a review shows the same files everywhere — the same reason the
/// diff algorithm is pinned. See docs/contract.md.
///
/// `limit` is the one place gix does not match git: git budgets
/// `diff.renameLimit` *squared* similarity checks (1000² by default) while
/// gix compares the raw permutation count against this field, so its 1000
/// default gives up on inexact renames at roughly 31 added × 31 deleted files.
/// A directory move of 40 edited files came back as 80 unrelated add/deletes.
fn rewrites() -> gix::diff::Rewrites {
  gix::diff::Rewrites {
    copies: None,
    percentage: Some(0.5),
    limit: 1000 * 1000,
    ..Default::default()
  }
}

pub fn changes(repo: &gix::Repository, base: &str, head: &str) -> Result<Vec<Change>> {
  let old_tree = repo.rev_parse_single(base)?.object()?.peel_to_tree()?;
  let new_tree = repo.rev_parse_single(head)?.object()?.peel_to_tree()?;

  let mut changes = Vec::new();
  let options = gix::diff::Options::default().with_rewrites(Some(rewrites()));
  repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), options)?.into_iter().for_each(
    |change| {
      // gix reports the containing tree as changed as well as its entries.
      // `git diff --raw` lists only the entries, and a tree has no diffable
      // content — `set_resource` rejects a tree mode outright.
      if change.entry_mode().is_tree() {
        return;
      }
      changes.push(match change {
        TreeChange::Addition { location, id, entry_mode, .. } => Change {
          path: location.clone(),
          old_path: None,
          status: FileStatus::Added,
          old: None,
          new: Some((id, entry_mode.kind())),
        },
        TreeChange::Deletion { location, id, entry_mode, .. } => Change {
          path: location.clone(),
          old_path: None,
          status: FileStatus::Deleted,
          old: Some((id, entry_mode.kind())),
          new: None,
        },
        TreeChange::Modification {
          location,
          previous_id,
          previous_entry_mode,
          id,
          entry_mode,
          ..
        } => Change {
          path: location.clone(),
          old_path: None,
          status: FileStatus::Modified,
          old: Some((previous_id, previous_entry_mode.kind())),
          new: Some((id, entry_mode.kind())),
        },
        TreeChange::Rewrite {
          source_location,
          source_id,
          source_entry_mode,
          location,
          id,
          entry_mode,
          copy,
          ..
        } => Change {
          path: location.clone(),
          old_path: Some(source_location.clone()),
          status: if copy { FileStatus::Copied } else { FileStatus::Renamed },
          old: Some((source_id, source_entry_mode.kind())),
          new: Some((id, entry_mode.kind())),
        },
      });
    },
  );
  Ok(changes)
}

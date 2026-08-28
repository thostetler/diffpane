//! HEAD tree versus index: `git diff --cached`.

use anyhow::Result;
use gix::diff::index::{Action, ChangeRef};
use gix::status::tree_index::TrackRenames;

use super::{Change, entry_kind};
use crate::model::FileStatus;

pub fn changes(repo: &gix::Repository) -> Result<Vec<Change>> {
  let tree_id = repo.head_tree_id_or_empty()?.detach();
  let index = repo.index_or_empty()?;

  let mut changes = Vec::new();
  repo.tree_index_status(&tree_id, &index, None, TrackRenames::AsConfigured, |change, _, _| {
    if let Some(change) = convert(&change) {
      changes.push(change);
    }
    Ok::<_, std::convert::Infallible>(Action::Continue(()))
  })?;
  Ok(changes)
}

/// Gitlinks have no blob to diff — git prints `Subproject commit <sha>` for them
/// from data the blob pipeline cannot supply, so they are dropped rather than
/// misrendered.
fn is_diffable(mode: gix::index::entry::Mode) -> bool {
  !mode.contains(gix::index::entry::Mode::COMMIT) && !mode.contains(gix::index::entry::Mode::DIR)
}

fn convert(change: &ChangeRef<'_, '_>) -> Option<Change> {
  Some(match change {
    ChangeRef::Addition { location, entry_mode, id, .. } => {
      if !is_diffable(*entry_mode) {
        return None;
      }
      Change {
        path: location.as_ref().into(),
        old_path: None,
        status: FileStatus::Added,
        old: None,
        new: Some((id.as_ref().to_owned(), entry_kind(*entry_mode))),
      }
    }
    ChangeRef::Deletion { location, entry_mode, id, .. } => {
      if !is_diffable(*entry_mode) {
        return None;
      }
      Change {
        path: location.as_ref().into(),
        old_path: None,
        status: FileStatus::Deleted,
        old: Some((id.as_ref().to_owned(), entry_kind(*entry_mode))),
        new: None,
      }
    }
    ChangeRef::Modification {
      location, previous_entry_mode, previous_id, entry_mode, id, ..
    } => {
      if !is_diffable(*entry_mode) || !is_diffable(*previous_entry_mode) {
        return None;
      }
      Change {
        path: location.as_ref().into(),
        old_path: None,
        status: FileStatus::Modified,
        old: Some((previous_id.as_ref().to_owned(), entry_kind(*previous_entry_mode))),
        new: Some((id.as_ref().to_owned(), entry_kind(*entry_mode))),
      }
    }
    ChangeRef::Rewrite {
      source_location,
      source_entry_mode,
      source_id,
      location,
      entry_mode,
      id,
      copy,
      ..
    } => {
      if !is_diffable(*entry_mode) || !is_diffable(*source_entry_mode) {
        return None;
      }
      Change {
        path: location.as_ref().into(),
        old_path: Some(source_location.as_ref().into()),
        status: if *copy { FileStatus::Copied } else { FileStatus::Renamed },
        old: Some((source_id.as_ref().to_owned(), entry_kind(*source_entry_mode))),
        new: Some((id.as_ref().to_owned(), entry_kind(*entry_mode))),
      }
    }
  })
}

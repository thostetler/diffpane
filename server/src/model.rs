//! The wire contract, verbatim: the browser UI reads these shapes directly, so
//! the JSON is snake_case and stays that way.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
  Added,
  Modified,
  Deleted,
  Renamed,
  Copied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineType {
  Context,
  Add,
  Del,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
  Old,
  New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
  Ok,
  Fix,
  Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorKind {
  Line,
  File,
  Chapter,
  Overall,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
  #[default]
  Note,
  Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressState {
  Unreviewed,
  Reviewed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
  #[default]
  Branch,
  Working,
  Staged,
  Range,
  Commit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
  pub i: usize,
  #[serde(rename = "type")]
  pub kind: LineType,
  pub old: Option<u32>,
  pub new: Option<u32>,
  pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
  pub id: String,
  pub header: String,
  pub old_start: u32,
  pub old_count: u32,
  pub new_start: u32,
  pub new_count: u32,
  pub additions: usize,
  pub deletions: usize,
  pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
  pub id: String,
  pub path: String,
  pub old_path: String,
  pub status: FileStatus,
  pub additions: usize,
  pub deletions: usize,
  pub binary: bool,
  pub noise: bool,
  pub language: Option<String>,
  pub truncated: bool,
  pub hunks: Vec<Hunk>,
}

/// `hunks.json` in full. The UI reads it as `{ files }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hunks {
  pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Totals {
  pub files: usize,
  pub additions: usize,
  pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
  pub repo: String,
  pub repo_root: String,
  pub slug: String,
  pub title: String,
  pub scope: Scope,
  pub base: String,
  pub head: String,
  pub diff_cmd: String,
  pub generated_at: String,
  pub totals: Totals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
  pub id: String,
  pub title: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub intent: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub why: Option<String>,
  #[serde(default)]
  pub hunks: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub size: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub flags: Option<Vec<String>>,
}

/// An agent annotation pinned to one diff line. Distinct from a `Comment`: the
/// agent writes these before the review, the human writes comments during it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineNote {
  pub hunk: String,
  pub side: Side,
  pub line: u32,
  pub body: String,
  #[serde(default)]
  pub kind: NoteKind,
}

/// Authored by the agent, not by diffpane. Everything in it is optional, and a
/// chapter may reference a hunk id that no longer exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub story: Option<String>,
  #[serde(default)]
  pub chapters: Vec<Chapter>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub file_notes: Option<std::collections::BTreeMap<String, String>>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub line_notes: Vec<LineNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
  pub kind: AnchorKind,
  pub file: Option<String>,
  pub hunk: Option<String>,
  pub side: Option<Side>,
  pub line: Option<u32>,
  pub chapter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
  pub id: String,
  pub anchor: Anchor,
  pub verdict: Verdict,
  pub body: String,
  pub created_at: String,
  pub updated_at: String,
  pub resolved: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Overall {
  pub verdict: Option<Verdict>,
  pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewState {
  pub comments: Vec<Comment>,
  pub progress: std::collections::BTreeMap<String, ProgressState>,
  pub overall: Overall,
  pub submitted: bool,
  pub submitted_at: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_review_without_line_notes_parses() {
    let review: Review = serde_json::from_str(r#"{"chapters":[]}"#).unwrap();
    assert!(review.line_notes.is_empty());
  }

  #[test]
  fn a_line_note_without_a_kind_is_a_plain_note() {
    let body =
      r#"{"chapters":[],"line_notes":[{"hunk":"f0h1","side":"new","line":73,"body":"x"}]}"#;
    let review: Review = serde_json::from_str(body).unwrap();
    assert_eq!(review.line_notes[0].kind, NoteKind::Note);
    assert_eq!(review.line_notes[0].side, Side::New);
  }
}

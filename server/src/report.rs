//! The report the agent reads back: markdown by default, `--json` on request.
//!
//! `--json` is the shape the `/diffpane` skill parses, and the exit code comes
//! from `outcome_of`, so both are contract.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::{
  Anchor, AnchorKind, Comment, FileDiff, LineType, Meta, ProgressState, Review, ReviewState, Side,
  Totals, Verdict,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
  Approved,
  ChangesRequested,
  Abandoned,
}

impl Outcome {
  pub fn exit_code(self) -> i32 {
    match self {
      Self::Approved => 0,
      Self::ChangesRequested => 1,
      Self::Abandoned => 2,
    }
  }
}

pub struct ReportInput<'a> {
  pub meta: &'a Meta,
  pub files: &'a [FileDiff],
  pub review: Option<&'a Review>,
  pub state: &'a ReviewState,
}

fn verdict_mark(verdict: Verdict) -> &'static str {
  match verdict {
    Verdict::Ok => "[ok]",
    Verdict::Fix => "[FIX]",
    Verdict::Question => "[?]",
  }
}

pub fn open_comments(state: &ReviewState) -> Vec<&Comment> {
  state.comments.iter().filter(|comment| !comment.resolved).collect()
}

pub fn outcome_of(state: &ReviewState) -> Outcome {
  if !state.submitted {
    return Outcome::Abandoned;
  }
  let blocking = open_comments(state).iter().any(|comment| comment.verdict == Verdict::Fix);
  if blocking || state.overall.verdict == Some(Verdict::Fix) {
    Outcome::ChangesRequested
  } else {
    Outcome::Approved
  }
}

/// The diff line a comment is pinned to, for quoting back in the report.
pub fn line_text(files: &[FileDiff], anchor: &Anchor) -> Option<String> {
  let path = anchor.file.as_deref()?;
  for file in files.iter().filter(|file| file.path == path) {
    for hunk in &file.hunks {
      if anchor.hunk.as_ref().is_some_and(|id| *id != hunk.id) {
        continue;
      }
      for line in &hunk.lines {
        let side = if anchor.side == Some(Side::Old) { line.old } else { line.new };
        if side == anchor.line && side.is_some() {
          let marker = match line.kind {
            LineType::Add => '+',
            LineType::Del => '-',
            LineType::Context => ' ',
          };
          return Some(format!("{marker}{}", line.text));
        }
      }
    }
  }
  None
}

/// A fence must be longer than any backtick run inside the content it wraps.
fn fence_for(content: &str) -> String {
  let mut longest = 0;
  let mut run = 0;
  for ch in content.chars() {
    run = if ch == '`' { run + 1 } else { 0 };
    longest = longest.max(run);
  }
  "`".repeat(3.max(longest + 1))
}

fn chapter_titles(review: Option<&Review>) -> BTreeMap<&str, &str> {
  review
    .map(|review| review.chapters.iter().map(|c| (c.id.as_str(), c.title.as_str())).collect())
    .unwrap_or_default()
}

fn group_key(comment: &Comment, chapters: &BTreeMap<&str, &str>) -> String {
  if let Some(file) = &comment.anchor.file {
    return file.clone();
  }
  if comment.anchor.kind == AnchorKind::Chapter
    && let Some(chapter) = &comment.anchor.chapter
  {
    let title = chapters.get(chapter.as_str()).copied().unwrap_or(chapter.as_str());
    return format!("chapter: {title}");
  }
  "general".to_string()
}

/// Insertion-ordered, like the TypeScript's `Map`: the report follows the order
/// the comments were made in, not alphabetical order.
fn group_comments<'a>(
  comments: &[&'a Comment],
  chapters: &BTreeMap<&str, &str>,
) -> Vec<(String, Vec<&'a Comment>)> {
  let mut groups: Vec<(String, Vec<&Comment>)> = Vec::new();
  for comment in comments {
    let key = group_key(comment, chapters);
    match groups.iter_mut().find(|(existing, _)| *existing == key) {
      Some((_, bucket)) => bucket.push(comment),
      None => groups.push((key, vec![comment])),
    }
  }
  for (_, bucket) in &mut groups {
    bucket.sort_by_key(|comment| comment.anchor.line.unwrap_or(0));
  }
  groups
}

pub fn build_markdown(input: &ReportInput) -> String {
  let ReportInput { meta, files, review, state } = *input;
  let chapters = chapter_titles(review);
  let open = open_comments(state);
  let resolved = state.comments.len() - open.len();
  let status = if state.submitted { "submitted" } else { "IN PROGRESS (not submitted)" };

  let mut lines = vec![
    format!("# Review feedback — {}", meta.title),
    String::new(),
    format!("Scope: `{}` · {status}", meta.diff_cmd),
  ];

  if state.overall.verdict.is_some() || !state.overall.body.is_empty() {
    let verdict = state.overall.verdict.map_or("n/a", verdict_word);
    lines.push(String::new());
    lines.push(format!("**Overall [{verdict}]** {}", state.overall.body));
  }
  lines.push(String::new());
  lines.push(format!("{} open comment(s), {resolved} resolved.", open.len()));

  for (key, group) in group_comments(&open, &chapters) {
    lines.push(String::new());
    lines.push(format!("## {key}"));
    for comment in group {
      let where_ = comment.anchor.line.map(|line| format!(":{line}")).unwrap_or_default();
      let mut body = comment.body.split('\n');
      let first = body.next().unwrap_or_default();
      lines.push(String::new());
      lines.push(format!("- **{} {key}{where_}** — {first}", verdict_mark(comment.verdict)));
      // Continuation lines must stay indented or they escape the list item.
      for line in body {
        lines.push(format!("  {line}"));
      }
      let snippet = match comment.anchor.kind {
        AnchorKind::Line => line_text(files, &comment.anchor),
        _ => None,
      };
      if let Some(snippet) = snippet {
        let fence = fence_for(&snippet);
        lines.push(format!("  {fence}"));
        lines.push(format!("  {snippet}"));
        lines.push(format!("  {fence}"));
      }
    }
  }

  let unreviewed: Vec<&str> = review
    .map(|review| {
      review
        .chapters
        .iter()
        .filter(|chapter| state.progress.get(&chapter.id) != Some(&ProgressState::Reviewed))
        .map(|chapter| chapter.title.as_str())
        .collect()
    })
    .unwrap_or_default();
  if !unreviewed.is_empty() {
    lines.push(String::new());
    lines.push(format!("Chapters not marked reviewed: {}", unreviewed.join(", ")));
  }

  let mut out = lines.join("\n");
  out.push('\n');
  out
}

fn verdict_word(verdict: Verdict) -> &'static str {
  match verdict {
    Verdict::Ok => "ok",
    Verdict::Fix => "fix",
    Verdict::Question => "question",
  }
}

#[derive(Debug, Serialize)]
pub struct JsonComment<'a> {
  pub verdict: Verdict,
  pub body: &'a str,
  pub file: Option<&'a str>,
  pub line: Option<u32>,
  pub kind: AnchorKind,
  pub chapter: Option<&'a str>,
  pub code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonReport<'a> {
  pub outcome: Outcome,
  pub submitted: bool,
  pub submitted_at: Option<&'a str>,
  pub scope: &'a str,
  pub totals: Totals,
  pub overall: &'a crate::model::Overall,
  pub progress: &'a BTreeMap<String, ProgressState>,
  pub comments: Vec<JsonComment<'a>>,
}

pub fn build_json<'a>(input: &ReportInput<'a>) -> JsonReport<'a> {
  let ReportInput { meta, files, state, .. } = *input;
  JsonReport {
    outcome: outcome_of(state),
    submitted: state.submitted,
    submitted_at: state.submitted_at.as_deref(),
    scope: &meta.diff_cmd,
    totals: meta.totals,
    overall: &state.overall,
    progress: &state.progress,
    comments: open_comments(state)
      .into_iter()
      .map(|comment| JsonComment {
        verdict: comment.verdict,
        body: &comment.body,
        file: comment.anchor.file.as_deref(),
        line: comment.anchor.line,
        kind: comment.anchor.kind,
        chapter: comment.anchor.chapter.as_deref(),
        code: match comment.anchor.kind {
          AnchorKind::Line => line_text(files, &comment.anchor),
          _ => None,
        },
      })
      .collect(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::{DiffLine, FileStatus, Hunk, Overall, Scope};

  fn files() -> Vec<FileDiff> {
    vec![FileDiff {
      id: "f0".into(),
      path: "src/a.ts".into(),
      old_path: "src/a.ts".into(),
      status: FileStatus::Modified,
      additions: 1,
      deletions: 1,
      binary: false,
      noise: false,
      language: Some("typescript".into()),
      truncated: false,
      hunks: vec![Hunk {
        id: "f0h0".into(),
        header: "@@ -1,2 +1,2 @@".into(),
        old_start: 1,
        old_count: 2,
        new_start: 1,
        new_count: 2,
        additions: 1,
        deletions: 1,
        lines: vec![
          DiffLine {
            i: 0,
            kind: LineType::Del,
            old: Some(1),
            new: None,
            text: "const a = 1;".into(),
          },
          DiffLine {
            i: 1,
            kind: LineType::Add,
            old: None,
            new: Some(1),
            text: "const a = 2;".into(),
          },
        ],
      }],
    }]
  }

  fn meta() -> Meta {
    Meta {
      repo: "demo".into(),
      repo_root: "/tmp/demo".into(),
      slug: "2026-01-01-demo".into(),
      title: "Demo".into(),
      scope: Scope::Branch,
      base: "main".into(),
      head: "feature".into(),
      diff_cmd: "git diff main...HEAD".into(),
      generated_at: "2026-01-01T00:00:00Z".into(),
      totals: Totals { files: 1, additions: 1, deletions: 1 },
    }
  }

  fn anchor(file: Option<&str>) -> Anchor {
    Anchor {
      kind: AnchorKind::Line,
      file: file.map(str::to_string),
      hunk: None,
      side: None,
      line: None,
      chapter: None,
    }
  }

  fn comment(verdict: Verdict, anchor: Anchor, resolved: bool) -> Comment {
    Comment {
      id: format!("c-{}", verdict_word(verdict)),
      anchor,
      verdict,
      body: format!("{} note", verdict_word(verdict)),
      created_at: "2026-01-01T00:00:00Z".into(),
      updated_at: "2026-01-01T00:00:00Z".into(),
      resolved,
    }
  }

  fn anchored_line() -> Anchor {
    Anchor {
      hunk: Some("f0h0".into()),
      side: Some(Side::New),
      line: Some(1),
      ..anchor(Some("src/a.ts"))
    }
  }

  fn submitted(comments: Vec<Comment>) -> ReviewState {
    ReviewState { submitted: true, comments, ..ReviewState::default() }
  }

  #[test]
  fn finds_the_diff_line_an_anchor_points_at() {
    let files = files();
    assert_eq!(line_text(&files, &anchored_line()).unwrap(), "+const a = 2;");
    let old = Anchor { side: Some(Side::Old), ..anchored_line() };
    assert_eq!(line_text(&files, &old).unwrap(), "-const a = 1;");
    assert_eq!(line_text(&files, &Anchor { line: Some(99), ..anchored_line() }), None);
    let elsewhere = Anchor { file: Some("nope.ts".into()), ..anchored_line() };
    assert_eq!(line_text(&files, &elsewhere), None);
  }

  #[test]
  fn an_unsubmitted_review_is_abandoned() {
    assert_eq!(outcome_of(&ReviewState::default()), Outcome::Abandoned);
    assert_eq!(Outcome::Abandoned.exit_code(), 2);
  }

  #[test]
  fn submitting_with_no_open_fix_comments_is_approval() {
    let state = submitted(vec![comment(Verdict::Question, anchor(Some("src/a.ts")), false)]);
    assert_eq!(outcome_of(&state), Outcome::Approved);
  }

  #[test]
  fn an_open_fix_comment_requests_changes() {
    let state = submitted(vec![comment(Verdict::Fix, anchor(Some("src/a.ts")), false)]);
    assert_eq!(outcome_of(&state), Outcome::ChangesRequested);
    assert_eq!(Outcome::ChangesRequested.exit_code(), 1);
  }

  #[test]
  fn a_resolved_fix_comment_no_longer_blocks() {
    let state = submitted(vec![comment(Verdict::Fix, anchor(Some("src/a.ts")), true)]);
    assert_eq!(outcome_of(&state), Outcome::Approved);
  }

  #[test]
  fn an_overall_fix_verdict_requests_changes_on_its_own() {
    let state = ReviewState {
      overall: Overall { verdict: Some(Verdict::Fix), body: "no".into() },
      ..submitted(vec![])
    };
    assert_eq!(outcome_of(&state), Outcome::ChangesRequested);
  }

  #[test]
  fn markdown_groups_by_file_and_quotes_the_anchored_line() {
    let files = files();
    let meta = meta();
    let state = submitted(vec![comment(Verdict::Fix, anchored_line(), false)]);
    let markdown =
      build_markdown(&ReportInput { meta: &meta, files: &files, review: None, state: &state });
    assert!(markdown.contains("## src/a.ts"), "{markdown}");
    assert!(markdown.contains("[FIX] src/a.ts:1"), "{markdown}");
    assert!(markdown.contains("+const a = 2;"), "{markdown}");
    assert!(markdown.contains("1 open comment(s), 0 resolved"), "{markdown}");
  }

  #[test]
  fn markdown_reports_an_in_progress_review_as_unsubmitted() {
    let files = files();
    let meta = meta();
    let state = ReviewState::default();
    let markdown =
      build_markdown(&ReportInput { meta: &meta, files: &files, review: None, state: &state });
    assert!(markdown.contains("IN PROGRESS (not submitted)"), "{markdown}");
  }

  #[test]
  fn markdown_indents_a_continuation_line_and_widens_the_fence() {
    let files = files();
    let meta = meta();
    let mut pinned = comment(Verdict::Fix, anchored_line(), false);
    pinned.body = "first\nsecond".into();
    let state = submitted(vec![pinned]);
    let markdown =
      build_markdown(&ReportInput { meta: &meta, files: &files, review: None, state: &state });
    assert!(markdown.contains("— first\n  second\n"), "{markdown}");
    assert_eq!(fence_for("no backticks"), "```");
    assert_eq!(fence_for("a ``` b"), "````");
  }

  #[test]
  fn json_output_carries_outcome_code_and_location() {
    let files = files();
    let meta = meta();
    let state = submitted(vec![comment(Verdict::Fix, anchored_line(), false)]);
    let report =
      build_json(&ReportInput { meta: &meta, files: &files, review: None, state: &state });
    assert_eq!(report.outcome, Outcome::ChangesRequested);
    assert_eq!(report.comments.len(), 1);
    assert_eq!(report.comments[0].file, Some("src/a.ts"));
    assert_eq!(report.comments[0].code.as_deref(), Some("+const a = 2;"));
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains(r#""outcome":"changes-requested""#), "{json}");
  }

  #[test]
  fn resolved_comments_stay_out_of_the_report() {
    let files = files();
    let meta = meta();
    let state = submitted(vec![comment(Verdict::Fix, anchor(Some("src/a.ts")), true)]);
    let markdown =
      build_markdown(&ReportInput { meta: &meta, files: &files, review: None, state: &state });
    assert!(markdown.contains("0 open comment(s), 1 resolved"), "{markdown}");
  }

  #[test]
  fn lists_unreviewed_chapters_by_title() {
    use crate::model::Chapter;
    let files = files();
    let meta = meta();
    let review = Review {
      title: None,
      story: None,
      chapters: vec![Chapter {
        id: "c1".into(),
        title: "The interesting one".into(),
        intent: None,
        why: None,
        hunks: vec![],
        size: None,
        flags: None,
      }],
      file_notes: None,
    };
    let state = submitted(vec![]);
    let markdown = build_markdown(&ReportInput {
      meta: &meta,
      files: &files,
      review: Some(&review),
      state: &state,
    });
    assert!(markdown.contains("Chapters not marked reviewed: The interesting one"), "{markdown}");
  }
}

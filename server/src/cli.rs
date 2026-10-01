//! The command's own logic.
//!
//! Everything here is a function `main` calls: printing and process exit live
//! in the binary, so this stays testable.

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result};
use jiff::Timestamp;
use tokio::sync::{mpsc, oneshot};

use crate::args::Options;
use crate::assets::Assets;
use crate::model::{FileDiff, Hunk, Hunks, LineNote, Meta, Review, ReviewState, Side, Totals};
use crate::report::{Outcome, ReportInput, build_json, build_markdown, outcome_of};
use crate::server::{AppState, bind, generate_token, serve};
use crate::session::{Session, now_iso, slugify, write_json};
use crate::wait::{Ending, wait_for_ending};
use crate::{diff, scope};

/// How long a shutdown may take before the CLI stops waiting for it. The
/// shutdown is graceful so an in-flight submit finishes first; this only bounds
/// a connection that never completes (item 21).
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

fn totals(files: &[FileDiff]) -> Totals {
  Totals {
    files: files.len(),
    additions: files.iter().map(|file| file.additions).sum(),
    deletions: files.iter().map(|file| file.deletions).sum(),
  }
}

fn today() -> String {
  Timestamp::now().strftime("%Y-%m-%d").to_string()
}

/// Chapters that point at a hunk id this diff does not have. Not an error: the
/// narrative is the agent's, and a stale id costs the reader one chapter, not
/// the review.
fn stale_chapter_refs(review: &Review, files: &[FileDiff]) -> Vec<String> {
  let hunks: std::collections::BTreeMap<&str, &Hunk> =
    files.iter().flat_map(|file| file.hunks.iter().map(|hunk| (hunk.id.as_str(), hunk))).collect();
  let mut warnings = Vec::new();
  for chapter in &review.chapters {
    for id in &chapter.hunks {
      if !hunks.contains_key(id.as_str()) {
        warnings.push(format!("chapter {} references unknown hunk {id}", chapter.id));
      }
    }
  }
  warnings
}

/// Keeps only the line notes this diff can actually render: a known hunk, a
/// line that hunk carries, and an anchor no earlier note already claimed. The
/// UI has one slot per anchor, so a second note there would either silently
/// replace the first or inherit its expanded state — dropping it loudly here
/// beats either.
fn valid_line_notes(
  review: &Review,
  files: &[FileDiff],
  warnings: &mut Vec<String>,
) -> Vec<LineNote> {
  let hunks: std::collections::BTreeMap<&str, &Hunk> =
    files.iter().flat_map(|file| file.hunks.iter().map(|hunk| (hunk.id.as_str(), hunk))).collect();
  let mut seen = std::collections::HashSet::new();
  let mut kept = Vec::new();
  for note in &review.line_notes {
    let side = match note.side {
      Side::Old => "old",
      Side::New => "new",
    };
    let Some(hunk) = hunks.get(note.hunk.as_str()) else {
      warnings.push(format!("line note references unknown hunk {}", note.hunk));
      continue;
    };
    let anchored = hunk.lines.iter().any(|line| match note.side {
      Side::Old => line.old == Some(note.line),
      Side::New => line.new == Some(note.line),
    });
    if !anchored {
      warnings.push(format!(
        "line note anchors to {side} line {}, which is not in hunk {}; dropped",
        note.line, note.hunk
      ));
      continue;
    }
    let key = format!("{}:{side}:{}", note.hunk, note.line);
    if !seen.insert(key) {
      warnings.push(format!(
        "duplicate line note on {side} line {} in hunk {}; keeping the first",
        note.line, note.hunk
      ));
      continue;
    }
    kept.push(note.clone());
  }
  kept
}

fn install_review(session: &Session, file: &str, files: &[FileDiff]) -> Result<()> {
  let body = std::fs::read_to_string(file).with_context(|| format!("read {file}"))?;
  let mut review: Review = serde_json::from_str(&body).with_context(|| format!("parse {file}"))?;
  let mut warnings = stale_chapter_refs(&review, files);
  review.line_notes = valid_line_notes(&review, files, &mut warnings);
  for warning in warnings {
    eprintln!("warning: {warning}");
  }
  write_json(&session.review_path(), &review)
}

const CANDIDATES_FILE: &str = "dev/comment-candidates.md";

fn install_candidates(session: &Session, root: &std::path::Path, files: &[FileDiff]) -> Result<()> {
  let path = root.join(CANDIDATES_FILE);
  let markdown = match std::fs::read_to_string(&path) {
    Ok(markdown) => markdown,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
    Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
  };
  write_json(&session.candidates_path(), &crate::candidates::build(&markdown, files))
}

pub struct Built {
  pub session: Session,
  pub meta: Meta,
}

fn remove_if_present(path: &std::path::Path) -> Result<()> {
  match std::fs::remove_file(path) {
    Ok(()) => Ok(()),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
    Err(error) => Err(error).context(format!("remove {}", path.display())),
  }
}

/// Re-running on the same branch the same day reuses the session directory, so
/// both of the previous run's mutable artefacts have to go. Comments would
/// replay a stale `submitted: true` and anchor to hunk ids that have moved; a
/// review left behind by an earlier `--review` would let chapters claim hunk
/// ids this diff does not have and make `PUT /api/progress` validate against a
/// chapter set nobody asked for.
fn clear_previous_run(session: &Session) -> Result<()> {
  write_json(&session.state_path(), &ReviewState::default())?;
  remove_if_present(&session.review_path())?;
  remove_if_present(&session.candidates_path())
}

/// Builds the session on disk, or `None` when there is nothing to review.
pub fn build_session(repo: &gix::Repository, options: &Options) -> Result<Option<Built>> {
  let root = repo.workdir().context("diffpane needs a work tree")?.to_path_buf();
  let request = scope::Request {
    scope: options.scope,
    base: options.base.clone(),
    range: options.range.clone(),
    commit: options.commit.clone(),
    paths: options.paths.clone(),
  };
  let resolved = scope::resolve(repo, &request)?;
  let files = diff::files(repo, &resolved.plan, &resolved.paths)?;
  if files.is_empty() {
    return Ok(None);
  }

  let title = options.title.clone();
  let wanted = format!("{}-{}", today(), slugify(title.as_deref().unwrap_or(&resolved.head)));
  let session = Session::create(&root, &wanted)?;
  // Not `wanted`: a concurrent run on the same branch pushes this one into a
  // suffixed directory, and `meta.slug` names the directory the report and the
  // UI are reading.
  let slug = session.slug();
  let meta = Meta {
    repo: root
      .file_name()
      .map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned()),
    repo_root: root.display().to_string(),
    slug: slug.clone(),
    title: title.unwrap_or_else(|| slug.clone()),
    scope: resolved.scope,
    base: resolved.base,
    head: resolved.head,
    diff_cmd: resolved.diff_cmd,
    generated_at: now_iso(),
    totals: totals(&files),
  };
  write_json(&session.meta_path(), &meta)?;
  write_json(&session.hunks_path(), &Hunks { files: files.clone() })?;
  clear_previous_run(&session)?;
  if let Some(file) = options.review_file.as_deref() {
    install_review(&session, file, &files)?;
  }
  install_candidates(&session, &root, &files)?;
  Ok(Some(Built { session, meta }))
}

pub struct Report {
  pub body: String,
  pub outcome: Outcome,
}

/// Renders the report the review earned. `--json` and `--out` decide where it
/// goes; the outcome decides the exit code.
pub fn render_report(session: &Session, options: &Options) -> Result<Report> {
  let meta = session.meta()?;
  let hunks = session.hunks()?;
  let review = session.review()?;
  let state = session.state()?;
  let candidates = session.candidates()?;
  let input = ReportInput {
    meta: &meta,
    files: &hunks.files,
    review: review.as_ref(),
    state: &state,
    candidates: Some(&candidates),
  };

  let markdown = build_markdown(&input);
  if let Some(path) = options.out_file.as_deref() {
    std::fs::write(path, &markdown).with_context(|| format!("write {path}"))?;
  }
  let body = if options.as_json {
    format!("{}\n", serde_json::to_string_pretty(&build_json(&input))?)
  } else if options.out_file.is_some() {
    String::new()
  } else {
    markdown
  };
  Ok(Report { body, outcome: outcome_of(&state) })
}

/// Serves the review until the human submits, quits, or the timeout fires.
async fn host_review(
  state: Arc<AppState>,
  submitted: &mut mpsc::Receiver<()>,
  options: &Options,
  token: &str,
  listener: tokio::net::TcpListener,
) -> Result<Ending> {
  let port = listener.local_addr()?.port();
  let url = format!("http://127.0.0.1:{port}/?t={token}");

  let (shutdown_tx, shutdown_rx) = oneshot::channel();
  let mut served = tokio::spawn(serve(listener, state, async {
    let _ = shutdown_rx.await;
  }));

  eprintln!("review    {url}");
  if options.should_open {
    crate::browser::open(&url);
  }

  let ending = tokio::select! {
    ending = wait_for_ending(submitted, options.timeout_seconds) => ending,
    stopped = &mut served => {
      // The server ended on its own, which the wait cannot see. Report why.
      return match stopped {
        Ok(Err(error)) => Err(error),
        Ok(Ok(())) => Ok(Ending::ServerStopped),
        Err(error) => Err(error).context("server task"),
      };
    }
  };

  let _ = shutdown_tx.send(());
  // The shutdown waits for in-flight responses, so a submit finishes flushing
  // here. The bound is for a connection that never does — and a second Ctrl+C
  // means the human is done waiting for it, so it cuts the grace short.
  tokio::select! {
    _ = tokio::time::timeout(SHUTDOWN_GRACE, served) => {}
    _ = tokio::signal::ctrl_c(), if ending == Ending::Interrupt => {}
  }
  Ok(ending)
}

/// gix cannot open a SHA-256 repository yet, and the error it raises names a
/// config key rather than the limitation. Even once it can, `scope::EMPTY_TREE`
/// is the SHA-1 hash, so this stays a real gap and not just a bad message.
fn discover_repo() -> Result<gix::Repository> {
  gix::discover(std::env::current_dir()?).map_err(|error| {
    if error.to_string().contains("objectFormat=sha256") {
      anyhow::anyhow!("diffpane cannot read SHA-256 repositories yet: {error}")
    } else {
      error.into()
    }
  })
}

pub async fn run(options: &Options) -> Result<i32> {
  let repo = discover_repo()?;
  let Some(built) = build_session(&repo, options)? else {
    eprintln!("no changes to review");
    return Ok(0);
  };

  let token = generate_token();
  let (submit_tx, mut submit_rx) = mpsc::channel(1);
  // Bound before the state is built: the port names the asset cookie.
  let listener = bind(options.port).await?;
  let port = listener.local_addr()?.port();
  let state =
    Arc::new(AppState::new(built.session, token.clone(), Assets::from_env(), submit_tx, port));

  let Totals { files, additions, deletions } = built.meta.totals;
  eprintln!("diffpane  {files} files, +{additions}/-{deletions}");
  let ending = host_review(Arc::clone(&state), &mut submit_rx, options, &token, listener).await?;
  match ending {
    Ending::Timeout => eprintln!("timed out waiting for the review"),
    Ending::ServerStopped => eprintln!("the review server stopped early"),
    Ending::Submitted | Ending::Interrupt => {}
  }

  // A connection that outlived the shutdown grace is still writing
  // `comments.json`; the report waits for it rather than reading underneath it.
  let frozen = state.freeze();
  let report = render_report(state.session(), options)?;
  drop(frozen);
  print_report(&report.body);
  Ok(report.outcome.exit_code())
}

/// `diffpane --json | head` closes stdout early. A broken pipe is not a
/// failure: swallowing it keeps the real exit code, and exiting 0 here would
/// report a review as approved that nobody ever read.
pub fn print_report(body: &str) {
  let mut stdout = std::io::stdout();
  if let Err(error) = stdout.write_all(body.as_bytes()).and_then(|()| stdout.flush())
    && error.kind() != std::io::ErrorKind::BrokenPipe
  {
    eprintln!("diffpane: {error}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::{DiffLine, LineNote, LineType, NoteKind, Overall, Verdict};

  #[test]
  fn sums_the_totals_over_files() {
    let file = |additions, deletions| FileDiff {
      id: "f0".into(),
      path: "a.ts".into(),
      old_path: "a.ts".into(),
      status: crate::model::FileStatus::Modified,
      additions,
      deletions,
      binary: false,
      noise: false,
      language: None,
      truncated: false,
      hunks: Vec::new(),
    };
    let summed = totals(&[file(3, 1), file(0, 4)]);
    assert_eq!(summed.files, 2);
    assert_eq!(summed.additions, 3);
    assert_eq!(summed.deletions, 5);
  }

  fn one_hunk_file() -> FileDiff {
    let line = |i, kind, old, new| DiffLine { i, kind, old, new, text: String::new() };
    FileDiff {
      id: "f0".into(),
      path: "a.ts".into(),
      old_path: "a.ts".into(),
      status: crate::model::FileStatus::Modified,
      additions: 1,
      deletions: 1,
      binary: false,
      noise: false,
      language: None,
      truncated: false,
      hunks: vec![Hunk {
        id: "f0h0".into(),
        header: "@@ -7,1 +7,1 @@".into(),
        old_start: 7,
        old_count: 1,
        new_start: 7,
        new_count: 1,
        additions: 1,
        deletions: 1,
        lines: vec![line(0, LineType::Del, Some(7), None), line(1, LineType::Add, None, Some(7))],
      }],
    }
  }

  fn note(hunk: &str, side: Side, line: u32) -> LineNote {
    note_kind(hunk, side, line, NoteKind::Note)
  }

  fn note_kind(hunk: &str, side: Side, line: u32, kind: NoteKind) -> LineNote {
    LineNote { hunk: hunk.into(), side, line, body: "why".into(), kind }
  }

  fn review_with(line_notes: Vec<LineNote>) -> Review {
    Review { title: None, story: None, chapters: Vec::new(), file_notes: None, line_notes }
  }

  #[test]
  fn an_anchored_line_note_warns_about_nothing_and_survives() {
    let review = review_with(vec![note("f0h0", Side::New, 7), note("f0h0", Side::Old, 7)]);
    let mut warnings = Vec::new();
    let kept = valid_line_notes(&review, &[one_hunk_file()], &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(kept.len(), 2);
  }

  #[test]
  fn a_line_note_that_renders_nowhere_warns_and_is_dropped() {
    let review = review_with(vec![
      note("f9h9", Side::New, 7),
      note("f0h0", Side::New, 400),
      // The deletion is on the old side; asking for new line 7 finds the
      // addition, so the sides are not interchangeable.
      note("f0h0", Side::Old, 999),
    ]);
    let mut warnings = Vec::new();
    let kept = valid_line_notes(&review, &[one_hunk_file()], &mut warnings);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings[0].contains("unknown hunk f9h9"), "{warnings:?}");
    assert!(warnings[1].contains("new line 400"), "{warnings:?}");
    assert!(warnings[2].contains("old line 999"), "{warnings:?}");
  }

  #[test]
  fn a_duplicate_anchor_keeps_the_first_and_warns() {
    let review = review_with(vec![
      note_kind("f0h0", Side::New, 7, NoteKind::Flag),
      note_kind("f0h0", Side::New, 7, NoteKind::Note),
    ]);
    let mut warnings = Vec::new();
    let kept = valid_line_notes(&review, &[one_hunk_file()], &mut warnings);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].kind, NoteKind::Flag, "the first note on the anchor should survive");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("duplicate line note"), "{warnings:?}");
  }

  #[test]
  fn dates_the_slug_in_iso_order() {
    let stamp = today();
    assert_eq!(stamp.len(), 10, "{stamp}");
    assert_eq!(stamp.matches('-').count(), 2, "{stamp}");
  }

  fn seeded_session(state: &ReviewState) -> (tempfile::TempDir, Session) {
    let temp = tempfile::tempdir().unwrap();
    let session = Session::new(temp.path().to_path_buf());
    write_json(
      &session.meta_path(),
      &Meta {
        repo: "demo".into(),
        repo_root: "/tmp/demo".into(),
        slug: "demo".into(),
        title: "Demo".into(),
        scope: crate::model::Scope::Branch,
        base: "main".into(),
        head: "feature".into(),
        diff_cmd: "git diff main...HEAD".into(),
        generated_at: "2026-01-01T00:00:00Z".into(),
        totals: Totals::default(),
      },
    )
    .unwrap();
    write_json(&session.hunks_path(), &Hunks::default()).unwrap();
    write_json(&session.state_path(), state).unwrap();
    (temp, session)
  }

  #[test]
  fn a_rerun_without_review_drops_the_previous_narrative() {
    let state = ReviewState { submitted: true, ..ReviewState::default() };
    let (_temp, session) = seeded_session(&state);
    let stale = Review {
      title: None,
      story: None,
      chapters: Vec::new(),
      file_notes: None,
      line_notes: Vec::new(),
    };
    write_json(&session.review_path(), &stale).unwrap();

    write_json(
      &session.candidates_path(),
      &crate::model::Candidates {
        items: vec![crate::model::CommentCandidate {
          id: "cc-0000".into(),
          file: "a.ts".into(),
          anchor: "x".into(),
          proposed: None,
          rationale: "y".into(),
          location: None,
        }],
      },
    )
    .unwrap();

    clear_previous_run(&session).unwrap();

    assert!(!session.review_path().exists(), "stale chapters point at hunks this diff lost");
    assert!(
      !session.candidates_path().exists(),
      "a removed dev/comment-candidates.md should not resurrect"
    );
    assert!(!session.state().unwrap().submitted, "a previous submit is not this run's");
  }

  #[test]
  fn clearing_a_session_that_never_had_a_review_is_fine() {
    let (_temp, session) = seeded_session(&ReviewState::default());
    clear_previous_run(&session).unwrap();
    assert!(!session.review_path().exists());
  }

  #[test]
  fn an_abandoned_review_reports_as_abandoned() {
    let (_temp, session) = seeded_session(&ReviewState::default());
    let report = render_report(&session, &Options::default()).unwrap();
    assert_eq!(report.outcome, Outcome::Abandoned);
    assert!(report.body.contains("Demo"), "{}", report.body);
  }

  #[test]
  fn writing_to_a_file_keeps_stdout_empty() {
    let state = ReviewState {
      submitted: true,
      submitted_at: Some("2026-01-01T00:00:01Z".into()),
      overall: Overall { verdict: Some(Verdict::Fix), body: "one blocker".into() },
      ..ReviewState::default()
    };
    let (temp, session) = seeded_session(&state);
    let out = temp.path().join("report.md");
    let options = Options { out_file: Some(out.display().to_string()), ..Options::default() };

    let report = render_report(&session, &options).unwrap();
    assert_eq!(report.outcome, Outcome::ChangesRequested);
    assert!(report.body.is_empty(), "the report went to the file");
    assert!(std::fs::read_to_string(&out).unwrap().contains("one blocker"));
  }

  #[test]
  fn json_output_is_machine_readable() {
    let state = ReviewState {
      submitted: true,
      submitted_at: Some("2026-01-01T00:00:01Z".into()),
      ..ReviewState::default()
    };
    let (_temp, session) = seeded_session(&state);
    let options = Options { as_json: true, ..Options::default() };
    let report = render_report(&session, &options).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&report.body).unwrap();
    assert_eq!(parsed["outcome"], "approved");
    assert_eq!(report.outcome, Outcome::Approved);
  }
}

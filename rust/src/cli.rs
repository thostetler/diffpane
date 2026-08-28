//! The command's own logic, ported from `src/cli.ts`.
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
use crate::model::{FileDiff, Hunks, Meta, Review, ReviewState, Totals};
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

/// Warns about chapters that point at hunk ids this diff does not have. Not an
/// error: the narrative is the agent's, and a stale id costs the reader one
/// chapter, not the review.
fn install_review(session: &Session, file: &str, files: &[FileDiff]) -> Result<()> {
  let body = std::fs::read_to_string(file).with_context(|| format!("read {file}"))?;
  let review: Review = serde_json::from_str(&body).with_context(|| format!("parse {file}"))?;
  let known: std::collections::BTreeSet<&str> =
    files.iter().flat_map(|file| file.hunks.iter().map(|hunk| hunk.id.as_str())).collect();
  for chapter in &review.chapters {
    for id in &chapter.hunks {
      if !known.contains(id.as_str()) {
        eprintln!("warning: chapter {} references unknown hunk {id}", chapter.id);
      }
    }
  }
  write_json(&session.review_path(), &review)
}

pub struct Built {
  pub session: Session,
  pub meta: Meta,
}

/// Re-running on the same branch the same day reuses the session directory, so
/// both of the previous run's mutable artefacts have to go. Comments would
/// replay a stale `submitted: true` and anchor to hunk ids that have moved; a
/// review left behind by an earlier `--review` would let chapters claim hunk
/// ids this diff does not have and make `PUT /api/progress` validate against a
/// chapter set nobody asked for.
fn clear_previous_run(session: &Session) -> Result<()> {
  write_json(&session.state_path(), &ReviewState::default())?;
  match std::fs::remove_file(session.review_path()) {
    Ok(()) => Ok(()),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
    Err(error) => Err(error).context(format!("remove {}", session.review_path().display())),
  }
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
  let slug = format!("{}-{}", today(), slugify(title.as_deref().unwrap_or(&resolved.head)));
  let session = Session::create(&root, &slug)?;
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
  let input =
    ReportInput { meta: &meta, files: &hunks.files, review: review.as_ref(), state: &state };

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
) -> Result<Ending> {
  let listener = bind(options.port).await?;
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
        Ok(Ok(())) => Ok(Ending::Interrupt),
        Err(error) => Err(error).context("server task"),
      };
    }
  };

  let _ = shutdown_tx.send(());
  // The shutdown waits for in-flight responses, so a submit finishes flushing
  // here. The bound is for a connection that never does (item 21).
  let _ = tokio::time::timeout(SHUTDOWN_GRACE, served).await;
  Ok(ending)
}

pub async fn run(options: &Options) -> Result<i32> {
  let repo = gix::discover(std::env::current_dir()?)?;
  let Some(built) = build_session(&repo, options)? else {
    eprintln!("no changes to review");
    return Ok(0);
  };

  let token = generate_token();
  let (submit_tx, mut submit_rx) = mpsc::channel(1);
  let state = Arc::new(AppState::new(built.session, token.clone(), Assets::from_env(), submit_tx));

  let Totals { files, additions, deletions } = built.meta.totals;
  eprintln!("diffpane  {files} files, +{additions}/-{deletions}");
  host_review(Arc::clone(&state), &mut submit_rx, options, &token).await?;

  let report = render_report(state.session(), options)?;
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
  use crate::model::{Overall, Verdict};

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
    let stale = Review { title: None, story: None, chapters: Vec::new(), file_notes: None };
    write_json(&session.review_path(), &stale).unwrap();

    clear_previous_run(&session).unwrap();

    assert!(!session.review_path().exists(), "stale chapters point at hunks this diff lost");
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

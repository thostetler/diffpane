//! CLI scope to a diff plan, plus the labels `meta.json` carries.
//!
//! The plan mirrors what the TypeScript handed to `git diff`, so `diff_cmd`
//! stays the command a user could paste into a shell — not the resolved object
//! ids the port actually diffs.

use anyhow::{Context, Result, bail};
use gix::remote::Direction;

use crate::diff::Plan;
use crate::model::Scope;

/// git's canonical empty tree, used as the base for a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const BASE_CANDIDATES: [&str; 5] =
  ["origin/HEAD", "origin/main", "origin/master", "main", "master"];

/// The scope the argument parser selected, unresolved.
#[derive(Debug, Default)]
pub struct Request {
  pub scope: Scope,
  pub base: Option<String>,
  pub range: Option<String>,
  pub commit: Option<String>,
  pub paths: Vec<String>,
}

pub struct Resolved {
  pub scope: Scope,
  pub plan: Plan,
  pub paths: Vec<String>,
  /// What the review is measured against, for display.
  pub base: String,
  pub head: String,
  pub diff_cmd: String,
}

/// The two sides of an explicit range. `a...b` is git's symmetric form, which
/// diffs the merge base against `b`.
#[derive(Debug, PartialEq, Eq)]
struct Range<'a> {
  left: &'a str,
  right: &'a str,
  symmetric: bool,
}

/// `git diff <rev>` means rev-versus-worktree, which no plan here can express.
/// The TypeScript passed the argument through to git and silently got that
/// behaviour; rejecting it is the honest option.
fn parse_range(spec: &str) -> Result<Range<'_>> {
  let (left, right, symmetric) = match spec.split_once("...") {
    Some((left, right)) => (left, right, true),
    None => {
      let (left, right) = spec
        .split_once("..")
        .with_context(|| format!("--range needs a..b or a...b, got {spec}"))?;
      (left, right, false)
    }
  };
  if right.contains("..") {
    bail!("--range takes one range, got {spec}");
  }
  // git fills an omitted side with HEAD.
  let left = if left.is_empty() { "HEAD" } else { left };
  let right = if right.is_empty() { "HEAD" } else { right };
  Ok(Range { left, right, symmetric })
}

fn commit_id(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId> {
  Ok(repo.rev_parse_single(rev)?.object()?.peel_to_commit()?.id)
}

fn merge_base(repo: &gix::Repository, left: &str, right: &str) -> Result<String> {
  let base = repo.merge_base(commit_id(repo, left)?, commit_id(repo, right)?)?;
  Ok(base.detach().to_string())
}

fn head_label(repo: &gix::Repository) -> String {
  match repo.head_name() {
    Ok(Some(name)) => name.shorten().to_string(),
    _ => "HEAD".to_owned(),
  }
}

/// `origin/HEAD` is a symref: report what it points at, the way
/// `git rev-parse --abbrev-ref` does, so the review says `origin/main`.
fn resolve_symbolic(repo: &gix::Repository, candidate: &str) -> String {
  let Ok(reference) = repo.find_reference(candidate) else { return candidate.to_owned() };
  match reference.target() {
    gix::refs::TargetRef::Symbolic(name) => name.shorten().to_string(),
    gix::refs::TargetRef::Object(_) => candidate.to_owned(),
  }
}

fn default_base(repo: &gix::Repository) -> Result<String> {
  if let Ok(Some(head)) = repo.head_name()
    && let Some(Ok(tracking)) =
      repo.branch_remote_tracking_ref_name(head.as_ref(), Direction::Fetch)
  {
    return Ok(tracking.shorten().to_string());
  }
  for candidate in BASE_CANDIDATES {
    if repo.rev_parse_single(candidate).is_ok() {
      return Ok(resolve_symbolic(repo, candidate));
    }
  }
  bail!("could not infer a base ref; pass --base")
}

fn resolve_plan(repo: &gix::Repository, request: &Request) -> Result<(Plan, String, Vec<String>)> {
  match request.scope {
    Scope::Working => Ok((Plan::Working, "working tree".to_owned(), Vec::new())),
    Scope::Staged => Ok((Plan::Staged, "index".to_owned(), vec!["--cached".to_owned()])),
    Scope::Range => {
      let spec = request.range.as_deref().context("--range needs a value")?;
      let range = parse_range(spec)?;
      let base = if range.symmetric {
        merge_base(repo, range.left, range.right)?
      } else {
        range.left.to_owned()
      };
      let plan = Plan::Trees { base, head: range.right.to_owned() };
      Ok((plan, spec.to_owned(), vec![spec.to_owned()]))
    }
    Scope::Commit => {
      let commit = request.commit.as_deref().context("--commit needs a value")?;
      // `<sha>^!` on a merge produces a combined diff no unified-diff reader can
      // take, and is empty for a clean merge. An explicit first-parent range is
      // an ordinary diff in both cases.
      let parent = commit_id(repo, &format!("{commit}^"))
        .map_or_else(|_| EMPTY_TREE.to_owned(), |id| id.to_string());
      let plan = Plan::Trees { base: parent.clone(), head: commit.to_owned() };
      Ok((plan, commit.to_owned(), vec![parent, commit.to_owned()]))
    }
    Scope::Branch => {
      let base = match request.base.clone() {
        Some(base) => base,
        None => default_base(repo)?,
      };
      let plan = Plan::Trees { base: merge_base(repo, &base, "HEAD")?, head: "HEAD".to_owned() };
      Ok((plan, base.clone(), vec![format!("{base}...HEAD")]))
    }
  }
}

pub fn resolve(repo: &gix::Repository, request: &Request) -> Result<Resolved> {
  let (plan, base, mut args) = resolve_plan(repo, request)?;
  if !request.paths.is_empty() {
    args.push("--".to_owned());
    args.extend(request.paths.iter().cloned());
  }
  Ok(Resolved {
    scope: request.scope,
    plan,
    paths: request.paths.clone(),
    base,
    head: head_label(repo),
    diff_cmd: format!("git diff {}", args.join(" ")).trim_end().to_owned(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn splits_both_range_forms() {
    assert_eq!(parse_range("a..b").unwrap(), Range { left: "a", right: "b", symmetric: false });
    assert_eq!(parse_range("a...b").unwrap(), Range { left: "a", right: "b", symmetric: true });
  }

  #[test]
  fn fills_an_omitted_side_with_head() {
    assert_eq!(parse_range("..b").unwrap(), Range { left: "HEAD", right: "b", symmetric: false });
    assert_eq!(parse_range("a...").unwrap(), Range { left: "a", right: "HEAD", symmetric: true });
  }

  #[test]
  fn rejects_a_bare_rev() {
    assert!(parse_range("HEAD").is_err());
    assert!(parse_range("a..b..c").is_err());
  }
}

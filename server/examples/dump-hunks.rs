//! Parity harness driver: print `hunks.json` for a scope and exit.
//!
//!   dump-hunks <repo> [--working|--staged|--range <a..b>|--commit <sha>
//!                      |--base <ref>] [-- <pathspec>...]
//!
//! The flags mirror the CLI's so the harness exercises `scope` as well as the
//! diff layer. Temporary: whether this becomes a real `--dump-hunks` flag is an
//! open decision — the skill currently counts hunk ids out of the diff by hand.

use anyhow::{Context, Result, bail};
use diffpane::model::{Hunks, Scope};
use diffpane::{diff, scope};

const USAGE: &str = "usage: dump-hunks <repo> [scope flags] [-- <pathspec>...]";

fn parse(args: &mut impl Iterator<Item = String>) -> Result<scope::Request> {
  let mut request = scope::Request::default();
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--working" => request.scope = Scope::Working,
      "--staged" => request.scope = Scope::Staged,
      "--base" => request.base = Some(args.next().context("--base needs a value")?),
      "--range" => {
        request.scope = Scope::Range;
        request.range = Some(args.next().context("--range needs a value")?);
      }
      "--commit" => {
        request.scope = Scope::Commit;
        request.commit = Some(args.next().context("--commit needs a value")?);
      }
      "--" => request.paths = args.by_ref().collect(),
      other => bail!("unknown argument: {other}\n{USAGE}"),
    }
  }
  Ok(request)
}

fn main() -> Result<()> {
  let mut args = std::env::args().skip(1);
  let root = args.next().context(USAGE)?;
  let request = parse(&mut args)?;

  let repo = gix::discover(root)?;
  let resolved = scope::resolve(&repo, &request)?;
  let files = diff::files(&repo, &resolved.plan, &resolved.paths)?;
  println!("{}", serde_json::to_string_pretty(&Hunks { files })?);
  Ok(())
}

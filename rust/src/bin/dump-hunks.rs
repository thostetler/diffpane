//! Parity harness driver: print `hunks.json` for a scope and exit.
//!
//!   dump-hunks <repo> working
//!   dump-hunks <repo> tree <base> <head>
//!
//! Temporary. Whether this becomes a real `--dump-hunks` flag on the CLI is an
//! open decision — the skill currently counts hunk ids out of the diff by hand.

use anyhow::{Result, bail};
use diffpane::diff::{self, Plan};
use diffpane::model::Hunks;

fn main() -> Result<()> {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let [root, mode, rest @ ..] = args.as_slice() else {
    bail!("usage: dump-hunks <repo> <working|tree> [base head]");
  };

  let plan = match mode.as_str() {
    "working" => Plan::Working,
    "tree" => {
      let [base, head] = rest else { bail!("tree mode needs <base> <head>") };
      Plan::Trees { base: base.clone(), head: head.clone() }
    }
    other => bail!("unknown mode: {other}"),
  };

  let repo = gix::discover(root)?;
  let files = diff::files(&repo, &plan)?;
  println!("{}", serde_json::to_string_pretty(&Hunks { files })?);
  Ok(())
}

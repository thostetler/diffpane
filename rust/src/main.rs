//! The `diffpane` binary: argument dispatch and exit codes, nothing else.

use std::process::ExitCode;

use diffpane::args::{self, Parsed, USAGE};
use diffpane::cli::{print_report, run};
use diffpane::skill::{default_skill_dir, install};

/// Reserved for an error that stopped the review before a verdict existed;
/// `docs/contract.md` lists 0/1/2 for the review's own outcomes.
const EXIT_ERROR: u8 = 3;

async fn dispatch() -> anyhow::Result<i32> {
  match args::parse(std::env::args().skip(1).collect())? {
    Parsed::Help => {
      print_report(USAGE);
      Ok(0)
    }
    Parsed::Version => {
      print_report(&format!("{}\n", env!("CARGO_PKG_VERSION")));
      Ok(0)
    }
    Parsed::InstallSkill { skill_dir } => {
      let dir = skill_dir.map_or_else(default_skill_dir, std::path::PathBuf::from);
      let installed = install(&dir)?;
      let verb = if installed.replaced { "replaced" } else { "wrote" };
      print_report(&format!(
        "{verb} {}\nrestart Claude Code, then run /diffpane\n",
        installed.path.display()
      ));
      Ok(0)
    }
    Parsed::Options(options) => run(&options).await,
  }
}

#[tokio::main]
async fn main() -> ExitCode {
  match dispatch().await {
    Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(EXIT_ERROR)),
    Err(error) => {
      eprintln!("diffpane: {error:#}");
      ExitCode::from(EXIT_ERROR)
    }
  }
}

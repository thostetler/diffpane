//! Argument parsing.
//!
//! Hand-rolled rather than clap: `USAGE` is user-facing text that the skill
//! quotes, and `--help` output is part of the frozen CLI surface, so it is
//! spelled out here instead of generated.

use anyhow::{Result, bail};

use crate::model::Scope;

pub const DEFAULT_PORT: u16 = 7777;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 3600;

pub const USAGE: &str = r"diffpane — review a git diff in your browser, then hand the feedback back.

Usage
  diffpane [options] [-- <pathspec>...]

Scope (default: current branch vs its base)
  --base <ref>       base ref for the branch diff
  --working          uncommitted changes
  --staged           staged changes
  --range <a..b>     an explicit range
  --commit <sha>     a single commit

Options
  --title <text>     human title for the review
  --review <file>    narrative JSON (chapters + descriptions) to render
  --out <file>       write the markdown report to a file
  --json             print machine-readable feedback to stdout
  --port <n>         preferred port (default 7777, walks forward if taken)
  --no-open          do not open a browser
  --timeout <sec>    give up waiting after N seconds (default 3600, 0 = never)
  -h, --help         show this
  -v, --version      show version

Agent setup
  --install-skill    install the Claude Code skill, then exit
  --skill-dir <dir>  where to install it (default ~/.claude/skills)

Exit codes
  0  approved (or nothing to review)   2  abandoned / timed out
  1  changes requested                 3  error
";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
  pub scope: Scope,
  pub base: Option<String>,
  pub range: Option<String>,
  pub commit: Option<String>,
  pub paths: Vec<String>,
  pub title: Option<String>,
  pub review_file: Option<String>,
  pub out_file: Option<String>,
  pub port: u16,
  pub timeout_seconds: u64,
  pub as_json: bool,
  pub should_open: bool,
}

impl Default for Options {
  fn default() -> Self {
    Self {
      scope: Scope::Branch,
      base: None,
      range: None,
      commit: None,
      paths: Vec::new(),
      title: None,
      review_file: None,
      out_file: None,
      port: DEFAULT_PORT,
      timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
      as_json: false,
      should_open: true,
    }
  }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
  Options(Box<Options>),
  InstallSkill { skill_dir: Option<String> },
  Help,
  Version,
}

/// Which of the mutually exclusive scope flags were given, for the error.
#[derive(Default)]
struct Selection {
  working: bool,
  staged: bool,
  range: bool,
  commit: bool,
}

impl Selection {
  fn named(&self) -> Vec<&'static str> {
    [
      ("working", self.working),
      ("staged", self.staged),
      ("range", self.range),
      ("commit", self.commit),
    ]
    .into_iter()
    .filter_map(|(name, given)| given.then_some(name))
    .collect()
  }

  fn scope(&self) -> Scope {
    // The order matches the TypeScript's, which only matters while exactly one
    // flag is set — more than one is an error before this is consulted.
    if self.range {
      Scope::Range
    } else if self.commit {
      Scope::Commit
    } else if self.working {
      Scope::Working
    } else if self.staged {
      Scope::Staged
    } else {
      Scope::Branch
    }
  }
}

struct Parser {
  args: std::vec::IntoIter<String>,
  /// Everything after a bare `--`, taken verbatim.
  rest: Vec<String>,
}

impl Parser {
  fn value(&mut self, flag: &str, inline: Option<String>) -> Result<String> {
    match inline.or_else(|| self.args.next()) {
      Some(value) => Ok(value),
      None => bail!("--{flag} needs a value"),
    }
  }
}

fn positive_int(value: &str, label: &str) -> Result<u64> {
  match value.parse::<f64>() {
    Ok(parsed) if parsed.is_finite() && parsed >= 0.0 => Ok(parsed.trunc() as u64),
    _ => bail!("--{label} must be a number >= 0"),
  }
}

pub fn parse(argv: Vec<String>) -> Result<Parsed> {
  let mut options = Options::default();
  let mut selection = Selection::default();
  let mut skill_dir: Option<String> = None;
  let mut install_skill = false;
  let mut port: Option<String> = None;
  let mut timeout: Option<String> = None;
  let mut parser = Parser { args: argv.into_iter(), rest: Vec::new() };

  while let Some(arg) = parser.args.next() {
    // `--` itself is the separator, never a flag with an inline value: reading
    // `--=x` as one dropped the `x` instead of taking it as a pathspec.
    let (flag, inline) = match arg.split_once('=') {
      Some((flag, value)) if flag.starts_with("--") && flag.len() > 2 => {
        (flag.to_owned(), Some(value.to_owned()))
      }
      _ => (arg.clone(), None),
    };
    match flag.as_str() {
      "-h" | "--help" => return Ok(Parsed::Help),
      "-v" | "--version" => return Ok(Parsed::Version),
      "--install-skill" => install_skill = true,
      "--skill-dir" => skill_dir = Some(parser.value("skill-dir", inline)?),
      "--working" => selection.working = true,
      "--staged" => selection.staged = true,
      "--json" => options.as_json = true,
      "--no-open" => options.should_open = false,
      "--base" => options.base = Some(parser.value("base", inline)?),
      "--title" => options.title = Some(parser.value("title", inline)?),
      "--review" => options.review_file = Some(parser.value("review", inline)?),
      "--out" => options.out_file = Some(parser.value("out", inline)?),
      "--port" => port = Some(parser.value("port", inline)?),
      "--timeout" => timeout = Some(parser.value("timeout", inline)?),
      "--range" => {
        selection.range = true;
        options.range = Some(parser.value("range", inline)?);
      }
      "--commit" => {
        selection.commit = true;
        options.commit = Some(parser.value("commit", inline)?);
      }
      // Extend, not replace: `diffpane a.ts -- b.ts` scopes to both, the way
      // node's parseArgs collected positionals from either side.
      "--" => parser.rest.extend(parser.args.by_ref()),
      other if other.starts_with('-') => bail!("unknown option: {other}"),
      other => parser.rest.push(other.to_owned()),
    }
  }

  let selected = selection.named();
  if install_skill {
    // It installs a file and exits, so every other flag was a misunderstanding
    // about what the command was going to do. Saying so beats obeying half of
    // it in silence.
    if options != Options::default()
      || port.is_some()
      || timeout.is_some()
      || !parser.rest.is_empty()
      || !selected.is_empty()
    {
      bail!("--install-skill takes no options but --skill-dir");
    }
    return Ok(Parsed::InstallSkill { skill_dir });
  }

  if selected.len() > 1 {
    let names: Vec<String> = selected.iter().map(|name| format!("--{name}")).collect();
    bail!("pick one scope, not {}", names.join(" and "));
  }

  options.scope = selection.scope();
  options.paths = parser.rest;
  if let Some(value) = port {
    let parsed = positive_int(&value, "port")?;
    if parsed > u64::from(u16::MAX) {
      bail!("--port must be a number between 0 and {}", u16::MAX);
    }
    options.port = parsed as u16;
  }
  if let Some(value) = timeout {
    options.timeout_seconds = positive_int(&value, "timeout")?;
  }
  Ok(Parsed::Options(Box::new(options)))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse_args(args: &[&str]) -> Result<Parsed> {
    parse(args.iter().map(|arg| (*arg).to_owned()).collect())
  }

  fn options(args: &[&str]) -> Options {
    match parse_args(args).unwrap() {
      Parsed::Options(options) => *options,
      other => panic!("expected options, got {other:?}"),
    }
  }

  #[test]
  fn defaults_to_the_branch_scope() {
    let parsed = options(&[]);
    assert_eq!(parsed.scope, Scope::Branch);
    assert_eq!(parsed.port, DEFAULT_PORT);
    assert_eq!(parsed.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    assert!(parsed.should_open);
    assert!(!parsed.as_json);
  }

  #[test]
  fn selects_each_scope() {
    assert_eq!(options(&["--working"]).scope, Scope::Working);
    assert_eq!(options(&["--staged"]).scope, Scope::Staged);
    assert_eq!(options(&["--range", "a..b"]).scope, Scope::Range);
    assert_eq!(options(&["--commit", "abc123"]).scope, Scope::Commit);
    assert_eq!(options(&["--base", "origin/main"]).scope, Scope::Branch);
  }

  #[test]
  fn refuses_two_scopes() {
    let error = parse_args(&["--working", "--staged"]).unwrap_err().to_string();
    assert_eq!(error, "pick one scope, not --working and --staged");
  }

  #[test]
  fn reads_values_inline_or_separated() {
    assert_eq!(options(&["--title=Fix the parser"]).title.as_deref(), Some("Fix the parser"));
    assert_eq!(options(&["--title", "Fix the parser"]).title.as_deref(), Some("Fix the parser"));
    assert_eq!(options(&["--port=9000"]).port, 9000);
  }

  #[test]
  fn collects_pathspecs_before_and_after_the_separator() {
    // `diffpane src/a.ts` and `diffpane -- src/a.ts` both scope to a path.
    assert_eq!(options(&["src/a.ts"]).paths, ["src/a.ts"]);
    assert_eq!(options(&["--", "src/a.ts", "ui/"]).paths, ["src/a.ts", "ui/"]);
    // Both sides count. Dropping the left side reviewed a narrower diff than
    // asked for, silently.
    assert_eq!(options(&["src/a.ts", "--", "ui/"]).paths, ["src/a.ts", "ui/"]);
    // Past `--`, something that looks like a flag is a pathspec.
    assert_eq!(options(&["--", "--weird-file"]).paths, ["--weird-file"]);
  }

  #[test]
  fn rejects_bad_numbers() {
    assert!(parse_args(&["--port", "nope"]).is_err());
    assert!(parse_args(&["--timeout", "-1"]).is_err());
    assert!(parse_args(&["--port", "70000"]).is_err());
    assert_eq!(options(&["--timeout", "0"]).timeout_seconds, 0);
    assert_eq!(options(&["--timeout", "1.9"]).timeout_seconds, 1);
  }

  #[test]
  fn rejects_unknown_options_and_missing_values() {
    assert!(parse_args(&["--nonsense"]).is_err());
    assert!(parse_args(&["--base"]).is_err());
  }

  #[test]
  fn a_bare_double_dash_never_carries_an_inline_value() {
    // `--=x` split into the `--` separator and silently dropped the `x`.
    assert_eq!(parse_args(&["--=x"]).unwrap_err().to_string(), "unknown option: --=x");
    assert_eq!(options(&["--", "x"]).paths, ["x"]);
  }

  #[test]
  fn install_skill_refuses_the_options_it_would_ignore() {
    let error = parse_args(&["--install-skill", "--working"]).unwrap_err().to_string();
    assert_eq!(error, "--install-skill takes no options but --skill-dir");
    assert!(parse_args(&["--install-skill", "--port", "9000"]).is_err());
    assert!(parse_args(&["--install-skill", "src/a.ts"]).is_err());
  }

  #[test]
  fn help_and_version_win_over_everything_else() {
    assert_eq!(parse_args(&["--working", "-h"]).unwrap(), Parsed::Help);
    assert_eq!(parse_args(&["--version"]).unwrap(), Parsed::Version);
  }

  #[test]
  fn install_skill_takes_an_optional_directory() {
    assert_eq!(parse_args(&["--install-skill"]).unwrap(), Parsed::InstallSkill { skill_dir: None });
    assert_eq!(
      parse_args(&["--install-skill", "--skill-dir", "/tmp/skills"]).unwrap(),
      Parsed::InstallSkill { skill_dir: Some("/tmp/skills".to_owned()) }
    );
  }

  #[test]
  fn documents_every_flag_it_parses() {
    // The usage text is the contract; a flag missing from it is a bug.
    for flag in [
      "--base",
      "--working",
      "--staged",
      "--range",
      "--commit",
      "--title",
      "--review",
      "--out",
      "--json",
      "--port",
      "--no-open",
      "--timeout",
      "--install-skill",
      "--skill-dir",
    ] {
      assert!(USAGE.contains(flag), "{flag} is undocumented");
    }
  }
}

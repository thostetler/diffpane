//! Installing the Claude Code skill, ported from `src/install-skill.ts`.
//!
//! The skill is compiled in rather than read from a sibling directory: the
//! Rust build ships as a single binary, so a file lookup relative to the
//! executable would be one more thing to get wrong on a user's machine.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const SKILL: &str = include_str!("../../skills/diffpane/SKILL.md");

pub fn default_skill_dir() -> PathBuf {
  let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
  Path::new(&home).join(".claude").join("skills")
}

pub struct Installed {
  pub path: PathBuf,
  pub replaced: bool,
}

/// Overwrites by design: the skill is versioned with the binary that writes it,
/// so a stale copy is the failure mode worth preventing. Reports which happened
/// so a hand-edited skill does not vanish without a word.
pub fn install(skill_dir: &Path) -> Result<Installed> {
  let target = skill_dir.join("diffpane");
  let path = target.join("SKILL.md");
  let replaced = path.exists();
  fs::create_dir_all(&target).with_context(|| format!("create {}", target.display()))?;
  fs::write(&path, SKILL).with_context(|| format!("write {}", path.display()))?;
  Ok(Installed { path, replaced })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_packaged_skill_is_a_real_loadable_skill_file() {
    assert!(SKILL.starts_with("---\n"), "skill needs YAML frontmatter");
    assert!(SKILL.lines().any(|line| line == "name: diffpane"));
    assert!(SKILL.lines().any(|line| line == "user-invocable: true"));
    let description = SKILL
      .lines()
      .find(|line| line.starts_with("description: "))
      .expect("skill needs a description");
    assert!(description.len() > 52, "description carries the trigger phrasing");
  }

  #[test]
  fn installing_writes_where_claude_code_looks() {
    let temp = tempfile::tempdir().unwrap();
    let first = install(temp.path()).unwrap();
    assert_eq!(first.path, temp.path().join("diffpane").join("SKILL.md"));
    assert!(!first.replaced);
    assert_eq!(fs::read_to_string(&first.path).unwrap(), SKILL);
  }

  #[test]
  fn reinstalling_reports_that_it_replaced_a_previous_copy() {
    let temp = tempfile::tempdir().unwrap();
    install(temp.path()).unwrap();
    fs::write(temp.path().join("diffpane").join("SKILL.md"), "hand-edited\n").unwrap();

    let second = install(temp.path()).unwrap();
    assert!(second.replaced, "silently clobbering a local edit is not acceptable");
    assert_eq!(fs::read_to_string(&second.path).unwrap(), SKILL);
  }

  #[test]
  fn defaults_under_the_home_directory() {
    assert!(default_skill_dir().ends_with(".claude/skills"));
  }
}

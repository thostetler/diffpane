//! Parity spike: produce diffpane's `hunks.json` from `gix` instead of from
//! git's patch text, so the two can be diffed against each other.
//!
//!   spike <repo> working
//!   spike <repo> tree <base> <head>

use gix::bstr::{BStr, BString, ByteSlice};
use gix::diff::blob::pipeline::{Mode, WorktreeRoots};
use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader};
use gix::diff::blob::{Diff, ResourceKind, UnifiedDiff};
use gix::object::tree::EntryKind;
use serde::Serialize;

const MAX_LINES_PER_FILE: usize = 2000;
const MAX_LINES_PER_NOISE_FILE: usize = 40;

#[derive(Serialize)]
struct Line {
  i: usize,
  #[serde(rename = "type")]
  kind: &'static str,
  old: Option<u32>,
  new: Option<u32>,
  text: String,
}

#[derive(Serialize)]
struct Hunk {
  id: String,
  header: String,
  old_start: u32,
  old_count: u32,
  new_start: u32,
  new_count: u32,
  additions: usize,
  deletions: usize,
  lines: Vec<Line>,
}

#[derive(Serialize)]
struct FileDiff {
  id: String,
  path: String,
  old_path: String,
  status: &'static str,
  additions: usize,
  deletions: usize,
  binary: bool,
  noise: bool,
  language: Option<&'static str>,
  truncated: bool,
  hunks: Vec<Hunk>,
}

#[derive(Serialize)]
struct Output {
  files: Vec<FileDiff>,
}

/// One changed file, before its content has been diffed.
struct Change {
  path: BString,
  old_path: Option<BString>,
  status: &'static str,
  old: Option<(gix::ObjectId, EntryKind)>,
  new: Option<(gix::ObjectId, EntryKind)>,
}

/// Collects `(header, lines)` callbacks into diffpane's hunk shape.
struct Collector {
  cap: usize,
  used: usize,
  truncated: bool,
  hunks: Vec<Hunk>,
}

impl ConsumeHunk for Collector {
  type Out = (Vec<Hunk>, bool);

  fn consume_hunk(&mut self, header: HunkHeader, lines: &[(DiffLineKind, &[u8])]) -> std::io::Result<()> {
    if self.used >= self.cap {
      self.truncated = true;
      return Ok(());
    }
    let mut old_no = header.before_hunk_start;
    let mut new_no = header.after_hunk_start;
    let (mut additions, mut deletions) = (0, 0);
    let mut out = Vec::with_capacity(lines.len());
    for &(kind, content) in lines {
      if self.used >= self.cap {
        self.truncated = true;
        break;
      }
      let text = strip_terminator(content).to_str_lossy().into_owned();
      let i = out.len();
      out.push(match kind {
        DiffLineKind::Context => {
          let line = Line { i, kind: "context", old: Some(old_no), new: Some(new_no), text };
          old_no += 1;
          new_no += 1;
          line
        }
        DiffLineKind::Add => {
          additions += 1;
          let line = Line { i, kind: "add", old: None, new: Some(new_no), text };
          new_no += 1;
          line
        }
        DiffLineKind::Remove => {
          deletions += 1;
          let line = Line { i, kind: "del", old: Some(old_no), new: None, text };
          old_no += 1;
          line
        }
      });
      self.used += 1;
    }
    self.hunks.push(Hunk {
      id: String::new(),
      header: render_header(header),
      old_start: start_of(header.before_hunk_start, header.before_hunk_len),
      old_count: header.before_hunk_len,
      new_start: start_of(header.after_hunk_start, header.after_hunk_len),
      new_count: header.after_hunk_len,
      additions,
      deletions,
      lines: out,
    });
    Ok(())
  }

  fn finish(self) -> Self::Out {
    (self.hunks, self.truncated)
  }
}

fn strip_terminator(line: &[u8]) -> &[u8] {
  let line = line.strip_suffix(b"\n").unwrap_or(line);
  line.strip_suffix(b"\r").unwrap_or(line)
}

/// An empty side is reported at the line *before* the hunk, so a wholly deleted
/// file is `@@ -1,6 +0,0 @@`. gix reports the start as 1 either way.
fn start_of(start: u32, len: u32) -> u32 {
  if len == 0 { start.saturating_sub(1) } else { start }
}

/// git omits the count when it is 1.
fn render_header(header: HunkHeader) -> String {
  fn part(start: u32, len: u32) -> String {
    let start = start_of(start, len);
    if len == 1 { format!("{start}") } else { format!("{start},{len}") }
  }
  format!(
    "@@ -{} +{} @@",
    part(header.before_hunk_start, header.before_hunk_len),
    part(header.after_hunk_start, header.after_hunk_len)
  )
}

/// git's default `xfuncname`: the nearest preceding line starting with an ASCII
/// letter, `_` or `$`, clamped to 80 bytes and then right-trimmed. Language
/// drivers selected via a `diff=<lang>` gitattribute are not implemented.
fn func_context(before: &[&[u8]], hunk_start: u32) -> Option<String> {
  const FUNCNAME_MAX: usize = 80;
  let mut index = i64::from(hunk_start) - 2;
  while index >= 0 {
    let line = before[index as usize];
    match line.first() {
      Some(byte) if byte.is_ascii_alphabetic() || *byte == b'_' || *byte == b'$' => {
        let clamped = &line[..line.len().min(FUNCNAME_MAX)];
        let end = clamped.iter().rposition(|b| !b.is_ascii_whitespace() && *b != 0x0b)?;
        return Some(String::from_utf8_lossy(&clamped[..=end]).into_owned());
      }
      _ => index -= 1,
    }
  }
  None
}

fn is_noise(path: &str) -> bool {
  const LOCKS: [&str; 11] = [
    "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "npm-shrinkwrap.json", "poetry.lock",
    "Pipfile.lock", "Cargo.lock", "composer.lock", "go.sum", "uv.lock", "pnpm-lock.yml",
  ];
  const DIRS: [&str; 9] =
    ["node_modules", "vendor", "dist", "build", "out", "coverage", ".next", "__pycache__", "__snapshots__"];
  let name = path.rsplit('/').next().unwrap_or(path);
  if LOCKS.contains(&name) {
    return true;
  }
  if path.split('/').rev().skip(1).any(|segment| DIRS.contains(&segment)) {
    return true;
  }
  name.ends_with(".snap")
    || name.ends_with(".map")
    || name.ends_with(".lock")
    || name.ends_with(".pb.go")
    || [".min.js", ".min.css", ".min.mjs"].iter().any(|suffix| name.ends_with(suffix))
    || name.split('.').nth_back(1) == Some("generated")
}

fn language_of(path: &str) -> Option<&'static str> {
  const LANGUAGES: [(&str, &str); 52] = [
    ("ts", "typescript"), ("tsx", "tsx"), ("mts", "typescript"), ("cts", "typescript"),
    ("js", "javascript"), ("jsx", "jsx"), ("mjs", "javascript"), ("cjs", "javascript"),
    ("py", "python"), ("rb", "ruby"), ("go", "go"), ("rs", "rust"), ("java", "java"),
    ("kt", "kotlin"), ("swift", "swift"), ("c", "c"), ("h", "c"), ("cc", "cpp"),
    ("cpp", "cpp"), ("hpp", "cpp"), ("cs", "csharp"), ("php", "php"), ("sh", "shell"),
    ("bash", "shell"), ("zsh", "shell"), ("fish", "shell"), ("sql", "sql"),
    ("css", "css"), ("scss", "scss"), ("less", "less"), ("html", "html"),
    ("vue", "vue"), ("svelte", "svelte"), ("json", "json"), ("jsonc", "json"),
    ("yml", "yaml"), ("yaml", "yaml"), ("toml", "toml"), ("ini", "ini"),
    ("xml", "xml"), ("md", "markdown"), ("mdx", "mdx"), ("rst", "rst"),
    ("graphql", "graphql"), ("gql", "graphql"), ("proto", "protobuf"),
    ("tf", "terraform"), ("lua", "lua"), ("vim", "vim"), ("mp", "mp"), ("np", "np"), ("qp", "qp"),
  ];
  let name = path.rsplit('/').next().unwrap_or(path);
  if name.to_ascii_lowercase().starts_with("dockerfile") {
    return Some("dockerfile");
  }
  if !name.contains('.') {
    return None;
  }
  let extension = name.rsplit('.').next()?.to_ascii_lowercase();
  LANGUAGES.iter().find(|(key, _)| *key == extension).map(|(_, value)| *value)
}

fn entry_kind(mode: gix::index::entry::Mode) -> EntryKind {
  use gix::index::entry::Mode;
  match mode {
    Mode::SYMLINK => EntryKind::Link,
    m if m.contains(Mode::FILE_EXECUTABLE) => EntryKind::BlobExecutable,
    _ => EntryKind::Blob,
  }
}

/// Index versus worktree, i.e. bare `git diff`.
fn working_changes(repo: &gix::Repository) -> anyhow::Result<Vec<Change>> {
  use gix::status::index_worktree::Item;
  use gix::status::plumbing::index_as_worktree::{Change as WtChange, EntryStatus};

  let mut changes = Vec::new();
  let iter = repo
    .status(gix::progress::Discard)?
    .untracked_files(gix::status::UntrackedFiles::None)
    .index_worktree_submodules(None)
    .into_index_worktree_iter(Vec::new())?;

  for item in iter {
    let Item::Modification { entry, rela_path, status, .. } = item? else { continue };
    let old = Some((entry.id, entry_kind(entry.mode)));
    let (status, new) = match status {
      EntryStatus::Change(WtChange::Removed) => ("deleted", None),
      EntryStatus::Change(WtChange::Modification { .. }) => {
        ("modified", Some((gix::ObjectId::null(repo.object_hash()), entry_kind(entry.mode))))
      }
      EntryStatus::Change(WtChange::Type { worktree_mode }) => {
        ("modified", Some((gix::ObjectId::null(repo.object_hash()), entry_kind(worktree_mode))))
      }
      EntryStatus::IntentToAdd => ("added", Some((gix::ObjectId::null(repo.object_hash()), EntryKind::Blob))),
      _ => continue,
    };
    changes.push(Change { path: rela_path, old_path: None, status, old, new });
  }
  changes.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(changes)
}

/// Tree versus tree, with rename tracking, i.e. `git diff <base> <head>`.
fn tree_changes(repo: &gix::Repository, base: &str, head: &str) -> anyhow::Result<Vec<Change>> {
  use gix::diff::tree_with_rewrites::Change as TreeChange;

  let old_tree = repo.rev_parse_single(base)?.object()?.peel_to_tree()?;
  let new_tree = repo.rev_parse_single(head)?.object()?.peel_to_tree()?;

  let mut changes = Vec::new();
  repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?.into_iter().for_each(|change| {
    // gix reports the containing tree as changed as well as its entries.
    // `git diff --raw` lists only the entries, and a tree has no diffable content.
    if change.entry_mode().is_tree() {
      return;
    }
    let entry = match change {
      TreeChange::Addition { location, id, entry_mode, .. } => Change {
        path: location.clone(),
        old_path: None,
        status: "added",
        old: None,
        new: Some((id, entry_mode.kind())),
      },
      TreeChange::Deletion { location, id, entry_mode, .. } => Change {
        path: location.clone(),
        old_path: None,
        status: "deleted",
        old: Some((id, entry_mode.kind())),
        new: None,
      },
      TreeChange::Modification { location, previous_id, previous_entry_mode, id, entry_mode, .. } => Change {
        path: location.clone(),
        old_path: None,
        status: "modified",
        old: Some((previous_id, previous_entry_mode.kind())),
        new: Some((id, entry_mode.kind())),
      },
      TreeChange::Rewrite {
        source_location,
        source_id,
        source_entry_mode,
        location,
        id,
        entry_mode,
        copy,
        ..
      } => Change {
        path: location.clone(),
        old_path: Some(source_location.clone()),
        status: if copy { "copied" } else { "renamed" },
        old: Some((source_id, source_entry_mode.kind())),
        new: Some((id, entry_mode.kind())),
      },
    };
    changes.push(entry);
  });
  changes.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(changes)
}

/// Hunks, whole-file line counts, truncation flag, binary flag.
struct FileDiffContent {
  hunks: Vec<Hunk>,
  additions: usize,
  deletions: usize,
  truncated: bool,
  binary: bool,
}

fn hunks_for(
  repo: &gix::Repository,
  cache: &mut gix::diff::blob::Platform,
  change: &Change,
  cap: usize,
) -> anyhow::Result<FileDiffContent> {
  let null = gix::ObjectId::null(repo.object_hash());
  let old_path: &BStr =
    change.old_path.as_ref().map_or(change.path.as_bstr(), |p| p.as_bstr());

  cache.clear_resource_cache_keep_allocation();
  let (old_id, old_kind) = change.old.unwrap_or((null, EntryKind::Blob));
  let (new_id, new_kind) = change.new.unwrap_or((null, EntryKind::Blob));
  cache.set_resource(old_id, old_kind, old_path, ResourceKind::OldOrSource, &repo.objects)?;
  cache.set_resource(new_id, new_kind, change.path.as_ref(), ResourceKind::NewOrDestination, &repo.objects)?;

  let outcome = cache.prepare_diff()?;
  use gix::diff::blob::platform::prepare_diff::Operation;
  let algorithm = match outcome.operation {
    Operation::InternalDiff { algorithm } => algorithm,
    Operation::ExternalCommand { .. } => return Ok(FileDiffContent::empty(false)),
    Operation::SourceOrDestinationIsBinary => return Ok(FileDiffContent::empty(true)),
  };

  let algorithm = match std::env::var("SPIKE_ALGO").as_deref() {
    Ok("myers") => gix::diff::blob::Algorithm::Myers,
    Ok("minimal") => gix::diff::blob::Algorithm::MyersMinimal,
    Ok("histogram") => gix::diff::blob::Algorithm::Histogram,
    _ => algorithm,
  };
  if std::env::var("SPIKE_ALGO_PRINT").is_ok() {
    eprintln!("algorithm={algorithm:?}");
  }
  // Keep line terminators, as git does: a change that only adds a trailing
  // newline must still show up as a deletion and an addition, not vanish.
  let input = gix::diff::blob::InternedInput::new(
    outcome.old.intern_source(),
    outcome.new.intern_source(),
  );
  let mut diff = Diff::compute(algorithm, &input);
  if std::env::var("SPIKE_NO_POSTPROCESS").is_err() {
    diff.postprocess_lines(&input);
  }
  let collector = Collector { cap, used: 0, truncated: false, hunks: Vec::new() };
  let (mut hunks, truncated) =
    UnifiedDiff::new(&diff, &input, collector, ContextSize::symmetrical(3)).consume()?;

  let before: Vec<&[u8]> = input.before.iter().map(|token| input.interner[*token]).collect();
  for hunk in &mut hunks {
    if let Some(context) = func_context(&before, hunk.old_start) {
      hunk.header = format!("{} {context}", hunk.header);
    }
  }
  Ok(FileDiffContent {
    // Whole-file counts, matching `git diff --numstat`: truncation drops hunks
    // from the payload but must not understate how large the change was.
    additions: diff.count_additions() as usize,
    deletions: diff.count_removals() as usize,
    hunks,
    truncated,
    binary: false,
  })
}

impl FileDiffContent {
  fn empty(binary: bool) -> Self {
    FileDiffContent { hunks: Vec::new(), additions: 0, deletions: 0, truncated: false, binary }
  }
}

fn main() -> anyhow::Result<()> {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let [root, mode, rest @ ..] = args.as_slice() else {
    anyhow::bail!("usage: spike <repo> <working|tree> [base head]");
  };

  let repo = gix::discover(root)?;
  let changes = match mode.as_str() {
    "working" => working_changes(&repo)?,
    "tree" => {
      let [base, head] = rest else { anyhow::bail!("tree mode needs <base> <head>") };
      tree_changes(&repo, base, head)?
    }
    other => anyhow::bail!("unknown mode: {other}"),
  };

  let roots = match mode.as_str() {
    "working" => WorktreeRoots {
      old_root: None,
      new_root: repo.workdir().map(std::path::Path::to_path_buf),
    },
    _ => WorktreeRoots::default(),
  };
  let mut cache = repo.diff_resource_cache(Mode::ToGit, roots)?;

  let mut files = Vec::new();
  for (index, change) in changes.iter().enumerate() {
    let path = change.path.to_str_lossy().into_owned();
    let noise = is_noise(&path);
    let cap = if noise { MAX_LINES_PER_NOISE_FILE } else { MAX_LINES_PER_FILE };
    let FileDiffContent { mut hunks, additions, deletions, truncated, binary } =
      hunks_for(&repo, &mut cache, change, cap)?;
    for (hunk_index, hunk) in hunks.iter_mut().enumerate() {
      hunk.id = format!("f{index}h{hunk_index}");
    }
    files.push(FileDiff {
      id: format!("f{index}"),
      old_path: change.old_path.as_ref().map_or_else(|| path.clone(), |p| p.to_str_lossy().into_owned()),
      language: language_of(&path),
      path,
      status: change.status,
      additions,
      deletions,
      binary,
      noise,
      truncated,
      hunks: if binary { Vec::new() } else { hunks },
    });
  }

  println!("{}", serde_json::to_string_pretty(&Output { files })?);
  Ok(())
}

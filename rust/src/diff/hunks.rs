//! Blob content to hunks, matching what `git diff` would have printed.

use anyhow::Result;
use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader};
use gix::diff::blob::{Algorithm, Diff, ResourceKind, UnifiedDiff};
use gix::object::tree::EntryKind;

use super::Change;
use crate::model::{DiffLine, Hunk, LineType};

/// `gix-imara-diff`'s histogram matches git's exactly; its Myers does not — 8 of
/// 112 real commits differed by a few lines of edit script, and no git config
/// reproduces it. Pinning histogram buys an exact parity assertion, at the cost
/// of differing slightly from a user's own `git diff` when their git defaults to
/// Myers. Both are valid diffs. See docs/contract.md.
const ALGORITHM: Algorithm = Algorithm::Histogram;

/// Hunks, whole-file line counts, truncation flag, binary flag.
pub struct Content {
  pub hunks: Vec<Hunk>,
  pub additions: usize,
  pub deletions: usize,
  pub truncated: bool,
  pub binary: bool,
}

impl Content {
  fn empty(binary: bool) -> Self {
    Content { hunks: Vec::new(), additions: 0, deletions: 0, truncated: false, binary }
  }
}

/// Collects `(header, lines)` callbacks into diffpane's hunk shape, stopping at
/// `cap` lines so one enormous file cannot bury the review.
struct Collector {
  cap: usize,
  used: usize,
  truncated: bool,
  hunks: Vec<Hunk>,
}

impl ConsumeHunk for Collector {
  type Out = (Vec<Hunk>, bool);

  fn consume_hunk(
    &mut self,
    header: HunkHeader,
    lines: &[(DiffLineKind, &[u8])],
  ) -> std::io::Result<()> {
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
          let line =
            DiffLine { i, kind: LineType::Context, old: Some(old_no), new: Some(new_no), text };
          old_no += 1;
          new_no += 1;
          line
        }
        DiffLineKind::Add => {
          additions += 1;
          let line = DiffLine { i, kind: LineType::Add, old: None, new: Some(new_no), text };
          new_no += 1;
          line
        }
        DiffLineKind::Remove => {
          deletions += 1;
          let line = DiffLine { i, kind: LineType::Del, old: Some(old_no), new: None, text };
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

/// The UI renders `text` itself, so the terminator is noise — and keeping `\r`
/// would render as a stray glyph on every line of a CRLF file.
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
/// letter, `_` or `$`, clamped to 80 bytes and then right-trimmed. gix's
/// `UnifiedDiff` does not produce it.
///
/// Known deviation: language drivers selected by a `diff=<lang>` gitattribute
/// are not implemented, so a file with one gets git's default here instead.
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

pub fn content(
  repo: &gix::Repository,
  cache: &mut gix::diff::blob::Platform,
  change: &Change,
  cap: usize,
) -> Result<Content> {
  use gix::diff::blob::platform::prepare_diff::Operation;

  let null = gix::ObjectId::null(repo.object_hash());
  let old_path: &BStr = change.old_path.as_ref().map_or(change.path.as_bstr(), |p| p.as_bstr());

  cache.clear_resource_cache_keep_allocation();
  let (old_id, old_kind) = change.old.unwrap_or((null, EntryKind::Blob));
  let (new_id, new_kind) = change.new.unwrap_or((null, EntryKind::Blob));
  cache.set_resource(old_id, old_kind, old_path, ResourceKind::OldOrSource, &repo.objects)?;
  cache.set_resource(
    new_id,
    new_kind,
    change.path.as_ref(),
    ResourceKind::NewOrDestination,
    &repo.objects,
  )?;

  let outcome = cache.prepare_diff()?;
  match outcome.operation {
    Operation::InternalDiff { .. } => {}
    // An external diff driver's output is not a unified diff we can read.
    Operation::ExternalCommand { .. } => return Ok(Content::empty(false)),
    Operation::SourceOrDestinationIsBinary => return Ok(Content::empty(true)),
  }

  // `intern_source()`, not `interned_input()`: the latter strips line
  // terminators, so a change that only adds a trailing newline collapses into a
  // context line and the diff shows nothing at all.
  let input =
    gix::diff::blob::InternedInput::new(outcome.old.intern_source(), outcome.new.intern_source());
  let mut diff = Diff::compute(ALGORITHM, &input);
  diff.postprocess_lines(&input);

  let collector = Collector { cap, used: 0, truncated: false, hunks: Vec::new() };
  let (mut hunks, truncated) =
    UnifiedDiff::new(&diff, &input, collector, ContextSize::symmetrical(3)).consume()?;

  let before: Vec<&[u8]> = input.before.iter().map(|token| input.interner[*token]).collect();
  for hunk in &mut hunks {
    if let Some(context) = func_context(&before, hunk.old_start) {
      hunk.header = format!("{} {context}", hunk.header);
    }
  }

  Ok(Content {
    // Whole-file counts, matching `git diff --numstat`: truncation drops hunks
    // from the payload but must not understate how large the change was.
    additions: diff.count_additions() as usize,
    deletions: diff.count_removals() as usize,
    hunks,
    truncated,
    binary: false,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn drops_both_halves_of_a_crlf_terminator() {
    assert_eq!(strip_terminator(b"line\r\n"), b"line");
    assert_eq!(strip_terminator(b"line\n"), b"line");
    assert_eq!(strip_terminator(b"line"), b"line");
    // A lone \r mid-file is content, not a terminator.
    assert_eq!(strip_terminator(b"line\r"), b"line");
  }

  #[test]
  fn reports_an_empty_side_at_the_line_before_the_hunk() {
    assert_eq!(start_of(1, 0), 0);
    assert_eq!(start_of(1, 6), 1);
    assert_eq!(start_of(0, 0), 0);
  }

  #[test]
  fn omits_a_count_of_one_from_the_header() {
    let header = HunkHeader {
      before_hunk_start: 4,
      before_hunk_len: 1,
      after_hunk_start: 4,
      after_hunk_len: 3,
    };
    assert_eq!(render_header(header), "@@ -4 +4,3 @@");
  }

  #[test]
  fn takes_func_context_from_the_nearest_qualifying_line_above() {
    let before: Vec<&[u8]> = vec![b"fn outer() {", b"  let x = 1;", b"  let y = 2;"];
    assert_eq!(func_context(&before, 3), Some("fn outer() {".to_string()));
    // Nothing above the first line qualifies.
    assert_eq!(func_context(&before, 1), None);
  }

  #[test]
  fn clamps_func_context_to_eighty_bytes_and_right_trims() {
    let long = format!("fn {}   ", "a".repeat(120));
    let before: Vec<&[u8]> = vec![long.as_bytes(), b"body"];
    let context = func_context(&before, 2).expect("context");
    assert_eq!(context.len(), 80);
    assert!(!context.ends_with(' '));
  }
}

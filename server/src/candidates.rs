use std::collections::BTreeMap;

use crate::model::{CandidateMatch, Candidates, CommentCandidate, FileDiff, Side};

struct Block {
  file: String,
  anchor: String,
  body: Vec<String>,
}

fn split_blocks(markdown: &str) -> Vec<Block> {
  let mut blocks = Vec::new();
  let mut current: Option<Block> = None;
  for line in markdown.lines() {
    if let Some(header) = line.strip_prefix("## ") {
      if let Some(block) = current.take() {
        blocks.push(block);
      }
      let (file, anchor) = match header.split_once(" — ") {
        Some((file, anchor)) => (file.trim().to_string(), strip_backticks(anchor.trim())),
        None => (header.trim().to_string(), String::new()),
      };
      current = Some(Block { file, anchor, body: Vec::new() });
    } else if let Some(block) = current.as_mut() {
      block.body.push(line.to_string());
    }
  }
  if let Some(block) = current.take() {
    blocks.push(block);
  }
  blocks
}

fn strip_backticks(text: &str) -> String {
  text.strip_prefix('`').and_then(|rest| rest.strip_suffix('`')).unwrap_or(text).to_string()
}

fn extract_proposed(body: &[String]) -> Option<String> {
  let start = body.iter().position(|line| line.trim().eq_ignore_ascii_case("proposed:"))? + 1;
  let mut collected = Vec::new();
  for line in &body[start..] {
    if line.trim().is_empty() {
      if collected.is_empty() {
        continue;
      }
      break;
    }
    match line.strip_prefix("    ") {
      Some(rest) => collected.push(rest.to_string()),
      None => break,
    }
  }
  if collected.is_empty() { None } else { Some(collected.join("\n")) }
}

fn looks_like_identifier(token: &str) -> bool {
  if token.contains('_') {
    return true;
  }
  let mut saw_lower = false;
  for ch in token.chars() {
    if ch.is_uppercase() && saw_lower {
      return true;
    }
    if ch.is_lowercase() {
      saw_lower = true;
    }
  }
  false
}

fn candidate_tokens(anchor: &str) -> Vec<String> {
  let mut needles = Vec::new();
  if !anchor.trim().is_empty() {
    needles.push(anchor.to_string());
  }
  for token in anchor.split(|ch: char| ch.is_whitespace() || ch == '/') {
    let trimmed = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_');
    if trimmed.len() >= 3 && looks_like_identifier(trimmed) {
      needles.push(trimmed.to_string());
    }
  }
  needles
}

fn locate(anchor: &str, file: &FileDiff) -> Option<CandidateMatch> {
  for needle in candidate_tokens(anchor) {
    for hunk in &file.hunks {
      for line in &hunk.lines {
        if line.new.is_some() && line.text.contains(&needle) {
          return Some(CandidateMatch { hunk: hunk.id.clone(), side: Side::New, line: line.new? });
        }
      }
    }
    for hunk in &file.hunks {
      for line in &hunk.lines {
        if line.old.is_some() && line.text.contains(&needle) {
          return Some(CandidateMatch { hunk: hunk.id.clone(), side: Side::Old, line: line.old? });
        }
      }
    }
  }
  None
}

pub fn build(markdown: &str, files: &[FileDiff]) -> Candidates {
  let by_path: BTreeMap<&str, &FileDiff> =
    files.iter().map(|file| (file.path.as_str(), file)).collect();
  let items = split_blocks(markdown)
    .into_iter()
    .enumerate()
    .filter(|(_, block)| !block.file.is_empty())
    .map(|(index, block)| {
      let location = by_path.get(block.file.as_str()).and_then(|file| locate(&block.anchor, file));
      CommentCandidate {
        id: format!("cc-{index:04}"),
        file: block.file,
        anchor: block.anchor,
        proposed: extract_proposed(&block.body),
        rationale: block.body.join("\n").trim().to_string(),
        location,
      }
    })
    .collect();
  Candidates { items }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::{DiffLine, FileStatus, Hunk, LineType};

  fn file(path: &str, lines: Vec<DiffLine>) -> FileDiff {
    FileDiff {
      id: "f0".into(),
      path: path.into(),
      old_path: path.into(),
      status: FileStatus::Modified,
      additions: 1,
      deletions: 0,
      binary: false,
      noise: false,
      language: None,
      truncated: false,
      hunks: vec![Hunk {
        id: "f0h0".into(),
        header: String::new(),
        old_start: 1,
        old_count: 0,
        new_start: 1,
        new_count: lines.len() as u32,
        additions: lines.len(),
        deletions: 0,
        lines,
      }],
    }
  }

  fn add(i: usize, new: u32, text: &str) -> DiffLine {
    DiffLine { i, kind: LineType::Add, old: None, new: Some(new), text: text.into() }
  }

  #[test]
  fn an_exact_backtick_anchor_matches_its_line() {
    let markdown = "## src/a.ts — `fullFacetFields`\n\nBody text.\n";
    let files = [file("src/a.ts", vec![add(0, 7, "const fullFacetFields = [];")])];
    let candidates = build(markdown, &files);
    assert_eq!(candidates.items.len(), 1);
    let location = candidates.items[0].location.as_ref().expect("should match");
    assert_eq!(location.hunk, "f0h0");
    assert_eq!(location.line, 7);
  }

  #[test]
  fn a_proposed_block_is_extracted_dedented() {
    let markdown =
      "## src/a.ts — x\n\nProposed:\n\n    // first line\n    // second line\n\nMore rationale.\n";
    let candidates = build(markdown, &[]);
    assert_eq!(candidates.items[0].proposed.as_deref(), Some("// first line\n// second line"));
  }

  #[test]
  fn prose_anchors_with_no_code_symbol_are_unplaced() {
    let markdown = "## src/a.ts — hoisted mock results\n\nBody.\n";
    let files = [file("src/a.ts", vec![add(0, 1, "const hoisted = mock(results);")])];
    let candidates = build(markdown, &files);
    assert!(candidates.items[0].location.is_none());
  }

  #[test]
  fn a_camel_case_token_inside_a_multi_word_anchor_is_tried() {
    let markdown = "## src/a.ts — withReferrer / item hrefs\n\nBody.\n";
    let files = [file("src/a.ts", vec![add(0, 3, "function withReferrer(url) {")])];
    let candidates = build(markdown, &files);
    let location = candidates.items[0].location.as_ref().expect("should match");
    assert_eq!(location.line, 3);
  }

  #[test]
  fn an_unknown_file_is_unplaced_not_dropped() {
    let markdown = "## src/missing.ts — thing\n\nBody.\n";
    let candidates = build(markdown, &[]);
    assert_eq!(candidates.items.len(), 1);
    assert!(candidates.items[0].location.is_none());
  }

  #[test]
  fn a_header_with_no_anchor_does_not_match_every_line() {
    let markdown = "## src/a.ts\n\nBody.\n";
    let files = [file("src/a.ts", vec![add(0, 1, "whatever is on this line")])];
    let candidates = build(markdown, &files);
    assert!(candidates.items[0].location.is_none());
  }
}

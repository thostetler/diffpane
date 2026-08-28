//! Boundary validation for the HTTP API.
//!
//! Serde would reject most of this on its own, but regression #9 is about the
//! *messages and statuses* the UI sees, so the checks stay hand-rolled and the
//! wording matches the TypeScript.

use serde_json::Value;

use crate::model::{Anchor, AnchorKind, ProgressState, Side, Verdict};

#[derive(Debug, Clone)]
pub struct ApiError {
  pub status: u16,
  pub message: String,
}

impl ApiError {
  pub fn new(message: impl Into<String>, status: u16) -> Self {
    Self { status, message: message.into() }
  }

  /// The default: a client mistake at the boundary.
  pub fn bad_request(message: impl Into<String>) -> Self {
    Self::new(message, 400)
  }
}

impl std::fmt::Display for ApiError {
  fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(out, "{}", self.message)
  }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn validate_verdict(value: Option<&Value>) -> ApiResult<Verdict> {
  match value.and_then(Value::as_str) {
    Some("ok") => Ok(Verdict::Ok),
    Some("fix") => Ok(Verdict::Fix),
    Some("question") => Ok(Verdict::Question),
    _ => Err(ApiError::bad_request("verdict must be one of ok, fix, question")),
  }
}

pub fn validate_progress_state(value: Option<&Value>) -> ApiResult<ProgressState> {
  match value.and_then(Value::as_str) {
    Some("unreviewed") => Ok(ProgressState::Unreviewed),
    Some("reviewed") => Ok(ProgressState::Reviewed),
    _ => Err(ApiError::bad_request("state must be one of unreviewed, reviewed")),
  }
}

pub fn validate_body(value: Option<&Value>) -> ApiResult<String> {
  let text = value.and_then(Value::as_str).unwrap_or("").trim();
  if text.is_empty() {
    return Err(ApiError::bad_request("comment body is empty"));
  }
  Ok(text.to_string())
}

pub fn validate_resolved(value: Option<&Value>) -> ApiResult<bool> {
  match value {
    Some(Value::Bool(flag)) => Ok(*flag),
    _ => Err(ApiError::bad_request("resolved must be a boolean")),
  }
}

/// An optional string field: trimmed to `None` when absent, empty or null.
fn optional_string(anchor: &Value, field: &str) -> ApiResult<Option<String>> {
  match anchor.get(field) {
    None | Some(Value::Null) => Ok(None),
    Some(Value::String(text)) if text.is_empty() => Ok(None),
    Some(Value::String(text)) => Ok(Some(text.clone())),
    Some(_) => Err(ApiError::bad_request(format!("anchor.{field} must be a string"))),
  }
}

/// Present-and-a-non-empty-string. Presence alone let objects reach the report.
fn require_string(anchor: &Value, kind: &str, field: &str) -> ApiResult<String> {
  match optional_string(anchor, field)? {
    Some(text) => Ok(text),
    None => Err(ApiError::bad_request(format!("{kind} anchor requires {field}"))),
  }
}

fn anchor_kind(value: Option<&Value>) -> ApiResult<AnchorKind> {
  match value.and_then(Value::as_str) {
    Some("line") => Ok(AnchorKind::Line),
    Some("file") => Ok(AnchorKind::File),
    Some("chapter") => Ok(AnchorKind::Chapter),
    Some("overall") => Ok(AnchorKind::Overall),
    _ => Err(ApiError::bad_request("anchor.kind must be one of line, file, chapter, overall")),
  }
}

fn validate_side(anchor: &Value) -> ApiResult<Side> {
  match anchor.get("side").and_then(Value::as_str) {
    Some("old") => Ok(Side::Old),
    Some("new") => Ok(Side::New),
    _ => Err(ApiError::bad_request("anchor.side must be 'old' or 'new'")),
  }
}

fn validate_line(anchor: &Value) -> ApiResult<u32> {
  let line = anchor.get("line").and_then(Value::as_u64).filter(|line| *line >= 1);
  match line {
    Some(line) if line <= u64::from(u32::MAX) => Ok(line as u32),
    _ => Err(ApiError::bad_request("anchor.line must be a positive integer")),
  }
}

pub fn validate_anchor(value: Option<&Value>) -> ApiResult<Anchor> {
  let anchor = match value {
    Some(value) if value.is_object() => value,
    _ => return Err(ApiError::bad_request("anchor must be an object")),
  };
  let kind = anchor_kind(anchor.get("kind"))?;

  let mut result = Anchor {
    kind,
    file: None,
    hunk: None,
    side: None,
    line: None,
    chapter: optional_string(anchor, "chapter")?,
  };

  match kind {
    AnchorKind::Line => {
      result.file = Some(require_string(anchor, "line", "file")?);
      result.hunk = Some(require_string(anchor, "line", "hunk")?);
      require_string(anchor, "line", "side")?;
      result.side = Some(validate_side(anchor)?);
      result.line = Some(validate_line(anchor)?);
    }
    AnchorKind::File => {
      result.file = Some(require_string(anchor, "file", "file")?);
      result.hunk = optional_string(anchor, "hunk")?;
    }
    AnchorKind::Chapter => {
      result.chapter = Some(require_string(anchor, "chapter", "chapter")?);
      result.file = optional_string(anchor, "file")?;
      result.hunk = optional_string(anchor, "hunk")?;
    }
    AnchorKind::Overall => {
      result.file = optional_string(anchor, "file")?;
      result.hunk = optional_string(anchor, "hunk")?;
    }
  }
  Ok(result)
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  fn anchor(value: Value) -> ApiResult<Anchor> {
    validate_anchor(Some(&value))
  }

  #[test]
  fn accepts_a_well_formed_line_anchor() {
    let value = json!({
      "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new", "line": 12, "chapter": "c1"
    });
    let parsed = anchor(value).unwrap();
    assert_eq!(parsed.kind, AnchorKind::Line);
    assert_eq!(parsed.line, Some(12));
    assert_eq!(parsed.chapter.as_deref(), Some("c1"));
    assert_eq!(parsed.side, Some(Side::New));
  }

  #[test]
  fn normalises_absent_anchor_fields_to_null() {
    let parsed = anchor(json!({ "kind": "file", "file": "a.ts" })).unwrap();
    assert_eq!(parsed.hunk, None);
    assert_eq!(parsed.line, None);
    assert_eq!(parsed.chapter, None);
  }

  #[test]
  fn rejects_malformed_anchors() {
    let cases = [
      Value::Null,
      json!("nope"),
      json!([]),
      json!({ "kind": "nonsense" }),
      json!({ "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new" }),
      json!({ "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "sideways", "line": 1 }),
      json!({ "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new", "line": 1.5 }),
      json!({ "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new", "line": 0 }),
      json!({ "kind": "file" }),
      json!({ "kind": "chapter" }),
      // Regression #9: an object here reached the report as `## [object Object]`.
      json!({ "kind": "file", "file": { "path": "a.ts" } }),
      json!({ "kind": "chapter", "chapter": { "id": "c1" } }),
    ];
    for case in cases {
      assert!(anchor(case.clone()).is_err(), "{case}");
    }
  }

  #[test]
  fn rejects_unknown_verdicts() {
    assert_eq!(validate_verdict(Some(&json!("fix"))).unwrap(), Verdict::Fix);
    assert!(validate_verdict(Some(&json!("lgtm"))).is_err());
    assert!(validate_verdict(None).is_err());
    assert!(validate_verdict(Some(&Value::Null)).is_err());
  }

  #[test]
  fn rejects_empty_comment_bodies() {
    assert_eq!(validate_body(Some(&json!("  hi  "))).unwrap(), "hi");
    assert!(validate_body(Some(&json!("   "))).is_err());
    assert!(validate_body(Some(&json!(42))).is_err());
    assert!(validate_body(None).is_err());
  }

  #[test]
  fn requires_a_real_boolean_for_resolved() {
    assert!(!validate_resolved(Some(&json!(false))).unwrap());
    // `Boolean("false")` was `true`, which is how this got through once.
    assert!(validate_resolved(Some(&json!("false"))).is_err());
    assert!(validate_resolved(Some(&json!(0))).is_err());
  }

  #[test]
  fn validates_progress_states() {
    assert_eq!(validate_progress_state(Some(&json!("reviewed"))).unwrap(), ProgressState::Reviewed);
    assert!(validate_progress_state(Some(&json!("done"))).is_err());
  }
}

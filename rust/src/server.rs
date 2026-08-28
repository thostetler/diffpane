//! The local HTTP server: the browser UI plus the JSON API it drives.
//!
//! The security posture is `docs/contract.md`'s and is not negotiable:
//! loopback-literal `Host`, the page gated on `?t=`, `/api/*` gated on the
//! `X-Diffpane-Token` header *only* (a cookie there would hand CSRF straight
//! back), and `application/json` required on mutations.
//!
//! Regression #7 — the submit response must flush before the process tears the
//! server down — is handled structurally here: a submit only signals on
//! `submitted`, and the caller drives `axum::serve`'s graceful shutdown, which
//! waits for in-flight responses. Nothing in this module destroys a socket.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::header::{
  CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, HeaderMap, REFERRER_POLICY, SET_COOKIE,
};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::assets::Assets;
use crate::model::{Comment, Overall, ReviewState};
use crate::session::{Session, now_iso};
use crate::validate::{
  ApiError, ApiResult, validate_anchor, validate_body, validate_progress_state, validate_resolved,
  validate_verdict,
};

const MAX_BODY_BYTES: usize = 1024 * 1024;
const COOKIE_NAME: &str = "diffpane_token";
const PORT_ATTEMPTS: u16 = 20;

pub struct AppState {
  session: Session,
  token: String,
  assets: Assets,
  /// Fires when a submit is routed. The receiver starts a graceful shutdown,
  /// which is what lets the response finish flushing (regression #7).
  submitted: mpsc::Sender<()>,
  /// Held across every read-modify-write of the review state; see `mutate`.
  writes: std::sync::Mutex<()>,
}

impl AppState {
  pub fn new(session: Session, token: String, assets: Assets, submitted: mpsc::Sender<()>) -> Self {
    Self { session, token, assets, submitted, writes: std::sync::Mutex::new(()) }
  }

  /// The report is rendered from the same session the server has been writing.
  pub fn session(&self) -> &Session {
    &self.session
  }
}

/// 16 bytes from the OS. This token is the whole of the access control —
/// loopback is not access control, per `docs/contract.md` — so it must not come
/// from `fastrand`, which is a fast PRNG and not a CSPRNG.
pub fn generate_token() -> String {
  let mut bytes = [0u8; 16];
  getrandom::fill(&mut bytes).expect("the OS must be able to produce 16 random bytes");
  bytes.iter().fold(String::with_capacity(32), |mut out, byte| {
    use std::fmt::Write;
    let _ = write!(out, "{byte:02x}");
    out
  })
}

/// Constant-time within a length class; the lengths themselves are not secret.
fn safe_equal(left: &str, right: &str) -> bool {
  let (left, right) = (left.as_bytes(), right.as_bytes());
  if left.len() != right.len() {
    return false;
  }
  left.iter().zip(right).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

/// Strips the port, tolerating `[::1]:8080` and a bare unbracketed `::1`.
fn host_name(host: &str) -> &str {
  if let Some(rest) = host.strip_prefix('[') {
    return rest.split_once(']').map_or("", |(name, _)| name);
  }
  // An unbracketed IPv6 literal cannot carry a port, so there is none to strip.
  if host.matches(':').count() > 1 {
    return host;
  }
  match host.rsplit_once(':') {
    Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
    _ => host,
  }
}

/// Rejects DNS-rebinding: the browser must be talking to a loopback literal.
/// `localhost` is a name, and `docs/contract.md` says literal — a resolver that
/// hands back something else is precisely the attack.
fn is_loopback_host(headers: &HeaderMap) -> bool {
  let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
    return false;
  };
  matches!(host_name(host), "127.0.0.1" | "::1")
}

/// The media type's essence: everything before the first `;`, lowercased. A
/// substring match accepted `text/plain; x=application/json`, which is a
/// CORS-simple content type and so defeats the point of requiring JSON.
fn media_type_essence(headers: &HeaderMap) -> String {
  headers
    .get(CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("")
    .split(';')
    .next()
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase()
}

fn read_cookie(headers: &HeaderMap, name: &str) -> String {
  let header = headers.get(COOKIE).and_then(|value| value.to_str().ok()).unwrap_or("");
  for part in header.split(';') {
    if let Some((key, value)) = part.trim().split_once('=')
      && key == name
    {
      return value.to_string();
    }
  }
  String::new()
}

fn content_type_for(path: &Path) -> &'static str {
  match path.extension().and_then(|ext| ext.to_str()) {
    Some("html") => "text/html; charset=utf-8",
    Some("js") => "text/javascript; charset=utf-8",
    Some("css") => "text/css; charset=utf-8",
    Some("json") => "application/json; charset=utf-8",
    Some("svg") => "image/svg+xml",
    Some("png") => "image/png",
    Some("ico") => "image/x-icon",
    _ => "application/octet-stream",
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json_response(status, &json!({ "error": self.message }))
  }
}

fn hardened(mut response: Response) -> Response {
  let headers = response.headers_mut();
  headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
  headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
  headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
  response
}

fn json_response(status: StatusCode, payload: &Value) -> Response {
  let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
  hardened((status, [(CONTENT_TYPE, "application/json; charset=utf-8")], body).into_response())
}

fn ok(payload: Value) -> ApiResult<Response> {
  Ok(json_response(StatusCode::OK, &payload))
}

fn internal(error: anyhow::Error) -> ApiError {
  ApiError::new(format!("{error:#}"), 500)
}

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
  t: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
  let api = Router::new()
    .route("/review", get(get_review))
    .route("/state", get(get_state))
    .route("/comments", post(create_comment))
    .route("/comments/{id}", axum::routing::delete(delete_comment).patch(patch_comment))
    .route("/progress", put(put_progress))
    .route("/overall", put(put_overall))
    .route("/submit", post(post_submit))
    // Both arms answer alike: the token is checked before the path is, so to an
    // unauthenticated caller an unknown path and a wrong method are the same
    // 403 and neither reveals which endpoints exist. Past the token, a
    // mutation without `application/json` still answers 415 before 404.
    .fallback(api_not_found)
    .method_not_allowed_fallback(api_not_found);

  Router::new()
    .route("/", get(serve_page))
    .route("/favicon.ico", get(serve_favicon))
    .route("/assets/{*name}", get(serve_asset))
    .nest("/api", api)
    .fallback(not_found)
    .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
    .with_state(state)
}

/// Binds loopback, walking forward from `preferred` when the port is taken.
/// Port 0 is honoured as "any", so callers get whatever the OS handed out.
pub async fn bind(preferred: u16) -> Result<TcpListener> {
  let mut port = preferred;
  // Counted, not compared against `preferred + PORT_ATTEMPTS`, which overflows
  // u16 for a preferred port near the top of the range.
  let mut attempts = PORT_ATTEMPTS;
  loop {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    match TcpListener::bind(address).await {
      Ok(listener) => return Ok(listener),
      Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && preferred != 0 => {
        match port.checked_add(1).filter(|_| attempts > 0) {
          Some(next) => {
            port = next;
            attempts -= 1;
          }
          None => return Err(error).with_context(|| format!("bind 127.0.0.1:{port}")),
        }
      }
      Err(error) => return Err(error).with_context(|| format!("bind 127.0.0.1:{port}")),
    }
  }
}

async fn api_not_found(
  State(state): State<Arc<AppState>>,
  method: Method,
  headers: HeaderMap,
) -> Response {
  match state.guard_api(&headers, &method) {
    Ok(()) => ApiError::new("no such endpoint", 404).into_response(),
    Err(error) => error.into_response(),
  }
}

async fn not_found(headers: HeaderMap) -> Response {
  if !is_loopback_host(&headers) {
    return ApiError::new("forbidden host", 403).into_response();
  }
  ApiError::new("not found", 404).into_response()
}

async fn serve_page(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  Query(query): Query<TokenQuery>,
) -> ApiResult<Response> {
  guard_host(&headers)?;
  // The page is gated on the token so a stray tab cannot read the diff. The
  // browser fetches app.css and app.js on its own, without a query string, so
  // the page load hands out a cookie for those follow-up requests.
  state.require_token(query.t.as_deref().unwrap_or(""))?;
  let file = state.read_ui_file("index.html")?;
  let cookie = format!("{COOKIE_NAME}={}; Path=/; SameSite=Strict; Max-Age=86400", state.token);
  Ok(hardened(
    (
      StatusCode::OK,
      [(CONTENT_TYPE, "text/html; charset=utf-8".to_string()), (SET_COOKIE, cookie)],
      file,
    )
      .into_response(),
  ))
}

/// Served before the token check, as an empty 204 once was: the browser asks
/// for this at the root with no cookie, and an icon is not the diff.
async fn serve_favicon(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
) -> ApiResult<Response> {
  guard_host(&headers)?;
  let file = state.read_ui_file("favicon.ico")?;
  Ok(hardened((StatusCode::OK, [(CONTENT_TYPE, "image/x-icon")], file).into_response()))
}

async fn serve_asset(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  UrlPath(name): UrlPath<String>,
  Query(query): Query<TokenQuery>,
) -> ApiResult<Response> {
  guard_host(&headers)?;
  let supplied = match query.t {
    Some(token) => token,
    None => read_cookie(&headers, COOKIE_NAME),
  };
  state.require_token(&supplied)?;
  let file = state.read_ui_file(&name)?;
  let content_type = content_type_for(Path::new(&name));
  Ok(hardened((StatusCode::OK, [(CONTENT_TYPE, content_type)], file).into_response()))
}

fn guard_host(headers: &HeaderMap) -> ApiResult<()> {
  if is_loopback_host(headers) { Ok(()) } else { Err(ApiError::new("forbidden host", 403)) }
}

impl AppState {
  fn require_token(&self, supplied: &str) -> ApiResult<()> {
    if safe_equal(supplied, &self.token) {
      return Ok(());
    }
    Err(ApiError::new("missing or invalid token", 403))
  }

  fn read_ui_file(&self, name: &str) -> ApiResult<Vec<u8>> {
    self.assets.read(name).ok_or_else(|| ApiError::new("not found", 404))
  }

  /// The API is gated on a custom header and nothing else. A custom header
  /// cannot be set cross-origin without a preflight, which is what actually
  /// blocks drive-by requests from another tab; the asset cookie must never
  /// authorise this, or CSRF comes straight back.
  fn guard_api(&self, headers: &HeaderMap, method: &Method) -> ApiResult<()> {
    guard_host(headers)?;
    let supplied =
      headers.get("x-diffpane-token").and_then(|value| value.to_str().ok()).unwrap_or("");
    self.require_token(supplied)?;
    if method != Method::GET && media_type_essence(headers) != "application/json" {
      return Err(ApiError::new("content-type must be application/json", 415));
    }
    Ok(())
  }

  fn state(&self) -> ApiResult<ReviewState> {
    self.session.state().map_err(internal)
  }

  /// Read-modify-write on `comments.json`, serialised. Two requests that both
  /// read before either saved silently dropped one of the changes, and the
  /// contract promises the UI that a 2xx is durable. The TypeScript got this
  /// for free from the event loop; a threaded runtime has to say it.
  ///
  /// The lock is a blocking one held across the file I/O. Nothing awaits inside
  /// it, the files are small and local, and there is exactly one reviewer.
  fn mutate<T>(&self, change: impl FnOnce(&mut ReviewState) -> ApiResult<T>) -> ApiResult<T> {
    let _writing = self.writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut state = self.state()?;
    let result = change(&mut state)?;
    self.session.save_state(&state).map_err(internal)?;
    Ok(result)
  }

  /// `unsorted` is the synthetic trailing chapter the UI appends; see contract.
  fn chapter_ids(&self) -> ApiResult<BTreeSet<String>> {
    let review = self.session.review().map_err(internal)?;
    let mut ids: BTreeSet<String> = review
      .map(|review| review.chapters.into_iter().map(|chapter| chapter.id).collect())
      .unwrap_or_default();
    ids.insert("unsorted".to_string());
    Ok(ids)
  }
}

/// An empty body is `{}`, matching the TypeScript; anything else must be a
/// JSON object, because every endpoint reads named fields off it.
fn parse_body(bytes: &Bytes) -> ApiResult<Value> {
  if bytes.is_empty() {
    return Ok(json!({}));
  }
  let parsed: Value = serde_json::from_slice(bytes)
    .map_err(|error| ApiError::bad_request(format!("invalid JSON: {error}")))?;
  if !parsed.is_object() {
    return Err(ApiError::bad_request("body must be a JSON object"));
  }
  Ok(parsed)
}

fn to_value<T: serde::Serialize>(payload: &T) -> ApiResult<Value> {
  serde_json::to_value(payload).map_err(|error| internal(error.into()))
}

async fn get_review(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::GET)?;
  let session = &state.session;
  ok(json!({
    "meta": to_value(&session.meta().map_err(internal)?)?,
    "hunks": to_value(&session.hunks().map_err(internal)?)?,
    "review": to_value(&session.review().map_err(internal)?)?,
    "comments": to_value(&state.state()?)?,
  }))
}

async fn get_state(State(state): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::GET)?;
  ok(to_value(&state.state()?)?)
}

async fn create_comment(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::POST)?;
  let body = parse_body(&body)?;
  let stamp = now_iso();
  let comment = Comment {
    id: format!("c-{:02x}{:02x}{:02x}", fastrand::u8(..), fastrand::u8(..), fastrand::u8(..)),
    anchor: validate_anchor(body.get("anchor"))?,
    verdict: validate_verdict(body.get("verdict"))?,
    body: validate_body(body.get("body"))?,
    created_at: stamp.clone(),
    updated_at: stamp,
    resolved: false,
  };
  state.mutate(|current| {
    current.comments.push(comment.clone());
    Ok(())
  })?;
  Ok(json_response(StatusCode::CREATED, &to_value(&comment)?))
}

async fn patch_comment(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  UrlPath(id): UrlPath<String>,
  body: Bytes,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::PATCH)?;
  let body = parse_body(&body)?;
  let verdict = match body.get("verdict") {
    Some(_) => Some(validate_verdict(body.get("verdict"))?),
    None => None,
  };
  let text = match body.get("body") {
    Some(_) => Some(validate_body(body.get("body"))?),
    None => None,
  };
  let resolved = match body.get("resolved") {
    Some(_) => Some(validate_resolved(body.get("resolved"))?),
    None => None,
  };

  let payload = state.mutate(|current| {
    let comment = current
      .comments
      .iter_mut()
      .find(|comment| comment.id == id)
      .ok_or_else(|| ApiError::new(format!("no such comment: {id}"), 404))?;
    if let Some(verdict) = verdict {
      comment.verdict = verdict;
    }
    if let Some(text) = text {
      comment.body = text;
    }
    if let Some(resolved) = resolved {
      comment.resolved = resolved;
    }
    comment.updated_at = now_iso();
    to_value(comment)
  })?;
  ok(payload)
}

async fn delete_comment(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  UrlPath(id): UrlPath<String>,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::DELETE)?;
  state.mutate(|current| {
    let before = current.comments.len();
    current.comments.retain(|comment| comment.id != id);
    if current.comments.len() == before {
      return Err(ApiError::new(format!("no such comment: {id}"), 404));
    }
    Ok(())
  })?;
  ok(json!({ "ok": true }))
}

async fn put_progress(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::PUT)?;
  let body = parse_body(&body)?;
  let chapter = match body.get("chapter").and_then(Value::as_str) {
    Some(chapter) if !chapter.is_empty() => chapter.to_string(),
    _ => return Err(ApiError::bad_request("chapter is required")),
  };
  if !state.chapter_ids()?.contains(&chapter) {
    return Err(ApiError::bad_request(format!("no such chapter: {chapter}")));
  }
  let value = validate_progress_state(body.get("state"))?;
  let payload = state.mutate(|current| {
    current.progress.insert(chapter, value);
    Ok(json!({ "progress": to_value(&current.progress)? }))
  })?;
  ok(payload)
}

fn parse_overall(body: &Value) -> ApiResult<Overall> {
  let verdict = match body.get("verdict") {
    None | Some(Value::Null) => None,
    Some(_) => Some(validate_verdict(body.get("verdict"))?),
  };
  let text = body.get("body").and_then(Value::as_str).unwrap_or("").trim().to_string();
  Ok(Overall { verdict, body: text })
}

async fn put_overall(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::PUT)?;
  let overall = parse_overall(&parse_body(&body)?)?;
  let payload = state.mutate(|current| {
    current.overall = overall;
    Ok(json!({ "overall": to_value(&current.overall)? }))
  })?;
  ok(payload)
}

async fn post_submit(
  State(state): State<Arc<AppState>>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Response> {
  state.guard_api(&headers, &Method::POST)?;
  let body = parse_body(&body)?;
  let stamp = now_iso();
  state.mutate(|current| {
    if let Some(overall) = body.get("overall").filter(|value| value.is_object()) {
      current.overall = parse_overall(overall)?;
    }
    current.submitted = true;
    current.submitted_at = Some(stamp.clone());
    Ok(())
  })?;
  // Signalling before the response is written is safe: the receiver's shutdown
  // is graceful, so this response still flushes (regression #7). A full channel
  // means a submit is already being handled, and one signal is enough.
  let _ = state.submitted.try_send(());
  ok(json!({ "submitted": true, "submitted_at": stamp }))
}

/// Runs until `shutdown` resolves. The shutdown is graceful, so a response
/// already on the wire — the submit response, in particular — finishes first.
pub async fn serve(
  listener: TcpListener,
  state: Arc<AppState>,
  shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
  axum::serve(listener, router(state)).with_graceful_shutdown(shutdown).await.context("serve")
}

#[cfg(test)]
mod tests;

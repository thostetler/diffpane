use reqwest::header::{CONTENT_TYPE, COOKIE, HOST};
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::*;
use crate::model::{Hunks, Meta, ProgressState, Review, Scope, Totals, Verdict};
use crate::session::write_json;

const TOKEN_HEADER: &str = "X-Diffpane-Token";

/// The assets a released binary serves: compiled in, no directory involved.
/// The override path has its own coverage in `assets`.
fn assets() -> Assets {
  Assets::Embedded
}

fn meta() -> Meta {
  Meta {
    repo: "demo".into(),
    repo_root: "/tmp/demo".into(),
    slug: "demo".into(),
    title: "Demo".into(),
    scope: Scope::Branch,
    base: "main".into(),
    head: "feature".into(),
    diff_cmd: "git diff main...HEAD".into(),
    generated_at: "2026-01-01T00:00:00Z".into(),
    totals: Totals::default(),
  }
}

fn review() -> Review {
  Review {
    title: None,
    story: None,
    chapters: vec![crate::model::Chapter {
      id: "c1".into(),
      title: "Cache layer".into(),
      intent: None,
      why: None,
      hunks: vec!["f0h0".into()],
      size: None,
      flags: None,
    }],
    file_notes: None,
  }
}

fn anchor() -> Value {
  json!({ "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new", "line": 1 })
}

struct Harness {
  base: String,
  token: String,
  state: Arc<AppState>,
  client: Client,
  _temp: tempfile::TempDir,
  shutdown: Option<oneshot::Sender<()>>,
  submitted: Option<mpsc::Receiver<()>>,
}

impl Harness {
  /// Boots the real server on an ephemeral port over a temp session dir, the
  /// way `src/server.test.ts` does. `seed_review` mirrors the fixture that has
  /// a `review.json`, which the progress cases need a real chapter from.
  async fn start(seed_review: bool) -> Self {
    let temp = tempfile::tempdir().expect("temp dir");
    let session = Session::new(temp.path().to_path_buf());
    write_json(&session.meta_path(), &meta()).expect("meta");
    write_json(&session.hunks_path(), &Hunks::default()).expect("hunks");
    if seed_review {
      write_json(&session.review_path(), &review()).expect("review");
    }

    let token = generate_token();
    let (submit_tx, submit_rx) = mpsc::channel(1);
    let state = Arc::new(AppState::new(session, token.clone(), assets(), submit_tx));
    let listener = bind(0).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let served = Arc::clone(&state);
    tokio::spawn(async move {
      let _ = serve(listener, served, async {
        let _ = shutdown_rx.await;
      })
      .await;
    });

    Self {
      base: format!("http://127.0.0.1:{port}"),
      token,
      state,
      client: Client::builder().build().expect("client"),
      _temp: temp,
      shutdown: Some(shutdown_tx),
      submitted: Some(submit_rx),
    }
  }

  fn url(&self, path: &str) -> String {
    format!("{}{path}", self.base)
  }

  /// A request with the API token header and, on mutations, the JSON
  /// content-type the server insists on — body or not, as the UI sends it.
  async fn api(&self, method: Method, path: &str, body: Option<Value>) -> Response {
    let is_mutation = method != Method::GET;
    let mut request = self.client.request(method, self.url(path)).header(TOKEN_HEADER, &self.token);
    if is_mutation {
      request = request.header(CONTENT_TYPE, "application/json");
    }
    if let Some(body) = body {
      request = request.body(body.to_string());
    }
    request.send().await.expect("send")
  }

  async fn get(&self, path: &str) -> Response {
    self.client.get(self.url(path)).send().await.expect("send")
  }

  async fn status_with_host(&self, host: &str) -> StatusCode {
    self
      .client
      .get(self.url("/api/review"))
      .header(TOKEN_HEADER, &self.token)
      .header(HOST, host)
      .send()
      .await
      .expect("send")
      .status()
  }

  fn state(&self) -> ReviewState {
    self.state.session.state().expect("state")
  }

  async fn create_comment(&self, body: &str) -> Value {
    let response = self
      .api(
        Method::POST,
        "/api/comments",
        Some(json!({
          "anchor": anchor(), "verdict": "ok", "body": body,
        })),
      )
      .await;
    assert_eq!(response.status(), 201);
    response.json().await.expect("json")
  }
}

impl Drop for Harness {
  fn drop(&mut self) {
    if let Some(shutdown) = self.shutdown.take() {
      let _ = shutdown.send(());
    }
  }
}

#[tokio::test]
async fn serves_the_page_only_with_a_valid_token() {
  let app = Harness::start(true).await;
  assert_eq!(app.get("/").await.status(), 403);
  assert_eq!(app.get("/?t=wrong").await.status(), 403);
  assert_eq!(app.get(&format!("/?t={}", app.token)).await.status(), 200);
}

#[tokio::test]
async fn serves_the_favicon_at_the_root_without_a_token() {
  let app = Harness::start(true).await;
  let response = app.get("/favicon.ico").await;
  assert_eq!(response.status(), 200);
  assert_eq!(response.headers()[CONTENT_TYPE], "image/x-icon");
  assert!(!response.bytes().await.expect("body").is_empty());
}

#[tokio::test]
async fn the_page_hands_out_a_cookie_for_its_own_assets() {
  let app = Harness::start(true).await;
  let page = app.get(&format!("/?t={}", app.token)).await;
  let cookie = page.headers()[reqwest::header::SET_COOKIE].to_str().expect("cookie").to_string();
  assert!(cookie.contains("diffpane_token="), "{cookie}");
  assert!(cookie.contains("SameSite=Strict"), "{cookie}");

  assert_eq!(app.get("/assets/app.css").await.status(), 403);
  let with_cookie = app
    .client
    .get(app.url("/assets/app.css"))
    .header(COOKIE, format!("diffpane_token={}", app.token))
    .send()
    .await
    .expect("send");
  assert_eq!(with_cookie.status(), 200);
  assert!(with_cookie.headers()[CONTENT_TYPE].to_str().expect("type").contains("text/css"));
}

#[tokio::test]
async fn the_asset_cookie_does_not_authorise_the_api() {
  // Cookies ride along automatically; the API must stay header-only.
  let app = Harness::start(true).await;
  let response = app
    .client
    .get(app.url("/api/review"))
    .header(COOKIE, format!("diffpane_token={}", app.token))
    .send()
    .await
    .expect("send");
  assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn rejects_api_calls_without_the_token_header() {
  // Without this, any site the user visits could drive the review.
  let app = Harness::start(true).await;
  assert_eq!(app.get("/api/review").await.status(), 403);
  assert_eq!(app.get(&format!("/api/review?t={}", app.token)).await.status(), 403);
}

#[tokio::test]
async fn rejects_a_non_loopback_host_header() {
  // Guards against DNS rebinding.
  let app = Harness::start(true).await;
  assert_eq!(app.status_with_host("evil.example.com").await, 403);
}

#[tokio::test]
async fn rejects_a_host_of_localhost_which_is_a_name() {
  let app = Harness::start(true).await;
  let port = app.base.rsplit(':').next().expect("port").to_string();
  assert_eq!(app.status_with_host("localhost").await, 403);
  assert_eq!(app.status_with_host(&format!("localhost:{port}")).await, 403);
}

#[tokio::test]
async fn accepts_loopback_literals_with_and_without_a_port_or_brackets() {
  let app = Harness::start(true).await;
  let port = app.base.rsplit(':').next().expect("port").to_string();
  assert_eq!(app.status_with_host("127.0.0.1").await, 200);
  assert_eq!(app.status_with_host(&format!("127.0.0.1:{port}")).await, 200);
  assert_eq!(app.status_with_host("::1").await, 200);
  assert_eq!(app.status_with_host(&format!("[::1]:{port}")).await, 200);
}

#[tokio::test]
async fn rejects_mutations_that_are_not_json() {
  let app = Harness::start(true).await;
  let response = app
    .client
    .post(app.url("/api/comments"))
    .header(TOKEN_HEADER, &app.token)
    .header(CONTENT_TYPE, "text/plain")
    .body(json!({ "anchor": anchor(), "verdict": "fix", "body": "x" }).to_string())
    .send()
    .await
    .expect("send");
  assert_eq!(response.status(), 415);
}

#[tokio::test]
async fn rejects_a_content_type_that_only_mentions_json_in_a_parameter() {
  // `text/plain; x=application/json` is a CORS-simple type, so a substring
  // match here reopened the very hole the check exists to close.
  let app = Harness::start(true).await;
  let response = app
    .client
    .post(app.url("/api/comments"))
    .header(TOKEN_HEADER, &app.token)
    .header(CONTENT_TYPE, "text/plain; x=application/json")
    .body(json!({ "anchor": anchor(), "verdict": "fix", "body": "x" }).to_string())
    .send()
    .await
    .expect("send");
  assert_eq!(response.status(), 415);
}

#[tokio::test]
async fn accepts_application_json_with_parameters_and_odd_casing() {
  let app = Harness::start(true).await;
  let response = app
    .client
    .post(app.url("/api/comments"))
    .header(TOKEN_HEADER, &app.token)
    .header(CONTENT_TYPE, "Application/JSON; charset=utf-8")
    .body(json!({ "anchor": anchor(), "verdict": "ok", "body": "fine" }).to_string())
    .send()
    .await
    .expect("send");
  assert_eq!(response.status(), 201);
}

#[tokio::test]
async fn refuses_to_serve_files_outside_the_ui_directory() {
  let app = Harness::start(true).await;
  let response = app.get(&format!("/assets/..%2F..%2Fetc%2Fpasswd?t={}", app.token)).await;
  assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn returns_the_full_payload() {
  let app = Harness::start(true).await;
  let response = app.api(Method::GET, "/api/review", None).await;
  assert_eq!(response.status(), 200);
  let payload: Value = response.json().await.expect("json");
  assert_eq!(payload["meta"]["title"], "Demo");
  assert_eq!(payload["comments"]["comments"], json!([]));
  assert_eq!(payload["review"]["chapters"][0]["id"], "c1");
}

#[tokio::test]
async fn creates_edits_resolves_and_deletes_a_comment() {
  let app = Harness::start(true).await;
  let created = app
    .api(
      Method::POST,
      "/api/comments",
      Some(json!({
        "anchor": anchor(), "verdict": "fix", "body": "needs a test",
      })),
    )
    .await;
  assert_eq!(created.status(), 201);
  let comment: Value = created.json().await.expect("json");
  assert_eq!(comment["verdict"], "fix");
  assert_eq!(app.state().comments.len(), 1);
  let id = comment["id"].as_str().expect("id").to_string();

  let patched = app
    .api(
      Method::PATCH,
      &format!("/api/comments/{id}"),
      Some(json!({ "resolved": true, "body": "needs a test, ideally" })),
    )
    .await;
  assert_eq!(patched.status(), 200);
  let stored = app.state();
  assert!(stored.comments[0].resolved);
  assert_eq!(stored.comments[0].body, "needs a test, ideally");

  let deleted = app.api(Method::DELETE, &format!("/api/comments/{id}"), None).await;
  assert_eq!(deleted.status(), 200);
  assert!(app.state().comments.is_empty());
}

#[tokio::test]
async fn rejects_invalid_comment_payloads() {
  let app = Harness::start(true).await;
  let bad = app
    .api(
      Method::POST,
      "/api/comments",
      Some(json!({
        "anchor": anchor(), "verdict": "lgtm", "body": "x",
      })),
    )
    .await;
  assert_eq!(bad.status(), 400);

  let empty = app
    .api(
      Method::POST,
      "/api/comments",
      Some(json!({
        "anchor": anchor(), "verdict": "ok", "body": "   ",
      })),
    )
    .await;
  assert_eq!(empty.status(), 400);

  let not_an_object = app
    .client
    .post(app.url("/api/comments"))
    .header(TOKEN_HEADER, &app.token)
    .header(CONTENT_TYPE, "application/json")
    .body("[1, 2]")
    .send()
    .await
    .expect("send");
  assert_eq!(not_an_object.status(), 400);
}

#[tokio::test]
async fn rejects_non_string_anchor_fields_and_bad_line_numbers() {
  let app = Harness::start(true).await;
  let object_file = app
    .api(
      Method::POST,
      "/api/comments",
      Some(json!({
        "anchor": { "kind": "file", "file": { "a": 1 } }, "verdict": "ok", "body": "x",
      })),
    )
    .await;
  assert_eq!(object_file.status(), 400);

  let zero_line = app
    .api(
      Method::POST,
      "/api/comments",
      Some(json!({
        "anchor": { "kind": "line", "file": "a.ts", "hunk": "f0h0", "side": "new", "line": 0 },
        "verdict": "ok",
        "body": "x",
      })),
    )
    .await;
  assert_eq!(zero_line.status(), 400);
}

#[tokio::test]
async fn rejects_a_non_boolean_resolved_value() {
  let app = Harness::start(true).await;
  let comment = app.create_comment("x").await;
  let id = comment["id"].as_str().expect("id").to_string();
  let response = app
    .api(Method::PATCH, &format!("/api/comments/{id}"), Some(json!({ "resolved": "false" })))
    .await;
  assert_eq!(response.status(), 400);
  assert!(!app.state().comments[0].resolved);
}

#[tokio::test]
async fn returns_404_on_an_unknown_comment_id() {
  let app = Harness::start(true).await;
  let response =
    app.api(Method::PATCH, "/api/comments/c-nope", Some(json!({ "resolved": true }))).await;
  assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn validates_progress_state() {
  let app = Harness::start(true).await;
  let good = app
    .api(Method::PUT, "/api/progress", Some(json!({ "chapter": "c1", "state": "reviewed" })))
    .await;
  assert_eq!(good.status(), 200);
  assert_eq!(app.state().progress["c1"], ProgressState::Reviewed);

  let bad =
    app.api(Method::PUT, "/api/progress", Some(json!({ "chapter": "c1", "state": "maybe" }))).await;
  assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn rejects_progress_for_a_chapter_that_is_not_in_review_json() {
  let app = Harness::start(true).await;
  let response = app
    .api(Method::PUT, "/api/progress", Some(json!({ "chapter": "c-nope", "state": "reviewed" })))
    .await;
  assert_eq!(response.status(), 400);
  assert!(!app.state().progress.contains_key("c-nope"));
}

#[tokio::test]
async fn accepts_progress_for_the_synthetic_unsorted_chapter() {
  let app = Harness::start(true).await;
  let response = app
    .api(Method::PUT, "/api/progress", Some(json!({ "chapter": "unsorted", "state": "reviewed" })))
    .await;
  assert_eq!(response.status(), 200);
  assert_eq!(app.state().progress["unsorted"], ProgressState::Reviewed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_mutations_all_survive() {
  // Twelve parallel creates on the real runtime once persisted eight: every
  // handler read the state before the last one saved. The contract tells the UI
  // a 2xx is durable, so a lost comment is a lie, not a race it can retry.
  let app = Harness::start(true).await;
  let mut posts = Vec::new();
  for index in 0..12 {
    let client = app.client.clone();
    let url = app.url("/api/comments");
    let token = app.token.clone();
    posts.push(tokio::spawn(async move {
      client
        .post(url)
        .header(TOKEN_HEADER, token)
        .header(CONTENT_TYPE, "application/json")
        .body(
          json!({
            "anchor": { "kind": "file", "file": "a.ts" },
            "verdict": "fix",
            "body": format!("comment {index}"),
          })
          .to_string(),
        )
        .send()
        .await
        .expect("send")
        .status()
    }));
  }
  for post in posts {
    assert_eq!(post.await.expect("join"), 201);
  }
  assert_eq!(app.state().comments.len(), 12);
}

#[tokio::test]
async fn stores_the_overall_verdict() {
  let app = Harness::start(true).await;
  let response = app
    .api(Method::PUT, "/api/overall", Some(json!({ "verdict": "fix", "body": "  one blocker  " })))
    .await;
  assert_eq!(response.status(), 200);
  let stored = app.state();
  assert_eq!(stored.overall.verdict, Some(Verdict::Fix));
  assert_eq!(stored.overall.body, "one blocker");

  let cleared = app.api(Method::PUT, "/api/overall", Some(json!({ "verdict": null }))).await;
  assert_eq!(cleared.status(), 200);
  assert_eq!(app.state().overall.verdict, None);
}

#[tokio::test]
async fn submitting_persists_the_verdict_and_signals_once() {
  let mut app = Harness::start(true).await;
  let response = app
    .api(
      Method::POST,
      "/api/submit",
      Some(json!({
        "overall": { "verdict": "fix", "body": "one blocker" },
      })),
    )
    .await;
  assert_eq!(response.status(), 200);
  let stored = app.state();
  assert!(stored.submitted);
  assert!(stored.submitted_at.is_some());
  assert_eq!(stored.overall.verdict, Some(Verdict::Fix));
  assert_eq!(stored.overall.body, "one blocker");

  let submitted = app.submitted.as_mut().expect("receiver");
  assert!(submitted.try_recv().is_ok());
  assert!(submitted.try_recv().is_err(), "one signal per submit");
}

#[tokio::test]
async fn the_submit_response_survives_the_shutdown_it_triggers() {
  // Regression #7: firing the teardown before the response flushed destroyed
  // the socket and the client saw an empty reply every time. Here the signal
  // starts a *graceful* shutdown, which waits for this response.
  let temp = tempfile::tempdir().expect("temp dir");
  let session = Session::new(temp.path().to_path_buf());
  write_json(&session.meta_path(), &meta()).expect("meta");
  write_json(&session.hunks_path(), &Hunks::default()).expect("hunks");

  let token = generate_token();
  let (submit_tx, mut submit_rx) = mpsc::channel(1);
  let state = Arc::new(AppState::new(session, token.clone(), assets(), submit_tx));
  let listener = bind(0).await.expect("bind");
  let port = listener.local_addr().expect("addr").port();
  let served = tokio::spawn(serve(listener, state, async move {
    let _ = submit_rx.recv().await;
  }));

  let response = Client::new()
    .post(format!("http://127.0.0.1:{port}/api/submit"))
    .header(TOKEN_HEADER, &token)
    .header(CONTENT_TYPE, "application/json")
    .body(json!({ "overall": { "verdict": "ok", "body": "fine" } }).to_string())
    .send()
    .await
    .expect("send");
  assert_eq!(response.status(), 200);
  let payload: Value = response.json().await.expect("json");
  assert_eq!(payload["submitted"], json!(true));

  served.await.expect("join").expect("serve");
}

#[tokio::test]
async fn returns_404_on_unknown_endpoints_and_static_paths() {
  let app = Harness::start(true).await;
  assert_eq!(app.api(Method::GET, "/api/nope", None).await.status(), 404);
  assert_eq!(app.get(&format!("/app.js?t={}", app.token)).await.status(), 404);
  assert_eq!(app.get(&format!("/assets/?t={}", app.token)).await.status(), 404);
}

#[tokio::test]
async fn answers_unknown_api_routes_without_leaking_them() {
  let app = Harness::start(true).await;
  // Unauthenticated: the same 403 as any other API call, so an unknown path
  // cannot be told apart from a known one.
  assert_eq!(app.get("/api/nope").await.status(), 403);
  let wrong_method = app.client.post(app.url("/api/review")).send().await.expect("send");
  assert_eq!(wrong_method.status(), 403);

  // Authenticated: a 404 in the contract's error shape, hardened like the rest.
  let known_path = app.api(Method::DELETE, "/api/progress", None).await;
  assert_eq!(known_path.status(), 404);
  assert_eq!(known_path.headers()["cache-control"], "no-store");
  let payload: Value = known_path.json().await.expect("json");
  assert_eq!(payload["error"], "no such endpoint");
}

#[tokio::test]
async fn serves_assets_with_a_query_token() {
  // The contract says query *or* cookie; the cookie path has its own test.
  let app = Harness::start(true).await;
  let response = app.get(&format!("/assets/app.js?t={}", app.token)).await;
  assert_eq!(response.status(), 200);
  assert!(response.headers()[CONTENT_TYPE].to_str().expect("type").contains("javascript"));
}

#[tokio::test]
async fn hardens_the_response_headers() {
  let app = Harness::start(true).await;
  let response = app.api(Method::GET, "/api/state", None).await;
  assert_eq!(response.headers()["cache-control"], "no-store");
  assert_eq!(response.headers()["referrer-policy"], "no-referrer");
  assert_eq!(response.headers()["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn reports_a_missing_session_rather_than_panicking() {
  let app = Harness::start(false).await;
  std::fs::remove_file(app.state.session.meta_path()).expect("remove meta");
  let response = app.api(Method::GET, "/api/review", None).await;
  assert_eq!(response.status(), 500);
  let payload: Value = response.json().await.expect("json");
  assert!(payload["error"].as_str().expect("error").contains("no meta.json"));
}

#[tokio::test]
async fn walks_forward_when_the_preferred_port_is_taken() {
  let first = bind(0).await.expect("bind");
  let port = first.local_addr().expect("addr").port();
  let second = bind(port).await.expect("bind");
  assert_ne!(second.local_addr().expect("addr").port(), port);
}

#[tokio::test]
async fn walks_forward_from_the_top_of_the_port_range() {
  // `preferred + PORT_ATTEMPTS` overflowed u16 here, which panics in a debug
  // build — reachable with `--port 65535` when the port is busy.
  let held = bind(65535).await.expect("bind");
  assert_eq!(held.local_addr().expect("addr").port(), 65535);
  assert!(bind(65535).await.is_err(), "nowhere left to walk to");
}

#[test]
fn strips_ports_from_host_headers() {
  assert_eq!(host_name("127.0.0.1:8080"), "127.0.0.1");
  assert_eq!(host_name("127.0.0.1"), "127.0.0.1");
  assert_eq!(host_name("[::1]:8080"), "::1");
  assert_eq!(host_name("::1"), "::1");
  assert_eq!(host_name("evil.example.com:80"), "evil.example.com");
  assert_eq!(host_name("[bad"), "");
}

#[test]
fn compares_tokens_without_leaking_a_prefix() {
  let token = generate_token();
  assert_eq!(token.len(), 32);
  assert!(safe_equal(&token, &token.clone()));
  assert!(!safe_equal(&token, &token[..31]));
  assert!(!safe_equal("", &token));
}

#[test]
fn types_assets_by_extension() {
  assert_eq!(content_type_for(Path::new("app.css")), "text/css; charset=utf-8");
  assert_eq!(content_type_for(Path::new("app.js")), "text/javascript; charset=utf-8");
  assert_eq!(content_type_for(Path::new("logo.png")), "image/png");
  assert_eq!(content_type_for(Path::new("weird.bin")), "application/octet-stream");
}

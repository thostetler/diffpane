import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { request as httpRequest, type Server } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, test } from 'node:test';

import { Session, writeJson } from './session.ts';
import { buildServer, generateToken, listen } from './server.ts';
import type { Comment, Meta, Review, ReviewState } from './types.ts';

const TOKEN = generateToken();

let dir: string;
let session: Session;
let server: Server;
let base: string;
let submitCount = 0;

const META: Meta = {
  repo: 'demo',
  repo_root: '/tmp/demo',
  slug: 'demo',
  title: 'Demo',
  scope: 'branch',
  base: 'main',
  head: 'feature',
  diff_cmd: 'git diff main...HEAD',
  generated_at: '2026-01-01T00:00:00Z',
  totals: { files: 0, additions: 0, deletions: 0 },
};

const REVIEW: Review = { chapters: [{ id: 'c1', title: 'Cache layer', hunks: ['f0h0'] }] };

const ANCHOR = { kind: 'line', file: 'a.ts', hunk: 'f0h0', side: 'new', line: 1 };

before(async () => {
  dir = mkdtempSync(join(tmpdir(), 'diffpane-'));
  session = new Session(dir);
  writeJson(session.metaPath, META);
  writeJson(session.hunksPath, { files: [] });
  writeJson(session.reviewPath, REVIEW);
  server = buildServer({
    session,
    token: TOKEN,
    onSubmit: () => {
      submitCount += 1;
    },
  });
  base = `http://127.0.0.1:${await listen(server, 0)}`;
});

after(() => {
  server.close();
  server.closeAllConnections();
  rmSync(dir, { recursive: true, force: true });
});

function api(path: string, init: RequestInit = {}): Promise<Response> {
  const method = init.method ?? 'GET';
  return fetch(`${base}${path}`, {
    ...init,
    method,
    headers: {
      'X-Diffpane-Token': TOKEN,
      ...(method === 'GET' ? {} : { 'Content-Type': 'application/json' }),
      ...init.headers,
    },
  });
}

test('serves the page only with a valid token', async () => {
  assert.equal((await fetch(`${base}/`)).status, 403);
  assert.equal((await fetch(`${base}/?t=wrong`)).status, 403);
  assert.equal((await fetch(`${base}/?t=${TOKEN}`)).status, 200);
});

test('serves the favicon at the root without a token', async () => {
  const response = await fetch(`${base}/favicon.ico`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-type'), 'image/x-icon');
  assert.ok((await response.arrayBuffer()).byteLength > 0);
});

test('the page hands out a cookie so the browser can fetch its own assets', async () => {
  const page = await fetch(`${base}/?t=${TOKEN}`);
  const cookie = page.headers.get('set-cookie') ?? '';
  assert.match(cookie, /diffpane_token=/);
  assert.match(cookie, /SameSite=Strict/);

  assert.equal((await fetch(`${base}/assets/app.css`)).status, 403);
  const withCookie = await fetch(`${base}/assets/app.css`, {
    headers: { Cookie: `diffpane_token=${TOKEN}` },
  });
  assert.equal(withCookie.status, 200);
  assert.match(withCookie.headers.get('content-type') ?? '', /text\/css/);
});

test('the asset cookie does not authorise the API', async () => {
  // Cookies ride along automatically; the API must stay header-only.
  const response = await fetch(`${base}/api/review`, {
    headers: { Cookie: `diffpane_token=${TOKEN}` },
  });
  assert.equal(response.status, 403);
});

test('rejects API calls without the token header', async () => {
  // Without this, any site the user visits could drive the review.
  const response = await fetch(`${base}/api/review`);
  assert.equal(response.status, 403);
});

test('rejects a token supplied only as a query parameter on the API', async () => {
  assert.equal((await fetch(`${base}/api/review?t=${TOKEN}`)).status, 403);
});

/** fetch() forbids setting Host, so Host-header cases go through raw http. */
function statusWithHost(host: string): Promise<number> {
  return new Promise<number>((resolvePromise, reject) => {
    const request = httpRequest(
      {
        host: '127.0.0.1',
        port: Number(new URL(base).port),
        path: '/api/review',
        headers: { 'X-Diffpane-Token': TOKEN, Host: host },
      },
      (response) => {
        response.resume();
        resolvePromise(response.statusCode ?? 0);
      },
    );
    request.on('error', reject);
    request.end();
  });
}

test('rejects requests with a non-loopback Host header', async () => {
  // Guards against DNS rebinding.
  assert.equal(await statusWithHost('evil.example.com'), 403);
});

test('rejects a Host of localhost, which is a name and not a literal', async () => {
  assert.equal(await statusWithHost('localhost'), 403);
  assert.equal(await statusWithHost(`localhost:${new URL(base).port}`), 403);
});

test('accepts loopback literals with and without a port or brackets', async () => {
  assert.equal(await statusWithHost('127.0.0.1'), 200);
  assert.equal(await statusWithHost(`127.0.0.1:${new URL(base).port}`), 200);
  assert.equal(await statusWithHost('::1'), 200);
  assert.equal(await statusWithHost(`[::1]:${new URL(base).port}`), 200);
});

test('rejects mutations that are not JSON', async () => {
  const response = await fetch(`${base}/api/comments`, {
    method: 'POST',
    headers: { 'X-Diffpane-Token': TOKEN, 'Content-Type': 'text/plain' },
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'fix', body: 'x' }),
  });
  assert.equal(response.status, 415);
});

test('rejects a content type that only mentions JSON in a parameter', async () => {
  // `text/plain; x=application/json` is a CORS-simple type, so a substring
  // match here reopened the very hole the check exists to close.
  const response = await fetch(`${base}/api/comments`, {
    method: 'POST',
    headers: {
      'X-Diffpane-Token': TOKEN,
      'Content-Type': 'text/plain; x=application/json',
    },
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'fix', body: 'x' }),
  });
  assert.equal(response.status, 415);
});

test('accepts application/json with parameters and odd casing', async () => {
  const response = await api('/api/comments', {
    method: 'POST',
    headers: { 'Content-Type': 'Application/JSON; charset=utf-8' },
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'ok', body: 'fine' }),
  });
  assert.equal(response.status, 201);
  const comment = (await response.json()) as Comment;
  await api(`/api/comments/${comment.id}`, { method: 'DELETE' });
});

test('refuses to serve files outside the ui directory', async () => {
  const response = await fetch(`${base}/assets/../../../etc/passwd?t=${TOKEN}`);
  assert.ok(response.status === 404 || response.status === 403, `got ${response.status}`);
});

test('returns the full payload', async () => {
  const response = await api('/api/review');
  assert.equal(response.status, 200);
  const payload = (await response.json()) as { meta: Meta; comments: ReviewState };
  assert.equal(payload.meta.title, 'Demo');
  assert.deepEqual(payload.comments.comments, []);
});

test('creates, edits, resolves and deletes a comment', async () => {
  const created = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'fix', body: 'needs a test' }),
  });
  assert.equal(created.status, 201);
  const comment = (await created.json()) as Comment;
  assert.equal(comment.verdict, 'fix');
  assert.equal(session.state().comments.length, 1);

  const patched = await api(`/api/comments/${comment.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ resolved: true, body: 'needs a test, ideally' }),
  });
  assert.equal(patched.status, 200);
  assert.equal(session.state().comments[0]?.resolved, true);
  assert.equal(session.state().comments[0]?.body, 'needs a test, ideally');

  const deleted = await api(`/api/comments/${comment.id}`, { method: 'DELETE' });
  assert.equal(deleted.status, 200);
  assert.equal(session.state().comments.length, 0);
});

test('rejects invalid comment payloads', async () => {
  const bad = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'lgtm', body: 'x' }),
  });
  assert.equal(bad.status, 400);

  const empty = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'ok', body: '   ' }),
  });
  assert.equal(empty.status, 400);
});

test('404s on an unknown comment id', async () => {
  const response = await api('/api/comments/c-nope', {
    method: 'PATCH',
    body: JSON.stringify({ resolved: true }),
  });
  assert.equal(response.status, 404);
});

test('validates progress state', async () => {
  const ok = await api('/api/progress', {
    method: 'PUT',
    body: JSON.stringify({ chapter: 'c1', state: 'reviewed' }),
  });
  assert.equal(ok.status, 200);
  assert.equal(session.state().progress['c1'], 'reviewed');

  const bad = await api('/api/progress', {
    method: 'PUT',
    body: JSON.stringify({ chapter: 'c1', state: 'maybe' }),
  });
  assert.equal(bad.status, 400);
});

test('rejects progress for a chapter that is not in review.json', async () => {
  const response = await api('/api/progress', {
    method: 'PUT',
    body: JSON.stringify({ chapter: 'c-nope', state: 'reviewed' }),
  });
  assert.equal(response.status, 400);
  assert.equal(session.state().progress['c-nope'], undefined);
});

test('accepts progress for the synthetic unsorted chapter', async () => {
  const response = await api('/api/progress', {
    method: 'PUT',
    body: JSON.stringify({ chapter: 'unsorted', state: 'reviewed' }),
  });
  assert.equal(response.status, 200);
  assert.equal(session.state().progress['unsorted'], 'reviewed');
});

test('submitting persists the verdict and fires the callback once', async () => {
  const before = submitCount;
  const response = await api('/api/submit', {
    method: 'POST',
    body: JSON.stringify({ overall: { verdict: 'fix', body: 'one blocker' } }),
  });
  assert.equal(response.status, 200);
  const state = session.state();
  assert.equal(state.submitted, true);
  assert.equal(state.overall.verdict, 'fix');
  assert.equal(state.overall.body, 'one blocker');
  assert.equal(submitCount, before + 1);
});

test('404s on an unknown endpoint', async () => {
  assert.equal((await api('/api/nope')).status, 404);
});

test('the submit response reaches the client even though onSubmit closes the server', async () => {
  // onSubmit tears the server down. Firing it before the response flushed
  // destroyed the socket and the client saw an empty reply every time.
  const dir2 = mkdtempSync(join(tmpdir(), 'diffpane-submit-'));
  const session2 = new Session(dir2);
  writeJson(session2.metaPath, META);
  writeJson(session2.hunksPath, { files: [] });
  let closed = false;
  const server2 = buildServer({
    session: session2,
    token: TOKEN,
    onSubmit: () => {
      closed = true;
      server2.close();
      server2.closeAllConnections();
    },
  });
  const port2 = await listen(server2, 0);
  try {
    const response = await fetch(`http://127.0.0.1:${port2}/api/submit`, {
      method: 'POST',
      headers: { 'X-Diffpane-Token': TOKEN, 'Content-Type': 'application/json' },
      body: JSON.stringify({ overall: { verdict: 'ok', body: 'fine' } }),
    });
    assert.equal(response.status, 200);
    const payload = (await response.json()) as { submitted: boolean };
    assert.equal(payload.submitted, true);
    assert.equal(closed, true);
  } finally {
    server2.close();
    server2.closeAllConnections();
    rmSync(dir2, { recursive: true, force: true });
  }
});

test('rejects a non-boolean resolved value', async () => {
  const created = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: ANCHOR, verdict: 'ok', body: 'x' }),
  });
  const comment = (await created.json()) as Comment;
  const response = await api(`/api/comments/${comment.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ resolved: 'false' }),
  });
  assert.equal(response.status, 400);
  assert.equal(session.state().comments.find((c) => c.id === comment.id)?.resolved, false);
  await api(`/api/comments/${comment.id}`, { method: 'DELETE' });
});

test('rejects non-string anchor fields', async () => {
  const response = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: { kind: 'file', file: { a: 1 } }, verdict: 'ok', body: 'x' }),
  });
  assert.equal(response.status, 400);
});

test('rejects a non-positive line number', async () => {
  const response = await api('/api/comments', {
    method: 'POST',
    body: JSON.stringify({ anchor: { ...ANCHOR, line: 0 }, verdict: 'ok', body: 'x' }),
  });
  assert.equal(response.status, 400);
});

test('404s on a static path outside /assets/', async () => {
  assert.equal((await fetch(`${base}/app.js?t=${TOKEN}`)).status, 404);
  assert.equal((await fetch(`${base}/assets/?t=${TOKEN}`)).status, 404);
});

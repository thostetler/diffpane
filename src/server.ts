import { randomBytes, timingSafeEqual } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { extname, normalize, resolve, sep } from 'node:path';

import type { Session } from './session.ts';
import { nowIso } from './session.ts';
import type { Comment, ReviewState } from './types.ts';
import {
  ApiError, validateAnchor, validateBody, validateProgressState, validateResolved, validateVerdict,
} from './validate.ts';

const MAX_BODY_BYTES = 1024 * 1024;
const COOKIE_NAME = 'diffpane_token';
const UI_DIR = resolve(import.meta.dirname, '..', 'ui');

const CONTENT_TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

export interface ServerOptions {
  session: Session;
  token: string;
  onSubmit: () => void;
}

interface RouteResult {
  payload: unknown;
  status: number;
  afterSend?: () => void;
}

function ok(payload: unknown): RouteResult {
  return { payload, status: 200 };
}

export function generateToken(): string {
  return randomBytes(16).toString('hex');
}

function safeEqual(a: string, b: string): boolean {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

/** Rejects DNS-rebinding: the browser must be talking to a loopback literal. */
function isLoopbackHost(host: string | undefined): boolean {
  if (host === undefined) return false;
  const name = host.replace(/:\d+$/, '').replace(/^\[|\]$/g, '');
  return name === '127.0.0.1' || name === 'localhost' || name === '::1';
}

async function readRequestBody(request: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    size += (chunk as Buffer).length;
    if (size > MAX_BODY_BYTES) throw new ApiError('request body too large', 413);
    chunks.push(chunk as Buffer);
  }
  if (size === 0) return {};
  const text = Buffer.concat(chunks).toString('utf8');
  try {
    const parsed: unknown = JSON.parse(text);
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      throw new ApiError('body must be a JSON object');
    }
    return parsed as Record<string, unknown>;
  } catch (error) {
    if (error instanceof ApiError) throw error;
    throw new ApiError(`invalid JSON: ${(error as Error).message}`);
  }
}

class RequestHandler {
  private readonly options: ServerOptions;

  constructor(options: ServerOptions) {
    this.options = options;
  }

  async handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
    try {
      if (!isLoopbackHost(request.headers.host)) throw new ApiError('forbidden host', 403);
      const url = new URL(request.url ?? '/', 'http://127.0.0.1');
      if (url.pathname.startsWith('/api/')) {
        await this.handleApi(request, response, url);
        return;
      }
      this.handleStatic(request, response, url);
    } catch (error) {
      const status = error instanceof ApiError ? error.status : 500;
      const message = error instanceof Error ? error.message : 'unknown error';
      sendJson(response, { error: message }, status);
    }
  }

  private handleStatic(request: IncomingMessage, response: ServerResponse, url: URL): void {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      throw new ApiError('method not allowed', 405);
    }
    // Served before the token check, as an empty 204 was: the browser asks for
    // this at the root with no cookie, and an icon is not the diff.
    if (url.pathname === '/favicon.ico') {
      send(response, 200, readFileSync(resolve(UI_DIR, 'favicon.ico')), 'image/x-icon');
      return;
    }
    // The page is gated on the token so a stray tab cannot read the diff. The
    // browser fetches app.css and app.js on its own, without a query string, so
    // the page load hands out a cookie for those follow-up requests.
    const isPage = url.pathname === '/';
    const supplied = isPage
      ? (url.searchParams.get('t') ?? '')
      : (url.searchParams.get('t') ?? readCookie(request.headers.cookie, COOKIE_NAME));
    if (!safeEqual(supplied, this.options.token)) {
      throw new ApiError('missing or invalid token', 403);
    }
    if (!isPage && !url.pathname.startsWith('/assets/')) throw new ApiError('not found', 404);
    const name = isPage ? 'index.html' : url.pathname.slice('/assets/'.length);
    if (name === '') throw new ApiError('not found', 404);
    const target = resolve(UI_DIR, normalize(name).replace(/^(\.\.[/\\])+/, ''));
    const stats = statSync(target, { throwIfNoEntry: false });
    if (!target.startsWith(UI_DIR + sep) || stats?.isFile() !== true) {
      throw new ApiError('not found', 404);
    }
    const cookie = `${COOKIE_NAME}=${this.options.token}; Path=/; SameSite=Strict; Max-Age=86400`;
    const extra: Record<string, string> = isPage ? { 'Set-Cookie': cookie } : {};
    send(
      response,
      200,
      readFileSync(target),
      CONTENT_TYPES[extname(target)] ?? 'application/octet-stream',
      extra,
    );
  }

  private async handleApi(
    request: IncomingMessage,
    response: ServerResponse,
    url: URL,
  ): Promise<void> {
    // A custom header cannot be set cross-origin without a preflight, so
    // requiring it is what actually blocks drive-by requests from other tabs.
    if (!safeEqual(String(request.headers['x-diffpane-token'] ?? ''), this.options.token)) {
      throw new ApiError('missing or invalid token', 403);
    }
    const method = request.method ?? 'GET';
    const contentType = String(request.headers['content-type'] ?? '');
    if (method !== 'GET' && !contentType.includes('application/json')) {
      throw new ApiError('content-type must be application/json', 415);
    }
    const result = await this.route(request, method, url.pathname);
    // The submit callback tears the server down, so it must not run until the
    // response has actually flushed — otherwise the client sees a dead socket.
    if (result.afterSend !== undefined) response.once('finish', result.afterSend);
    sendJson(response, result.payload, result.status);
  }

  private async route(
    request: IncomingMessage,
    method: string,
    path: string,
  ): Promise<RouteResult> {
    const { session } = this.options;
    if (method === 'GET' && path === '/api/review') {
      return ok({
        meta: session.meta(),
        hunks: session.hunks(),
        review: session.review(),
        comments: session.state(),
      });
    }
    if (method === 'GET' && path === '/api/state') return ok(session.state());
    if (method === 'POST' && path === '/api/comments') {
      return { payload: this.createComment(await readRequestBody(request)), status: 201 };
    }
    if (path.startsWith('/api/comments/')) {
      const id = decodeURIComponent(path.slice('/api/comments/'.length));
      if (method === 'DELETE') return ok(this.deleteComment(id));
      if (method === 'PATCH') return ok(this.patchComment(id, await readRequestBody(request)));
    }
    if (method === 'PUT' && path === '/api/progress') {
      return ok(this.setProgress(await readRequestBody(request)));
    }
    if (method === 'PUT' && path === '/api/overall') {
      return ok(this.setOverall(await readRequestBody(request)));
    }
    if (method === 'POST' && path === '/api/submit') {
      return {
        payload: this.submit(await readRequestBody(request)),
        status: 200,
        afterSend: this.options.onSubmit,
      };
    }
    throw new ApiError('no such endpoint', 404);
  }

  private mutate<T>(change: (state: ReviewState) => T): T {
    const state = this.options.session.state();
    const result = change(state);
    this.options.session.saveState(state);
    return result;
  }

  private createComment(body: Record<string, unknown>): Comment {
    const stamp = nowIso();
    const comment: Comment = {
      id: `c-${randomBytes(3).toString('hex')}`,
      anchor: validateAnchor(body['anchor']),
      verdict: validateVerdict(body['verdict']),
      body: validateBody(body['body']),
      created_at: stamp,
      updated_at: stamp,
      resolved: false,
    };
    return this.mutate((state) => {
      state.comments.push(comment);
      return comment;
    });
  }

  private patchComment(id: string, body: Record<string, unknown>): Comment {
    return this.mutate((state) => {
      const match = state.comments.find((comment) => comment.id === id);
      if (match === undefined) throw new ApiError(`no such comment: ${id}`, 404);
      if ('verdict' in body) match.verdict = validateVerdict(body['verdict']);
      if ('body' in body) match.body = validateBody(body['body']);
      if ('resolved' in body) match.resolved = validateResolved(body['resolved']);
      match.updated_at = nowIso();
      return match;
    });
  }

  private deleteComment(id: string): { ok: true } {
    return this.mutate((state) => {
      const before = state.comments.length;
      state.comments = state.comments.filter((comment) => comment.id !== id);
      if (state.comments.length === before) throw new ApiError(`no such comment: ${id}`, 404);
      return { ok: true } as const;
    });
  }

  private setProgress(body: Record<string, unknown>): { progress: ReviewState['progress'] } {
    const chapter = body['chapter'];
    if (typeof chapter !== 'string' || chapter === '') throw new ApiError('chapter is required');
    const value = validateProgressState(body['state']);
    return this.mutate((state) => {
      state.progress[chapter] = value;
      return { progress: state.progress };
    });
  }

  private setOverall(body: Record<string, unknown>): { overall: ReviewState['overall'] } {
    const overall = {
      verdict: body['verdict'] === undefined || body['verdict'] === null
        ? null
        : validateVerdict(body['verdict']),
      body: typeof body['body'] === 'string' ? body['body'].trim() : '',
    };
    return this.mutate((state) => {
      state.overall = overall;
      return { overall };
    });
  }

  private submit(body: Record<string, unknown>): { submitted: true; submitted_at: string } {
    const result = this.mutate((state) => {
      if (typeof body['overall'] === 'object' && body['overall'] !== null) {
        this.applyOverall(state, body['overall'] as Record<string, unknown>);
      }
      state.submitted = true;
      state.submitted_at = nowIso();
      return { submitted: true as const, submitted_at: state.submitted_at };
    });
    return result;
  }

  private applyOverall(state: ReviewState, overall: Record<string, unknown>): void {
    state.overall = {
      verdict: overall['verdict'] === undefined || overall['verdict'] === null
        ? null
        : validateVerdict(overall['verdict']),
      body: typeof overall['body'] === 'string' ? overall['body'].trim() : '',
    };
  }
}

function readCookie(header: string | undefined, name: string): string {
  for (const part of (header ?? '').split(';')) {
    const [key, ...rest] = part.trim().split('=');
    if (key === name) return rest.join('=');
  }
  return '';
}

function send(
  response: ServerResponse,
  status: number,
  body: Buffer,
  contentType: string,
  extraHeaders: Record<string, string> = {},
): void {
  response.writeHead(status, {
    'Content-Type': contentType,
    'Content-Length': body.length,
    'Cache-Control': 'no-store',
    'Referrer-Policy': 'no-referrer',
    'X-Content-Type-Options': 'nosniff',
    ...extraHeaders,
  });
  response.end(body);
}

function sendJson(response: ServerResponse, payload: unknown, status = 200): void {
  send(response, status, Buffer.from(JSON.stringify(payload)), 'application/json; charset=utf-8');
}

export function buildServer(options: ServerOptions): Server {
  const handler = new RequestHandler(options);
  return createServer((request, response) => {
    void handler.handle(request, response);
  });
}

export function listen(server: Server, preferredPort: number): Promise<number> {
  return new Promise((resolvePort, reject) => {
    let port = preferredPort;
    const attempt = (): void => {
      const onError = (error: NodeJS.ErrnoException): void => {
        if (error.code === 'EADDRINUSE' && port < preferredPort + 20) {
          port += 1;
          attempt();
          return;
        }
        reject(error);
      };
      server.once('error', onError);
      server.listen(port, '127.0.0.1', () => {
        // Leaving this attached would re-enter listen() on a later error.
        server.removeListener('error', onError);
        // Read it back rather than trusting `port`, so port 0 works too.
        const address = server.address();
        resolvePort(typeof address === 'object' && address !== null ? address.port : port);
      });
    };
    attempt();
  });
}

export { UI_DIR };

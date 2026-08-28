import { createHash, randomBytes } from 'node:crypto';
import {
  existsSync, mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join } from 'node:path';

import type { FileDiff, Meta, Review, ReviewState } from './types.ts';

export function cacheRoot(): string {
  const xdg = process.env['XDG_CACHE_HOME'];
  const base = xdg !== undefined && xdg !== '' ? xdg : join(homedir(), '.cache');
  return join(base, 'diffpane');
}

export function nowIso(): string {
  return new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
}

export function slugify(text: string): string {
  const slug = text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  return slug.slice(0, 48) === '' ? 'review' : slug.slice(0, 48);
}

export function emptyState(): ReviewState {
  return {
    comments: [],
    progress: {},
    overall: { verdict: null, body: '' },
    submitted: false,
    submitted_at: null,
  };
}

/** Atomic so a crash mid-review cannot truncate the comments file. */
export function writeJson(path: string, payload: unknown): void {
  mkdirSync(dirname(path), { recursive: true });
  const temp = `${path}.${randomBytes(4).toString('hex')}.tmp`;
  try {
    writeFileSync(temp, `${JSON.stringify(payload, null, 2)}\n`, 'utf8');
    renameSync(temp, path);
  } catch (error) {
    if (existsSync(temp)) unlinkSync(temp);
    throw error;
  }
}

export function readJson<T>(path: string): T | null {
  if (!existsSync(path)) return null;
  return JSON.parse(readFileSync(path, 'utf8')) as T;
}

export class Session {
  readonly dir: string;

  constructor(dir: string) {
    this.dir = dir;
  }

  static create(repoRootPath: string, slug: string): Session {
    // Two checkouts can share a basename; the path hash keeps them apart.
    const fingerprint = createHash('sha256').update(repoRootPath).digest('hex').slice(0, 8);
    const dir = join(cacheRoot(), `${basename(repoRootPath)}-${fingerprint}`, slug);
    const session = new Session(dir);
    mkdirSync(session.dir, { recursive: true });
    return session;
  }

  get metaPath(): string {
    return join(this.dir, 'meta.json');
  }

  get hunksPath(): string {
    return join(this.dir, 'hunks.json');
  }

  get reviewPath(): string {
    return join(this.dir, 'review.json');
  }

  get statePath(): string {
    return join(this.dir, 'comments.json');
  }

  meta(): Meta {
    const meta = readJson<Meta>(this.metaPath);
    if (meta === null) throw new Error(`session has no meta.json: ${this.dir}`);
    return meta;
  }

  hunks(): { files: FileDiff[] } {
    return readJson<{ files: FileDiff[] }>(this.hunksPath) ?? { files: [] };
  }

  review(): Review | null {
    return readJson<Review>(this.reviewPath);
  }

  state(): ReviewState {
    const state = readJson<ReviewState>(this.statePath);
    if (state !== null) return state;
    const fresh = emptyState();
    writeJson(this.statePath, fresh);
    return fresh;
  }

  saveState(state: ReviewState): void {
    writeJson(this.statePath, state);
  }
}

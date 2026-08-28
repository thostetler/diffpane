import { execFileSync } from 'node:child_process';

import type { FileStatus, Scope } from './types.ts';

const MAX_BUFFER = 512 * 1024 * 1024;

/** git's canonical empty tree, used as the base for a root commit. */
const EMPTY_TREE = '4b825dc642cb6eb9a060e54bf8d69288fbee4904';

const STATUS_LETTERS: Record<string, FileStatus> = {
  A: 'added',
  M: 'modified',
  D: 'deleted',
  R: 'renamed',
  C: 'copied',
  T: 'modified',
};

export interface NumstatEntry {
  additions: number;
  deletions: number;
  binary: boolean;
}

export interface RawEntry {
  status: FileStatus;
  oldPath: string | null;
}

export interface ScopeOptions {
  scope?: Scope;
  base?: string;
  range?: string;
  commit?: string;
  paths?: string[];
}

export interface ResolvedScope {
  scope: Scope;
  diffArgs: string[];
  base: string;
}

export function git(root: string, args: string[]): string {
  return execFileSync('git', ['-c', 'core.quotePath=false', ...args], {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: MAX_BUFFER,
  });
}

/** Returns null instead of throwing, for probing refs that may not exist. */
export function gitQuiet(root: string, args: string[]): string | null {
  try {
    return git(root, args).trim();
  } catch {
    return null;
  }
}

export function repoRoot(cwd: string): string {
  const root = gitQuiet(cwd, ['rev-parse', '--show-toplevel']);
  if (root === null) throw new Error(`not a git repository: ${cwd}`);
  return root;
}

export function currentBranch(root: string): string {
  return gitQuiet(root, ['rev-parse', '--abbrev-ref', 'HEAD']) ?? 'HEAD';
}

export function defaultBase(root: string): string {
  const upstream = gitQuiet(root, ['rev-parse', '--abbrev-ref', '--symbolic-full-name', '@{u}']);
  if (upstream !== null && upstream !== '') return upstream;
  for (const candidate of ['origin/HEAD', 'origin/main', 'origin/master', 'main', 'master']) {
    if (gitQuiet(root, ['rev-parse', '--verify', '--quiet', candidate]) !== null) {
      return gitQuiet(root, ['rev-parse', '--abbrev-ref', candidate]) ?? candidate;
    }
  }
  throw new Error('could not infer a base ref; pass --base');
}

export function resolveScope(root: string, options: ScopeOptions): ResolvedScope {
  const resolved = selectScope(root, options);
  const paths = options.paths ?? [];
  if (paths.length > 0) resolved.diffArgs = [...resolved.diffArgs, '--', ...paths];
  return resolved;
}

function selectScope(root: string, options: ScopeOptions): ResolvedScope {
  if (options.range !== undefined) {
    return { scope: 'range', diffArgs: [options.range], base: options.range };
  }
  if (options.commit !== undefined) {
    // `<sha>^!` on a merge produces a combined diff (`diff --cc`, `@@@`) that no
    // unified-diff parser can read, and it is empty for a clean merge. An
    // explicit first-parent range gives an ordinary diff in both cases.
    const parent = gitQuiet(root, ['rev-parse', '--verify', '--quiet', `${options.commit}^`]);
    return {
      scope: 'commit',
      diffArgs: [parent ?? EMPTY_TREE, options.commit],
      base: options.commit,
    };
  }
  if (options.scope === 'working') return { scope: 'working', diffArgs: [], base: 'working tree' };
  if (options.scope === 'staged') return { scope: 'staged', diffArgs: ['--cached'], base: 'index' };
  const base = options.base ?? defaultBase(root);
  return { scope: 'branch', diffArgs: [`${base}...HEAD`], base };
}

function splitNul(blob: string): string[] {
  const parts = blob.split('\0');
  if (parts.at(-1) === '') parts.pop();
  return parts;
}

/** path -> counts. Renames and copies key on the new path. */
export function readNumstat(root: string, diffArgs: string[]): Map<string, NumstatEntry> {
  const fields = splitNul(git(root, ['diff', ...diffArgs, '--numstat', '-z']));
  const stats = new Map<string, NumstatEntry>();
  let i = 0;
  while (i < fields.length) {
    const [adds = '', dels = '', head = ''] = (fields[i] ?? '').split('\t');
    i += 1;
    let path = head;
    if (path === '') {
      path = fields[i + 1] ?? '';
      i += 2;
    }
    const binary = adds === '-' || dels === '-';
    stats.set(path, {
      additions: binary ? 0 : Number(adds),
      deletions: binary ? 0 : Number(dels),
      binary,
    });
  }
  return stats;
}

/** path -> status. Renames and copies key on the new path. */
export function readRaw(root: string, diffArgs: string[]): Map<string, RawEntry> {
  const fields = splitNul(git(root, ['diff', ...diffArgs, '--raw', '-z']));
  const entries = new Map<string, RawEntry>();
  let i = 0;
  while (i < fields.length) {
    const letter = (fields[i] ?? '').split(' ').at(-1) ?? '';
    i += 1;
    const status = STATUS_LETTERS[letter[0] ?? 'M'] ?? 'modified';
    if (letter.startsWith('R') || letter.startsWith('C')) {
      entries.set(fields[i + 1] ?? '', { status, oldPath: fields[i] ?? null });
      i += 2;
    } else {
      entries.set(fields[i] ?? '', { status, oldPath: null });
      i += 1;
    }
  }
  return entries;
}

export function readPatch(root: string, diffArgs: string[]): string {
  return git(root, ['diff', ...diffArgs, '--no-color', '--unified=3']);
}

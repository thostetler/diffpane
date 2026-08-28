import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, test } from 'node:test';

import { assembleDiff, computeTotals } from './assemble.ts';
import { resolveScope } from './git.ts';

// These run against a real repository on purpose. Hand-written patches are what
// let binary and rename handling ship broken: git does not emit ---/+++ for
// them, but an invented fixture did.
let repo: string;

function git(...args: string[]): string {
  return execFileSync('git', args, { cwd: repo, encoding: 'utf8' });
}

function staged(): ReturnType<typeof assembleDiff> {
  return assembleDiff(repo, resolveScope(repo, { scope: 'staged' }).diffArgs);
}

before(() => {
  repo = mkdtempSync(join(tmpdir(), 'diffpane-git-'));
  git('init', '-q');
  git('config', 'user.email', 'test@example.com');
  git('config', 'user.name', 'Test');
  writeFileSync(join(repo, 'text.txt'), 'hello\n');
  writeFileSync(join(repo, 'img.bin'), Buffer.from([0, 1, 2, 3, 0, 255]));
  writeFileSync(join(repo, 'old.ts'), 'export const a = 1;\n');
  git('add', '-A');
  git('commit', '-qm', 'init');
});

after(() => {
  rmSync(repo, { recursive: true, force: true });
});

test('keeps binary-only changes, which have no ---/+++ headers', () => {
  writeFileSync(join(repo, 'img.bin'), Buffer.from([9, 9, 9, 9, 9, 9]));
  git('add', '-A');
  const files = staged();
  const binary = files.find((file) => file.path === 'img.bin');
  assert.ok(binary, 'binary file was dropped from the diff');
  assert.equal(binary.binary, true);
  assert.equal(binary.status, 'modified');
  assert.deepEqual(binary.hunks, []);
  git('checkout', '--', '.');
  git('reset', '-q');
});

test('keeps a pure rename, which has no hunks at all', () => {
  git('mv', 'old.ts', 'renamed.ts');
  const files = staged();
  const renamed = files.find((file) => file.path === 'renamed.ts');
  assert.ok(renamed, 'rename was dropped from the diff');
  assert.equal(renamed.status, 'renamed');
  assert.equal(renamed.old_path, 'old.ts');
  git('mv', 'renamed.ts', 'old.ts');
  git('reset', '-q');
});

test('a diff of only binary and rename changes is not reported as empty', () => {
  // This exact combination reported "no changes to review" and exited 0,
  // silently approving unreviewed work.
  writeFileSync(join(repo, 'img.bin'), Buffer.from([7, 7, 7]));
  git('mv', 'old.ts', 'moved.ts');
  git('add', '-A');
  const files = staged();
  assert.equal(files.length, 2);
  assert.deepEqual(files.map((file) => file.path).sort(), ['img.bin', 'moved.ts']);
  git('reset', '-q', '--hard', 'HEAD');
});

test('parses ordinary text changes with hunks and totals', () => {
  writeFileSync(join(repo, 'text.txt'), 'hello\nworld\n');
  git('add', '-A');
  const files = staged();
  const text = files.find((file) => file.path === 'text.txt');
  assert.ok(text);
  assert.equal(text.additions, 1);
  assert.equal(text.hunks.length, 1);
  assert.equal(text.hunks[0]?.id, `${text.id}h0`);
  assert.equal(computeTotals(files).additions, 1);
  git('reset', '-q', '--hard', 'HEAD');
});

test('reports added and deleted files with their status', () => {
  writeFileSync(join(repo, 'fresh.ts'), 'export const b = 2;\n');
  execFileSync('git', ['rm', '-q', 'text.txt'], { cwd: repo });
  git('add', '-A');
  const files = staged();
  assert.equal(files.find((file) => file.path === 'fresh.ts')?.status, 'added');
  assert.equal(files.find((file) => file.path === 'text.txt')?.status, 'deleted');
  git('reset', '-q', '--hard', 'HEAD');
});

test('reviews a merge commit via its first parent', () => {
  // `<sha>^!` yields an unreadable combined diff and is empty for a clean merge.
  git('checkout', '-q', '-b', 'side');
  writeFileSync(join(repo, 'side.ts'), 'export const s = 1;\n');
  git('add', '-A');
  git('commit', '-qm', 'side work');
  git('checkout', '-q', '-');
  writeFileSync(join(repo, 'main.ts'), 'export const m = 1;\n');
  git('add', '-A');
  git('commit', '-qm', 'main work');
  git('merge', '-q', '--no-ff', 'side', '-m', 'merge side');

  const sha = git('rev-parse', 'HEAD').trim();
  const files = assembleDiff(repo, resolveScope(repo, { commit: sha }).diffArgs);
  assert.deepEqual(files.map((file) => file.path), ['side.ts']);
});

test('narrows the diff to a pathspec', () => {
  // The pathspec used to be appended to the scope args, so `--raw`, `-z` and
  // `--numstat` reached git as pathspecs and every read came back as a plain
  // patch with no file list at all.
  writeFileSync(join(repo, 'kept.ts'), 'export const k = 1;\n');
  writeFileSync(join(repo, 'ignored.ts'), 'export const i = 1;\n');
  git('add', '-A');
  const { diffArgs, paths } = resolveScope(repo, { scope: 'staged', paths: ['kept.ts'] });
  const files = assembleDiff(repo, diffArgs, paths);
  assert.deepEqual(files.map((file) => file.path), ['kept.ts']);
  assert.equal(files[0]?.additions, 1);
  git('reset', '-q', '--hard', 'HEAD');
});

test('flags lockfiles as noise and detects language', () => {
  writeFileSync(join(repo, 'pnpm-lock.yaml'), 'lockfileVersion: 9\n');
  writeFileSync(join(repo, 'app.ts'), 'export const c = 3;\n');
  git('add', '-A');
  const files = staged();
  assert.equal(files.find((file) => file.path === 'pnpm-lock.yaml')?.noise, true);
  assert.equal(files.find((file) => file.path === 'app.ts')?.language, 'typescript');
  git('reset', '-q', '--hard', 'HEAD');
});

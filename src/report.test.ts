import assert from 'node:assert/strict';
import { test } from 'node:test';

import { buildJson, buildMarkdown, lineText, outcomeOf, type ReportInput } from './report.ts';
import { emptyState } from './session.ts';
import type { Anchor, Comment, FileDiff, Meta, ReviewState, Verdict } from './types.ts';

const FILES: FileDiff[] = [{
  id: 'f0',
  path: 'src/a.ts',
  old_path: 'src/a.ts',
  status: 'modified',
  additions: 1,
  deletions: 1,
  binary: false,
  noise: false,
  language: 'typescript',
  truncated: false,
  hunks: [{
    id: 'f0h0',
    header: '@@ -1,2 +1,2 @@',
    old_start: 1,
    old_count: 2,
    new_start: 1,
    new_count: 2,
    additions: 1,
    deletions: 1,
    lines: [
      { i: 0, type: 'del', old: 1, new: null, text: 'const a = 1;' },
      { i: 1, type: 'add', old: null, new: 1, text: 'const a = 2;' },
    ],
  }],
}];

const META: Meta = {
  repo: 'demo',
  repo_root: '/tmp/demo',
  slug: '2026-01-01-demo',
  title: 'Demo',
  scope: 'branch',
  base: 'main',
  head: 'feature',
  diff_cmd: 'git diff main...HEAD',
  generated_at: '2026-01-01T00:00:00Z',
  totals: { files: 1, additions: 1, deletions: 1 },
};

function comment(verdict: Verdict, anchor: Partial<Anchor>, resolved = false): Comment {
  return {
    id: `c-${verdict}`,
    anchor: { kind: 'line', file: null, hunk: null, side: null, line: null, chapter: null, ...anchor },
    verdict,
    body: `${verdict} note`,
    created_at: META.generated_at,
    updated_at: META.generated_at,
    resolved,
  };
}

function input(state: ReviewState): ReportInput {
  return { meta: META, files: FILES, review: null, state };
}

test('finds the diff line an anchor points at', () => {
  const anchor: Anchor = { kind: 'line', file: 'src/a.ts', hunk: 'f0h0', side: 'new', line: 1, chapter: null };
  assert.equal(lineText(FILES, anchor), '+const a = 2;');
  assert.equal(lineText(FILES, { ...anchor, side: 'old' }), '-const a = 1;');
  assert.equal(lineText(FILES, { ...anchor, line: 99 }), null);
  assert.equal(lineText(FILES, { ...anchor, file: 'nope.ts' }), null);
});

test('an unsubmitted review is abandoned', () => {
  assert.equal(outcomeOf(emptyState()), 'abandoned');
});

test('submitting with no open fix comments is approval', () => {
  const state = { ...emptyState(), submitted: true, comments: [comment('question', { file: 'src/a.ts' })] };
  assert.equal(outcomeOf(state), 'approved');
});

test('an open fix comment requests changes', () => {
  const state = { ...emptyState(), submitted: true, comments: [comment('fix', { file: 'src/a.ts' })] };
  assert.equal(outcomeOf(state), 'changes-requested');
});

test('a resolved fix comment no longer blocks', () => {
  const state = { ...emptyState(), submitted: true, comments: [comment('fix', { file: 'src/a.ts' }, true)] };
  assert.equal(outcomeOf(state), 'approved');
});

test('an overall fix verdict requests changes on its own', () => {
  const state: ReviewState = { ...emptyState(), submitted: true, overall: { verdict: 'fix', body: 'no' } };
  assert.equal(outcomeOf(state), 'changes-requested');
});

test('markdown groups by file and quotes the anchored line', () => {
  const state: ReviewState = {
    ...emptyState(),
    submitted: true,
    comments: [comment('fix', { file: 'src/a.ts', hunk: 'f0h0', side: 'new', line: 1 })],
  };
  const markdown = buildMarkdown(input(state));
  assert.match(markdown, /## src\/a\.ts/);
  assert.match(markdown, /\[FIX\] src\/a\.ts:1/);
  assert.match(markdown, /\+const a = 2;/);
  assert.match(markdown, /1 open comment\(s\), 0 resolved/);
});

test('markdown reports an in-progress review as unsubmitted', () => {
  assert.match(buildMarkdown(input(emptyState())), /IN PROGRESS \(not submitted\)/);
});

test('json output carries outcome, code and location', () => {
  const state: ReviewState = {
    ...emptyState(),
    submitted: true,
    comments: [comment('fix', { file: 'src/a.ts', hunk: 'f0h0', side: 'new', line: 1 })],
  };
  const payload = buildJson(input(state)) as {
    outcome: string;
    comments: { file: string; line: number; code: string }[];
  };
  assert.equal(payload.outcome, 'changes-requested');
  assert.equal(payload.comments.length, 1);
  assert.equal(payload.comments[0]?.file, 'src/a.ts');
  assert.equal(payload.comments[0]?.code, '+const a = 2;');
});

test('resolved comments stay out of the report', () => {
  const state = { ...emptyState(), submitted: true, comments: [comment('fix', { file: 'src/a.ts' }, true)] };
  assert.match(buildMarkdown(input(state)), /0 open comment\(s\), 1 resolved/);
});

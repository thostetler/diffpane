import assert from 'node:assert/strict';
import { test } from 'node:test';

import { ApiError, validateAnchor, validateBody, validateVerdict } from './validate.ts';

test('accepts a well-formed line anchor', () => {
  const anchor = validateAnchor({
    kind: 'line', file: 'a.ts', hunk: 'f0h0', side: 'new', line: 12, chapter: 'c1',
  });
  assert.equal(anchor.kind, 'line');
  assert.equal(anchor.line, 12);
  assert.equal(anchor.chapter, 'c1');
});

test('normalises absent anchor fields to null', () => {
  const anchor = validateAnchor({ kind: 'file', file: 'a.ts' });
  assert.equal(anchor.hunk, null);
  assert.equal(anchor.line, null);
  assert.equal(anchor.chapter, null);
});

test('rejects malformed anchors', () => {
  const cases: unknown[] = [
    null,
    'nope',
    { kind: 'nonsense' },
    { kind: 'line', file: 'a.ts', hunk: 'f0h0', side: 'new' },
    { kind: 'line', file: 'a.ts', hunk: 'f0h0', side: 'sideways', line: 1 },
    { kind: 'line', file: 'a.ts', hunk: 'f0h0', side: 'new', line: 1.5 },
    { kind: 'file' },
    { kind: 'chapter' },
  ];
  for (const value of cases) {
    assert.throws(() => validateAnchor(value), ApiError, JSON.stringify(value));
  }
});

test('rejects unknown verdicts', () => {
  assert.equal(validateVerdict('fix'), 'fix');
  assert.throws(() => validateVerdict('lgtm'), ApiError);
  assert.throws(() => validateVerdict(undefined), ApiError);
});

test('rejects empty comment bodies', () => {
  assert.equal(validateBody('  hi  '), 'hi');
  assert.throws(() => validateBody('   '), ApiError);
  assert.throws(() => validateBody(42), ApiError);
});

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, test } from 'node:test';

import { parseOptions } from './args.ts';
import { SKILL_SOURCE, installSkill } from './install-skill.ts';

let dir: string;

before(() => {
  dir = mkdtempSync(join(tmpdir(), 'diffpane-skill-'));
});

after(() => {
  rmSync(dir, { recursive: true, force: true });
});

test('--install-skill short-circuits before any scope handling', () => {
  assert.deepEqual(parseOptions(['--install-skill']), { kind: 'install-skill' });
  assert.deepEqual(parseOptions(['--install-skill', '--skill-dir', '/tmp/x']), {
    kind: 'install-skill',
    skillDir: '/tmp/x',
  });
});

test('the packaged skill is a real, loadable skill file', () => {
  const body = readFileSync(SKILL_SOURCE, 'utf8');
  assert.match(body, /^---\n/, 'skill needs YAML frontmatter');
  assert.match(body, /^name: diffpane$/m);
  assert.match(body, /^user-invocable: true$/m);
  assert.match(body, /^description: .{40,}/m, 'description carries the trigger phrasing');
});

test('installing writes the skill where Claude Code looks for it', () => {
  const first = installSkill(dir);
  assert.equal(first.path, join(dir, 'diffpane', 'SKILL.md'));
  assert.equal(first.replaced, false);
  assert.equal(readFileSync(first.path, 'utf8'), readFileSync(SKILL_SOURCE, 'utf8'));
});

test('reinstalling reports that it replaced a previous copy', () => {
  writeFileSync(join(dir, 'diffpane', 'SKILL.md'), 'hand-edited\n', 'utf8');
  const second = installSkill(dir);
  assert.equal(second.replaced, true, 'silently clobbering a local edit is not acceptable');
  assert.equal(readFileSync(second.path, 'utf8'), readFileSync(SKILL_SOURCE, 'utf8'));
});

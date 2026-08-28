import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isNoise, languageOf } from './classify.ts';

test('flags lockfiles and generated output as noise', () => {
  for (const path of [
    'pnpm-lock.yaml',
    'apps/web/package-lock.json',
    'go.sum',
    'dist/bundle.js',
    'src/__snapshots__/a.snap',
    'src/vendor/lib.js',
    'a/b/thing.generated.ts',
    'web/app.min.js',
  ]) {
    assert.equal(isNoise(path), true, path);
  }
});

test('leaves ordinary source alone', () => {
  for (const path of ['src/index.ts', 'lib/dist-helper.ts', 'README.md', 'src/build-config.ts']) {
    assert.equal(isNoise(path), false, path);
  }
});

test('maps extensions to languages', () => {
  assert.equal(languageOf('src/a.ts'), 'typescript');
  assert.equal(languageOf('src/a.TSX'), 'tsx');
  assert.equal(languageOf('deploy/Dockerfile'), 'dockerfile');
  assert.equal(languageOf('deploy/Dockerfile.prod'), 'dockerfile');
  assert.equal(languageOf('Makefile'), null);
  assert.equal(languageOf('a/b/thing.unknownext'), null);
});

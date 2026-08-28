import assert from 'node:assert/strict';
import { test } from 'node:test';

import { parseUnifiedDiff } from './diff-parse.ts';

const SIMPLE = `diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,3 +1,4 @@
 const a = 1;
-const b = 2;
+const b = 3;
+const c = 4;
 export { a };
`;

test('parses a single hunk with correct line numbering', () => {
  const [file] = parseUnifiedDiff(SIMPLE);
  assert.ok(file);
  assert.equal(file.path, 'src/a.ts');
  assert.equal(file.hunks.length, 1);
  const hunk = file.hunks[0];
  assert.ok(hunk);
  assert.deepEqual(
    hunk.lines.map((line) => [line.type, line.old, line.new]),
    [
      ['context', 1, 1],
      ['del', 2, null],
      ['add', null, 2],
      ['add', null, 3],
      ['context', 3, 4],
    ],
  );
  assert.equal(hunk.additions, 2);
  assert.equal(hunk.deletions, 1);
});

test('hunk line counts reconcile with the @@ header', () => {
  const [file] = parseUnifiedDiff(SIMPLE);
  const hunk = file?.hunks[0];
  assert.ok(hunk);
  const oldLines = hunk.lines.filter((line) => line.type !== 'add').length;
  const newLines = hunk.lines.filter((line) => line.type !== 'del').length;
  assert.equal(oldLines, hunk.old_count);
  assert.equal(newLines, hunk.new_count);
});

test('the diff trailing newline does not become a phantom context line', () => {
  // Regression: splitting on "\n" yields a final "" that once parsed as a
  // blank context line, inflating every file's last hunk by one.
  const [file] = parseUnifiedDiff(SIMPLE);
  const hunk = file?.hunks[0];
  assert.ok(hunk);
  assert.equal(hunk.lines.length, 5);
  assert.equal(hunk.lines.at(-1)?.text, 'export { a };');
});

test('keeps genuinely blank context lines', () => {
  const patch = 'diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n x\n \n-y\n+z\n';
  const hunk = parseUnifiedDiff(patch)[0]?.hunks[0];
  assert.ok(hunk);
  assert.equal(hunk.lines.length, 4);
  assert.deepEqual(hunk.lines[1], { i: 1, type: 'context', old: 2, new: 2, text: '' });
});

test('ignores the no-newline-at-eof marker', () => {
  const patch = 'diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n';
  const hunk = parseUnifiedDiff(patch)[0]?.hunks[0];
  assert.ok(hunk);
  assert.deepEqual(hunk.lines.map((line) => line.type), ['del', 'add']);
});

test('handles added and deleted files', () => {
  const patch = `diff --git a/new.ts b/new.ts
new file mode 100644
--- /dev/null
+++ b/new.ts
@@ -0,0 +1,2 @@
+one
+two
diff --git a/gone.ts b/gone.ts
deleted file mode 100644
--- a/gone.ts
+++ /dev/null
@@ -1,1 +0,0 @@
-bye
`;
  const files = parseUnifiedDiff(patch);
  assert.equal(files.length, 2);
  assert.equal(files[0]?.path, 'new.ts');
  assert.equal(files[0]?.oldPath, null);
  assert.equal(files[1]?.path, 'gone.ts');
  assert.equal(files[1]?.hunks[0]?.lines[0]?.type, 'del');
});

test('marks binary files and gives them no hunks', () => {
  const patch = 'diff --git a/img.png b/img.png\n--- a/img.png\n+++ b/img.png\nBinary files a/img.png and b/img.png differ\n';
  const [file] = parseUnifiedDiff(patch);
  assert.equal(file?.binary, true);
  assert.equal(file?.hunks.length, 0);
});

test('parses multiple hunks in one file', () => {
  const patch = `diff --git a/a.ts b/a.ts
--- a/a.ts
+++ b/a.ts
@@ -1,2 +1,2 @@
-a
+b
 c
@@ -10,2 +10,2 @@
-d
+e
 f
`;
  const [file] = parseUnifiedDiff(patch);
  assert.equal(file?.hunks.length, 2);
  assert.equal(file?.hunks[1]?.old_start, 10);
  assert.equal(file?.hunks[1]?.lines[0]?.old, 10);
});

test('unquotes paths containing spaces', () => {
  const patch = 'diff --git "a/my dir/a.ts" "b/my dir/a.ts"\n--- "a/my dir/a.ts"\n+++ "b/my dir/a.ts"\n@@ -1 +1 @@\n-a\n+b\n';
  assert.equal(parseUnifiedDiff(patch)[0]?.path, 'my dir/a.ts');
});

test('defaults an omitted hunk count to 1', () => {
  const patch = 'diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -5 +5 @@\n-a\n+b\n';
  const hunk = parseUnifiedDiff(patch)[0]?.hunks[0];
  assert.equal(hunk?.old_count, 1);
  assert.equal(hunk?.new_count, 1);
});

test('truncates noise files at the lower cap', () => {
  const body = Array.from({ length: 200 }, (_, i) => `+line ${i}`).join('\n');
  const patch = `diff --git a/pnpm-lock.yaml b/pnpm-lock.yaml\n--- a/pnpm-lock.yaml\n+++ b/pnpm-lock.yaml\n@@ -1,0 +1,200 @@\n${body}\n`;
  const [file] = parseUnifiedDiff(patch);
  assert.equal(file?.truncated, true);
  const total = file?.hunks.reduce((sum, hunk) => sum + hunk.lines.length, 0) ?? 0;
  assert.ok(total <= 40, `expected <= 40 lines, got ${total}`);
});

test('returns nothing for an empty diff', () => {
  assert.deepEqual(parseUnifiedDiff(''), []);
});

test('does not mistake a deleted "-- " line for a file header', () => {
  // Deleting the SQL comment `-- old` emits `--- old`, which looks like a
  // `---` file header and used to truncate the hunk and corrupt the path.
  const patch = [
    'diff --git a/q.sql b/q.sql',
    '--- a/q.sql',
    '+++ b/q.sql',
    '@@ -1,3 +1,3 @@',
    ' SELECT 1;',
    '--- old comment',
    '+++ new comment',
    ' SELECT 2;',
    '',
  ].join('\n');
  const [file] = parseUnifiedDiff(patch);
  assert.equal(file?.path, 'q.sql');
  assert.equal(file?.oldPath, 'q.sql');
  const hunk = file?.hunks[0];
  assert.equal(hunk?.lines.length, 4);
  assert.deepEqual(
    hunk?.lines.map((line) => [line.type, line.text]),
    [
      ['context', 'SELECT 1;'],
      ['del', '-- old comment'],
      ['add', '++ new comment'],
      ['context', 'SELECT 2;'],
    ],
  );
});

test('does not mistake diff content inside a hunk for a new file', () => {
  // Patches committed as fixtures contain `diff --git` lines as content.
  const patch = [
    'diff --git a/fixture.txt b/fixture.txt',
    '--- a/fixture.txt',
    '+++ b/fixture.txt',
    '@@ -1,1 +1,2 @@',
    ' keep',
    '+diff --git a/inner b/inner',
    '',
  ].join('\n');
  const files = parseUnifiedDiff(patch);
  assert.equal(files.length, 1);
  assert.equal(files[0]?.hunks[0]?.lines.length, 2);
});

test('starts a new file once the previous hunk is exhausted', () => {
  const patch = [
    'diff --git a/a.ts b/a.ts',
    '--- a/a.ts',
    '+++ b/a.ts',
    '@@ -1,1 +1,1 @@',
    '-a',
    '+b',
    'diff --git a/b.ts b/b.ts',
    '--- a/b.ts',
    '+++ b/b.ts',
    '@@ -1,1 +1,1 @@',
    '-c',
    '+d',
    '',
  ].join('\n');
  const files = parseUnifiedDiff(patch);
  assert.deepEqual(files.map((file) => file.path), ['a.ts', 'b.ts']);
});

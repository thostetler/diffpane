/**
 * Browser coverage for ui/. Every case here is a bug that actually shipped, or
 * a contract clause that only exists at runtime. Assertions are behavioural:
 * colours, spacing and copy are expected to churn, focus and data are not.
 */
import assert from 'node:assert/strict';
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { Readable } from 'node:stream';
import { after, afterEach, before, beforeEach, test } from 'node:test';

import { chromium, type Browser, type Page } from 'playwright';

interface Fixture {
  meta: unknown;
  hunks: unknown;
  review: unknown;
  comments: { comments: unknown[] };
}

interface Address {
  url: string;
  token: string;
}

interface State {
  overall: { verdict: string | null; body: string };
}

const ROOT = join(import.meta.dirname, '..');
const MANIFEST = join(ROOT, 'Cargo.toml');

/**
 * Where cargo actually puts the build. Assuming `target/debug` meant that on a
 * machine with CARGO_TARGET_DIR set the build succeeded, the spawn hit ENOENT,
 * and the suite blamed the server for exiting without an address.
 */
function serverPath(): string {
  const metadata = execFileSync(
    'cargo',
    ['metadata', '--no-deps', '--format-version', '1', '--manifest-path', MANIFEST],
    { encoding: 'utf8' },
  );
  const { target_directory } = JSON.parse(metadata) as { target_directory: string };
  return join(target_directory, 'debug', 'examples', 'serve-fixture');
}

const FIXTURE = JSON.parse(
  readFileSync(join(import.meta.dirname, 'fixture.json'), 'utf8'),
) as Fixture;

/** The fixture ships a comment on this file; several cases depend on that. */
const COMMENTED_FILE = 'src/search/cache.ts';

let dir: string;
let server: ChildProcess;
let browser: Browser;
let page: Page;
let url: string;

function writeJson(path: string, payload: unknown): void {
  writeFileSync(path, `${JSON.stringify(payload, null, 2)}\n`);
}

function state(): State {
  return JSON.parse(readFileSync(join(dir, 'comments.json'), 'utf8')) as State;
}

/** The address the server picked. It prints one line, then serves. */
function firstLine(stream: Readable): Promise<string> {
  return new Promise((resolve, reject) => {
    let buffered = '';
    const onData = (chunk: Buffer): void => {
      buffered += chunk.toString();
      const end = buffered.indexOf('\n');
      if (end === -1) return;
      stream.off('data', onData);
      resolve(buffered.slice(0, end));
    };
    stream.on('data', onData);
    stream.once('error', reject);
    stream.once('end', () => reject(new Error('the server exited without an address')));
  });
}

before(async () => {
  // The suite drives the shipping server, so it needs it built. cargo no-ops
  // when it already is; `pnpm test:ui` is meant to work from a cold checkout.
  execFileSync('cargo', ['build', '--manifest-path', MANIFEST, '--example', 'serve-fixture'], {
    stdio: 'inherit',
  });

  dir = mkdtempSync(join(tmpdir(), 'diffpane-ui-'));
  writeJson(join(dir, 'meta.json'), FIXTURE.meta);
  writeJson(join(dir, 'hunks.json'), FIXTURE.hunks);
  writeJson(join(dir, 'review.json'), FIXTURE.review);

  server = spawn(serverPath(), [dir], { stdio: ['ignore', 'pipe', 'inherit'] });
  const address = JSON.parse(await firstLine(server.stdout!)) as Address;
  url = address.url;
  browser = await chromium.launch();
});

after(async () => {
  await browser.close();
  server.kill();
  rmSync(dir, { recursive: true, force: true });
});

beforeEach(async () => {
  writeJson(join(dir, 'comments.json'), {
    comments: FIXTURE.comments.comments,
    progress: {},
    overall: { verdict: null, body: '' },
    submitted: false,
    submitted_at: null,
  });
  page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.goto(url);
  await page.waitForSelector('.diff-row');
});

afterEach(async () => {
  await page.close();
});

/** Fails a single endpoint so the error path can be exercised for real. */
async function breakEndpoint(pattern: string): Promise<void> {
  await page.route(pattern, (route) =>
    route.fulfill({
      status: 500,
      contentType: 'application/json',
      body: JSON.stringify({ error: 'injected failure' }),
    }),
  );
}

async function focusedKey(): Promise<string> {
  return page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (el === null || el === document.body) return 'BODY';
    return el.dataset['fk'] ?? el.tagName;
  });
}

/** Clicks past pointer interception, for asserting things about overlays. */
function clickThrough(selector: string): Promise<void> {
  return page.evaluate((target) => {
    document.querySelector<HTMLElement>(target)?.click();
  }, selector);
}

test('focus survives the re-render its own click causes', async () => {
  const button = page.locator('[data-fk^="chapter-review:"]').first();
  await button.focus();
  await button.click();
  assert.match(await focusedKey(), /^chapter-review:/);
});

test('the diff is one tab stop, not one per row', async () => {
  const rows = await page.locator('.diff-row').count();
  assert.ok(rows > 20, `expected a substantial diff, got ${rows} rows`);
  assert.equal(await page.locator('.diff-row[tabindex="0"]').count(), 1);
});

test('add and del are never conveyed by colour alone', async () => {
  const rows = await page.locator('.diff-row.add, .diff-row.del').all();
  assert.ok(rows.length > 0);
  for (const row of rows) {
    const marker = (await row.locator('.marker').textContent()) ?? '';
    assert.match(marker.trim(), /^[+-]$/);
    assert.match((await row.getAttribute('aria-label')) ?? '', /^(added|removed) line/);
  }
});

test('the comment button is visible once it holds focus', async () => {
  const row = page.locator('.diff-row').first();
  await row.click();
  const button = row.locator('.add-comment');
  await button.focus();
  assert.equal(await button.evaluate((el) => getComputedStyle(el).opacity), '1');
});

test('the composer traps focus and Escape returns it to the line', async () => {
  const row = page.locator('.diff-row').first();
  await row.click();
  await row.locator('.add-comment').click();
  await page.waitForSelector('.composer textarea');

  // The checked radio is the group's only tab stop, and it is not index 0.
  await page.locator('.composer input:checked').focus();
  await page.keyboard.press('Shift+Tab');
  assert.equal(await page.locator('.composer :focus').count(), 1, 'focus escaped the composer');

  await page.keyboard.press('Escape');
  await page.waitForSelector('.composer', { state: 'detached' });
  assert.match(await focusedKey(), /^add-comment:/);
});

test('collapse all keeps files that carry a comment expanded', async () => {
  await page.locator('[data-fk="fold-all"]').click();
  const commented = page.locator(`.file[data-file="${COMMENTED_FILE}"] .file-toggle`).first();
  assert.equal(await commented.getAttribute('aria-expanded'), 'true');
  assert.ok(
    (await page.locator('.file-toggle[aria-expanded="false"]').count()) > 0,
    'collapse all collapsed nothing',
  );
});

test('the fold control toggles its own label', async () => {
  const button = page.locator('[data-fk="fold-all"]');
  assert.equal(await button.textContent(), 'Collapse all');
  await button.click();
  assert.equal(await button.textContent(), 'Expand all');
  await button.click();
  assert.equal(await button.textContent(), 'Collapse all');
});

test('a failed comment save keeps the text and says so', async () => {
  await breakEndpoint('**/api/comments');
  const row = page.locator('.diff-row').first();
  await row.click();
  await row.locator('.add-comment').click();
  await page.locator('.composer textarea').fill('do not lose me');
  await page.locator('.composer button[type="submit"]').click();

  await page.waitForSelector('.composer .error');
  assert.equal(await page.locator('.composer textarea').inputValue(), 'do not lose me');
  assert.equal(await page.locator('.composer .error').getAttribute('role'), 'alert');
});

test('a failed autosave keeps the overall notes and surfaces a banner', async () => {
  await breakEndpoint('**/api/overall');
  const notes = page.locator('.overall-notes');
  await notes.fill('overall text that must survive');
  await notes.blur();

  await page.waitForSelector('.banner');
  assert.equal(await page.locator('.banner').getAttribute('role'), 'alert');
  assert.equal(await notes.inputValue(), 'overall text that must survive');
});

test('the outcome control shows the verdict a submit would send', async () => {
  // emptyState() leaves the verdict null; readOverall() falls back to "ok".
  assert.equal(await page.locator('.footer input:checked').inputValue(), 'ok');

  // The label is the click target: the radio itself is hidden and inert.
  const saved = page.waitForResponse(
    (response) => response.url().endsWith('/api/overall') && response.status() === 200,
  );
  await page.locator('.footer .seg.fix').click();
  await saved;
  assert.equal(state().overall.verdict, 'fix');
});

test('a saved comment is anchored under its line and counted in the sidebar', async () => {
  const row = page.locator('.diff-row.add').first();
  // Pin the row up front. Saving re-renders, so re-resolving "the first add
  // row" afterwards asserts about whatever is first *now* — which is not
  // necessarily the line the comment was left on.
  const rowId = await row.getAttribute('id');
  const chapter = await row.getAttribute('data-chapter');
  await row.click();
  await row.locator('.add-comment').click();
  await page.locator('.composer .seg.question').click();
  await page.locator('.composer textarea').fill('why this way?');

  // Wait on the save itself. The fixture already ships a question comment, so
  // waiting for `.comment-box.question` matches at load and waits for nothing.
  const saved = page.waitForResponse(
    (response) => response.url().endsWith('/api/comments') && response.request().method() === 'POST',
  );
  await page.locator('.composer button[type="submit"]').click();
  await saved;
  await page.waitForSelector('.composer', { state: 'detached' });

  const under = await page.locator(`[id="${rowId}"]`).evaluate((el) => ({
    class: el.nextElementSibling?.className ?? 'nothing',
    text: el.nextElementSibling?.textContent ?? '',
  }));
  assert.match(under.class, /comment-box question/, `under ${rowId}: ${under.class}`);
  assert.match(under.text, /why this way\?/, 'the anchored comment is not the one just saved');
  assert.match(
    (await page.locator(`.nav-item[data-chapter="${chapter}"]`).getAttribute('aria-label')) ?? '',
    /open comments?$/,
  );
});

test('the help overlay survives a re-render and restores focus', async () => {
  await page.locator('[data-fk="help"]').click();
  await page.waitForSelector('#help-overlay.show');

  await clickThrough('[data-fk="fold-all"]');
  assert.equal(await page.locator('#help-overlay.show').count(), 1, 'a render closed the overlay');

  await page.keyboard.press('Escape');
  await page.waitForSelector('#help-overlay.show', { state: 'detached' });
  assert.equal(await focusedKey(), 'help');
});

test('the file header stays under the page header while its file is on screen', async () => {
  const parked = await page.evaluate((path) => {
    const file = document.querySelector<HTMLElement>(`.file[data-file="${path}"]`)!;
    // Put the file's top well above the viewport so its header must stick.
    scrollTo(0, file.getBoundingClientRect().top + scrollY + 250);
    const head = file.querySelector('.file-head')!.getBoundingClientRect();
    const top = document.querySelector('.top')!.getBoundingClientRect();
    return { headerTop: head.top, pageHeaderBottom: top.bottom, fileBottom: file.getBoundingClientRect().bottom };
  }, COMMENTED_FILE);

  assert.ok(parked.fileBottom > parked.pageHeaderBottom, 'file scrolled out of view entirely');
  assert.ok(
    Math.abs(parked.headerTop - parked.pageHeaderBottom) < 2,
    `file header at ${parked.headerTop}, expected to park at ${parked.pageHeaderBottom}`,
  );
});

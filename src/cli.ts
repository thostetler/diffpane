#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { parseOptions, USAGE, type Options } from './args.ts';
import { assembleDiff, computeTotals } from './assemble.ts';
import { currentBranch, repoRoot, resolveScope } from './git.ts';
import { installSkill } from './install-skill.ts';
import { openBrowser } from './open-browser.ts';
import { buildJson, buildMarkdown, outcomeOf, type Outcome } from './report.ts';
import { Session, emptyState, nowIso, slugify, writeJson } from './session.ts';
import { buildServer, generateToken, listen } from './server.ts';
import type { Meta, Review } from './types.ts';

const EXIT: Record<Outcome, number> = {
  approved: 0,
  'changes-requested': 1,
  abandoned: 2,
};

function version(): string {
  const path = resolve(import.meta.dirname, '..', 'package.json');
  return (JSON.parse(readFileSync(path, 'utf8')) as { version: string }).version;
}

function buildSession(options: Options, root: string): { session: Session; meta: Meta } | null {
  const { scope, diffArgs, base } = resolveScope(root, options);
  const files = assembleDiff(root, diffArgs);
  if (files.length === 0) return null;

  const branch = currentBranch(root);
  const slug = `${new Date().toISOString().slice(0, 10)}-${slugify(options.title ?? branch)}`;
  const session = Session.create(root, slug);
  const meta: Meta = {
    repo: root.split('/').pop() ?? root,
    repo_root: root,
    slug,
    title: options.title ?? slug,
    scope,
    base,
    head: branch,
    diff_cmd: ['git', 'diff', ...diffArgs].join(' ').trimEnd(),
    generated_at: nowIso(),
    totals: computeTotals(files),
  };
  writeJson(session.metaPath, meta);
  writeJson(session.hunksPath, { files });
  // Always start clean. Re-running on the same branch the same day reuses the
  // session directory, and inheriting the last run's comments would replay a
  // stale `submitted: true` and anchor comments to hunk ids that have moved.
  writeJson(session.statePath, emptyState());
  if (options.reviewFile !== undefined) installReview(session, options.reviewFile, files);
  return { session, meta };
}

function installReview(
  session: Session,
  file: string,
  files: ReturnType<typeof assembleDiff>,
): void {
  const review = JSON.parse(readFileSync(file, 'utf8')) as Review;
  const known = new Set(files.flatMap((entry) => entry.hunks.map((hunk) => hunk.id)));
  for (const chapter of review.chapters ?? []) {
    for (const id of chapter.hunks ?? []) {
      if (!known.has(id)) {
        process.stderr.write(`warning: chapter ${chapter.id} references unknown hunk ${id}\n`);
      }
    }
  }
  writeJson(session.reviewPath, review);
}

/** Resolves when the review is submitted, the timeout fires, or the user quits. */
function waitForSubmit(timeoutSeconds: number): { promise: Promise<void>; onSubmit: () => void } {
  let onSubmit = (): void => undefined;
  const promise = new Promise<void>((resolvePromise) => {
    onSubmit = resolvePromise;
    if (timeoutSeconds > 0) setTimeout(resolvePromise, timeoutSeconds * 1000).unref();
    process.once('SIGINT', () => {
      process.stderr.write('\n');
      resolvePromise();
    });
  });
  return { promise, onSubmit };
}

function emitReport(session: Session, options: Options): Outcome {
  const input = {
    meta: session.meta(),
    files: session.hunks().files,
    review: session.review(),
    state: session.state(),
  };
  const markdown = buildMarkdown(input);
  if (options.outFile !== undefined) writeFileSync(options.outFile, markdown, 'utf8');
  if (options.asJson) process.stdout.write(`${JSON.stringify(buildJson(input), null, 2)}\n`);
  else if (options.outFile === undefined) process.stdout.write(markdown);
  return outcomeOf(input.state);
}

async function run(options: Options): Promise<number> {
  const root = repoRoot(process.cwd());
  const built = buildSession(options, root);
  if (built === null) {
    process.stderr.write('no changes to review\n');
    return 0;
  }

  const token = generateToken();
  const { promise, onSubmit } = waitForSubmit(options.timeoutSeconds);
  const server = buildServer({ session: built.session, token, onSubmit });
  const port = await listen(server, options.port);
  const url = `http://127.0.0.1:${port}/?t=${token}`;

  const { files, additions, deletions } = built.meta.totals;
  process.stderr.write(`diffpane  ${files} files, +${additions}/-${deletions}\n`);
  process.stderr.write(`review    ${url}\n`);
  if (options.shouldOpen) openBrowser(url);

  await promise;
  // Let the submit response finish leaving the socket before tearing down.
  await new Promise((resolveTick) => {
    setImmediate(resolveTick);
  });
  server.close();
  server.closeAllConnections();
  return EXIT[emitReport(built.session, options)];
}

async function main(): Promise<number> {
  const parsed = parseOptions(process.argv.slice(2));
  if (parsed.kind === 'help') {
    process.stdout.write(USAGE);
    return 0;
  }
  if (parsed.kind === 'version') {
    process.stdout.write(`${version()}\n`);
    return 0;
  }
  if (parsed.kind === 'install-skill') {
    const { path, replaced } = installSkill(parsed.skillDir);
    process.stdout.write(`${replaced ? 'replaced' : 'wrote'} ${path}\n`);
    process.stdout.write('restart Claude Code, then run /diffpane\n');
    return 0;
  }
  return run(parsed.options);
}

main().then(
  (code) => {
    process.exitCode = code;
  },
  (error: unknown) => {
    process.stderr.write(`diffpane: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 3;
  },
);

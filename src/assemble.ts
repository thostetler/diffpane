import { isNoise, languageOf } from './classify.ts';
import { parseUnifiedDiff } from './diff-parse.ts';
import { readNumstat, readPatch, readRaw } from './git.ts';
import type { FileDiff, Totals } from './types.ts';

/**
 * The file list comes from `git diff --raw`, not from the patch text. Binary,
 * rename-only and mode-only changes have no `---`/`+++` lines at all, so a
 * patch-driven list silently drops them. The patch supplies hunks and nothing
 * else.
 */
export function assembleDiff(root: string, diffArgs: string[]): FileDiff[] {
  const raw = readRaw(root, diffArgs);
  const numstat = readNumstat(root, diffArgs);
  const parsed = parseUnifiedDiff(readPatch(root, diffArgs));
  const hunksByPath = new Map(parsed.map((file) => [file.path, file]));

  return [...raw.entries()].map(([path, meta], index) => {
    const stats = numstat.get(path);
    const patch = hunksByPath.get(path);
    const binary = stats?.binary === true || patch?.binary === true;
    const hunks = binary ? [] : (patch?.hunks ?? []);
    hunks.forEach((hunk, hunkIndex) => {
      hunk.id = `f${index}h${hunkIndex}`;
    });
    return {
      id: `f${index}`,
      path,
      old_path: meta.oldPath ?? path,
      status: meta.status,
      additions: stats?.additions ?? 0,
      deletions: stats?.deletions ?? 0,
      binary,
      noise: isNoise(path),
      language: languageOf(path),
      truncated: patch?.truncated ?? false,
      hunks,
    };
  });
}

export function computeTotals(files: FileDiff[]): Totals {
  return {
    files: files.length,
    additions: files.reduce((sum, file) => sum + file.additions, 0),
    deletions: files.reduce((sum, file) => sum + file.deletions, 0),
  };
}

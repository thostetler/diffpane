import { parseArgs } from 'node:util';

import type { Scope } from './types.ts';

export interface Options {
  scope: Scope;
  base?: string;
  range?: string;
  commit?: string;
  paths: string[];
  title?: string;
  reviewFile?: string;
  outFile?: string;
  port: number;
  timeoutSeconds: number;
  asJson: boolean;
  shouldOpen: boolean;
}

export const USAGE = `diffpane — review a git diff in your browser, then hand the feedback back.

Usage
  diffpane [options] [-- <pathspec>...]

Scope (default: current branch vs its base)
  --base <ref>       base ref for the branch diff
  --working          uncommitted changes
  --staged           staged changes
  --range <a..b>     an explicit range
  --commit <sha>     a single commit

Options
  --title <text>     human title for the review
  --review <file>    narrative JSON (chapters + descriptions) to render
  --out <file>       write the markdown report to a file
  --json             print machine-readable feedback to stdout
  --port <n>         preferred port (default 7777, walks forward if taken)
  --no-open          do not open a browser
  --timeout <sec>    give up waiting after N seconds (default 3600, 0 = never)
  -h, --help         show this
  -v, --version      show version

Agent setup
  --install-skill    install the Claude Code skill, then exit
  --skill-dir <dir>  where to install it (default ~/.claude/skills)

Exit codes
  0  approved (or nothing to review)   2  abandoned / timed out
  1  changes requested                 3  error
`;

function toPositiveInt(value: string | undefined, fallback: number, label: string): number {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) throw new Error(`--${label} must be a number >= 0`);
  return Math.floor(parsed);
}

function selectScope(values: Record<string, unknown>): Scope {
  if (values['range'] !== undefined) return 'range';
  if (values['commit'] !== undefined) return 'commit';
  if (values['working'] === true) return 'working';
  if (values['staged'] === true) return 'staged';
  return 'branch';
}

export type ParseResult =
  | { kind: 'options'; options: Options }
  | { kind: 'install-skill'; skillDir?: string }
  | { kind: 'help' }
  | { kind: 'version' };

export function parseOptions(argv: string[]): ParseResult {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      base: { type: 'string' },
      range: { type: 'string' },
      commit: { type: 'string' },
      working: { type: 'boolean' },
      staged: { type: 'boolean' },
      title: { type: 'string' },
      review: { type: 'string' },
      out: { type: 'string' },
      json: { type: 'boolean' },
      port: { type: 'string' },
      'no-open': { type: 'boolean' },
      timeout: { type: 'string' },
      'install-skill': { type: 'boolean' },
      'skill-dir': { type: 'string' },
      help: { type: 'boolean', short: 'h' },
      version: { type: 'boolean', short: 'v' },
    },
  });

  if (values.help === true) return { kind: 'help' };
  if (values.version === true) return { kind: 'version' };
  if (values['install-skill'] === true) {
    const skillDir = values['skill-dir'];
    return skillDir === undefined ? { kind: 'install-skill' } : { kind: 'install-skill', skillDir };
  }

  const exclusive = ([
    ['range', values.range],
    ['commit', values.commit],
    ['working', values.working],
    ['staged', values.staged],
  ] as const)
    .filter(([, value]) => value !== undefined && value !== false)
    .map(([key]) => key);
  if (exclusive.length > 1) {
    throw new Error(`pick one scope, not ${exclusive.map((key) => `--${key}`).join(' and ')}`);
  }

  return {
    kind: 'options',
    options: {
      scope: selectScope(values),
      base: values.base,
      range: values.range,
      commit: values.commit,
      paths: positionals,
      title: values.title,
      reviewFile: values.review,
      outFile: values.out,
      port: toPositiveInt(values.port, 7777, 'port'),
      timeoutSeconds: toPositiveInt(values.timeout, 3600, 'timeout'),
      asJson: values.json === true,
      shouldOpen: values['no-open'] !== true,
    },
  };
}

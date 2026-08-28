import type { Anchor, Comment, FileDiff, Meta, Review, ReviewState, Verdict } from './types.ts';

const VERDICT_MARK: Record<Verdict, string> = {
  ok: '[ok]',
  fix: '[FIX]',
  question: '[?]',
};

export type Outcome = 'approved' | 'changes-requested' | 'abandoned';

export interface ReportInput {
  meta: Meta;
  files: FileDiff[];
  review: Review | null;
  state: ReviewState;
}

export function openComments(state: ReviewState): Comment[] {
  return state.comments.filter((comment) => !comment.resolved);
}

export function outcomeOf(state: ReviewState): Outcome {
  if (!state.submitted) return 'abandoned';
  const blocking = openComments(state).some((comment) => comment.verdict === 'fix');
  return blocking || state.overall.verdict === 'fix' ? 'changes-requested' : 'approved';
}

/** The diff line a comment is pinned to, for quoting back in the report. */
export function lineText(files: FileDiff[], anchor: Anchor): string | null {
  const marker: Record<string, string> = { add: '+', del: '-', context: ' ' };
  for (const file of files) {
    if (file.path !== anchor.file) continue;
    for (const hunk of file.hunks) {
      if (anchor.hunk !== null && hunk.id !== anchor.hunk) continue;
      for (const line of hunk.lines) {
        const side = anchor.side === 'old' ? line.old : line.new;
        if (side === anchor.line) return `${marker[line.type] ?? ' '}${line.text}`;
      }
    }
  }
  return null;
}

/** A fence must be longer than any backtick run inside the content it wraps. */
function fenceFor(content: string): string {
  const runs = [...content.matchAll(/`+/g)];
  const longest = runs.reduce((max, run) => Math.max(max, run[0].length), 0);
  return '`'.repeat(Math.max(3, longest + 1));
}

function groupKey(comment: Comment, chapters: Map<string, string>): string {
  const { anchor } = comment;
  if (anchor.file !== null) return anchor.file;
  if (anchor.kind === 'chapter' && anchor.chapter !== null) {
    return `chapter: ${chapters.get(anchor.chapter) ?? anchor.chapter}`;
  }
  return 'general';
}

function chapterTitles(review: Review | null): Map<string, string> {
  return new Map((review?.chapters ?? []).map((chapter) => [chapter.id, chapter.title]));
}

export function buildMarkdown(input: ReportInput): string {
  const { meta, files, review, state } = input;
  const chapters = chapterTitles(review);
  const open = openComments(state);
  const resolved = state.comments.length - open.length;
  const status = state.submitted ? 'submitted' : 'IN PROGRESS (not submitted)';
  const lines: string[] = [
    `# Review feedback — ${meta.title}`,
    '',
    `Scope: \`${meta.diff_cmd}\` · ${status}`,
  ];

  if (state.overall.verdict !== null || state.overall.body !== '') {
    lines.push('', `**Overall [${state.overall.verdict ?? 'n/a'}]** ${state.overall.body}`);
  }
  lines.push('', `${open.length} open comment(s), ${resolved} resolved.`);

  for (const [key, group] of groupComments(open, chapters)) {
    lines.push('', `## ${key}`);
    for (const comment of group) {
      const where = comment.anchor.line === null ? '' : `:${comment.anchor.line}`;
      const [first = '', ...rest] = comment.body.split('\n');
      lines.push('', `- **${VERDICT_MARK[comment.verdict]} ${key}${where}** — ${first}`);
      // Continuation lines must stay indented or they escape the list item.
      for (const line of rest) lines.push(`  ${line}`);
      const snippet = comment.anchor.kind === 'line' ? lineText(files, comment.anchor) : null;
      if (snippet !== null) {
        const fence = fenceFor(snippet);
        lines.push(`  ${fence}`, `  ${snippet}`, `  ${fence}`);
      }
    }
  }

  const unreviewed = (review?.chapters ?? [])
    .filter((chapter) => state.progress[chapter.id] !== 'reviewed')
    .map((chapter) => chapter.title);
  if (unreviewed.length > 0) {
    lines.push('', `Chapters not marked reviewed: ${unreviewed.join(', ')}`);
  }
  return `${lines.join('\n')}\n`;
}

function groupComments(
  comments: Comment[],
  chapters: Map<string, string>,
): Map<string, Comment[]> {
  const groups = new Map<string, Comment[]>();
  for (const comment of comments) {
    const key = groupKey(comment, chapters);
    const bucket = groups.get(key);
    if (bucket === undefined) groups.set(key, [comment]);
    else bucket.push(comment);
  }
  for (const bucket of groups.values()) {
    bucket.sort((a, b) => (a.anchor.line ?? 0) - (b.anchor.line ?? 0));
  }
  return groups;
}

export function buildJson(input: ReportInput): unknown {
  const { meta, files, state } = input;
  return {
    outcome: outcomeOf(state),
    submitted: state.submitted,
    submitted_at: state.submitted_at,
    scope: meta.diff_cmd,
    totals: meta.totals,
    overall: state.overall,
    progress: state.progress,
    comments: openComments(state).map((comment) => ({
      verdict: comment.verdict,
      body: comment.body,
      file: comment.anchor.file,
      line: comment.anchor.line,
      kind: comment.anchor.kind,
      chapter: comment.anchor.chapter,
      code: comment.anchor.kind === 'line' ? lineText(files, comment.anchor) : null,
    })),
  };
}

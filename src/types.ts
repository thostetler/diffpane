// The JSON wire format is snake_case: it is a documented, frozen contract that
// the browser UI reads directly. See docs/contract.md.

export type FileStatus = 'added' | 'modified' | 'deleted' | 'renamed' | 'copied';
export type LineType = 'context' | 'add' | 'del';
export type Side = 'old' | 'new';
export type Verdict = 'ok' | 'fix' | 'question';
export type AnchorKind = 'line' | 'file' | 'chapter' | 'overall';
export type ProgressState = 'unreviewed' | 'reviewed';
export type Scope = 'branch' | 'working' | 'staged' | 'range' | 'commit';

export interface DiffLine {
  i: number;
  type: LineType;
  old: number | null;
  new: number | null;
  text: string;
}

export interface Hunk {
  id: string;
  header: string;
  old_start: number;
  old_count: number;
  new_start: number;
  new_count: number;
  additions: number;
  deletions: number;
  lines: DiffLine[];
}

export interface FileDiff {
  id: string;
  path: string;
  old_path: string;
  status: FileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
  noise: boolean;
  language: string | null;
  truncated: boolean;
  hunks: Hunk[];
}

export interface Totals {
  files: number;
  additions: number;
  deletions: number;
}

export interface Meta {
  repo: string;
  repo_root: string;
  slug: string;
  title: string;
  scope: Scope;
  base: string;
  head: string;
  diff_cmd: string;
  generated_at: string;
  totals: Totals;
}

export interface Chapter {
  id: string;
  title: string;
  intent?: string;
  why?: string;
  hunks: string[];
  size?: string;
  flags?: string[];
}

export interface Review {
  title?: string;
  story?: string;
  chapters: Chapter[];
  file_notes?: Record<string, string>;
}

export interface Anchor {
  kind: AnchorKind;
  file: string | null;
  hunk: string | null;
  side: Side | null;
  line: number | null;
  chapter: string | null;
}

export interface Comment {
  id: string;
  anchor: Anchor;
  verdict: Verdict;
  body: string;
  created_at: string;
  updated_at: string;
  resolved: boolean;
}

export interface Overall {
  verdict: Verdict | null;
  body: string;
}

export interface ReviewState {
  comments: Comment[];
  progress: Record<string, ProgressState>;
  overall: Overall;
  submitted: boolean;
  submitted_at: string | null;
}

export interface Payload {
  meta: Meta;
  hunks: { files: FileDiff[] };
  review: Review | null;
  comments: ReviewState;
}

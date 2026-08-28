import type { Anchor, AnchorKind, ProgressState, Side, Verdict } from './types.ts';

const VERDICTS: Verdict[] = ['ok', 'fix', 'question'];
const ANCHOR_KINDS: AnchorKind[] = ['line', 'file', 'chapter', 'overall'];
const PROGRESS_STATES: ProgressState[] = ['unreviewed', 'reviewed'];

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status = 400) {
    super(message);
    this.status = status;
  }
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new ApiError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

export function validateVerdict(value: unknown): Verdict {
  if (!VERDICTS.includes(value as Verdict)) {
    throw new ApiError(`verdict must be one of ${VERDICTS.join(', ')}`);
  }
  return value as Verdict;
}

export function validateProgressState(value: unknown): ProgressState {
  if (!PROGRESS_STATES.includes(value as ProgressState)) {
    throw new ApiError(`state must be one of ${PROGRESS_STATES.join(', ')}`);
  }
  return value as ProgressState;
}

export function validateBody(value: unknown): string {
  const text = typeof value === 'string' ? value.trim() : '';
  if (text === '') throw new ApiError('comment body is empty');
  return text;
}

export function validateResolved(value: unknown): boolean {
  if (typeof value !== 'boolean') throw new ApiError('resolved must be a boolean');
  return value;
}

/** Present-and-a-non-empty-string. Presence alone let objects reach the report. */
function requireString(anchor: Record<string, unknown>, kind: AnchorKind, field: string): void {
  const value = anchor[field];
  if (value === undefined || value === null || value === '') {
    throw new ApiError(`${kind} anchor requires ${field}`);
  }
  if (typeof value !== 'string') throw new ApiError(`anchor.${field} must be a string`);
}

function requireFields(anchor: Record<string, unknown>, kind: AnchorKind): void {
  const required: Record<AnchorKind, string[]> = {
    line: ['file', 'hunk', 'side'],
    file: ['file'],
    chapter: ['chapter'],
    overall: [],
  };
  for (const field of required[kind]) {
    requireString(anchor, kind, field);
  }
}

export function validateAnchor(value: unknown): Anchor {
  const anchor = asRecord(value, 'anchor');
  const kind = anchor['kind'];
  if (!ANCHOR_KINDS.includes(kind as AnchorKind)) {
    throw new ApiError(`anchor.kind must be one of ${ANCHOR_KINDS.join(', ')}`);
  }
  requireFields(anchor, kind as AnchorKind);
  if (kind === 'line') {
    if (anchor['side'] !== 'old' && anchor['side'] !== 'new') {
      throw new ApiError("anchor.side must be 'old' or 'new'");
    }
    const line = anchor['line'];
    if (!Number.isInteger(line) || (line as number) < 1) {
      throw new ApiError('anchor.line must be a positive integer');
    }
  }
  if (kind !== 'line' && anchor['chapter'] !== undefined && anchor['chapter'] !== null) {
    requireString(anchor, 'chapter', 'chapter');
  }
  return {
    kind: kind as AnchorKind,
    file: (anchor['file'] as string | undefined) ?? null,
    hunk: (anchor['hunk'] as string | undefined) ?? null,
    side: (anchor['side'] as Side | undefined) ?? null,
    line: (anchor['line'] as number | undefined) ?? null,
    chapter: (anchor['chapter'] as string | undefined) ?? null,
  };
}

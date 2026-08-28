import { isNoise } from './classify.ts';
import type { DiffLine, Hunk, LineType } from './types.ts';

// Hard caps so a monorepo-wide diff cannot produce a hundred-megabyte payload.
const MAX_LINES_PER_FILE = 2000;
const MAX_LINES_PER_NOISE_FILE = 40;

const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

export interface ParsedFile {
  path: string;
  oldPath: string | null;
  binary: boolean;
  truncated: boolean;
  hunks: Hunk[];
}

function unquote(path: string): string {
  if (!path.startsWith('"') || !path.endsWith('"')) return path;
  return path
    .slice(1, -1)
    .replace(/\\(\d{3})/g, (_, oct: string) => String.fromCharCode(parseInt(oct, 8)))
    .replace(/\\(.)/g, '$1');
}

/** Every line in a hunk body starts with one of these markers. */
function isContentLine(raw: string): boolean {
  const marker = raw[0];
  return marker === ' ' || marker === '+' || marker === '-' || marker === '\\';
}

/** Drop git's `a/` or `b/` prefix from a `---`/`+++` path. */
function stripPrefix(path: string): string {
  const bare = unquote(path.trim());
  if (bare === '/dev/null') return bare;
  return bare.length > 2 && bare[1] === '/' ? bare.slice(2) : bare;
}

class DiffParser {
  private readonly files: ParsedFile[] = [];
  private current: ParsedFile | null = null;
  private hunk: Hunk | null = null;
  private oldNo = 0;
  private newNo = 0;
  private oldRemaining = 0;
  private newRemaining = 0;
  private linesUsed = 0;
  private cap = MAX_LINES_PER_FILE;

  parse(patch: string): ParsedFile[] {
    for (const raw of patch.split('\n')) {
      this.handle(raw);
    }
    this.closeHunk();
    return this.files.filter((file) => file.path !== '');
  }

  private handle(raw: string): void {
    // Inside an unexhausted hunk every line is content, however much it looks
    // like a header. Deleting a SQL comment `-- x` emits `--- x`; treating that
    // as a file header truncates the hunk and corrupts the path.
    if (this.isHunkOpen() && isContentLine(raw)) return this.addLine(raw);
    if (raw.startsWith('diff --git ')) return this.startFile();
    if (this.current === null) return;
    if (raw.startsWith('--- ')) return this.setOldPath(raw.slice(4));
    if (raw.startsWith('+++ ')) return this.setNewPath(raw.slice(4));
    if (raw.startsWith('Binary files ') || raw.startsWith('GIT binary patch')) {
      this.closeHunk();
      this.current.binary = true;
      return;
    }
    const header = HUNK_HEADER.exec(raw);
    if (header !== null) return this.startHunk(raw, header);
    this.addLine(raw);
  }

  /** A hunk is open until it has yielded the line counts its header promised. */
  private isHunkOpen(): boolean {
    return this.hunk !== null && (this.oldRemaining > 0 || this.newRemaining > 0);
  }

  private startFile(): void {
    this.closeHunk();
    this.current = { path: '', oldPath: null, binary: false, truncated: false, hunks: [] };
    this.files.push(this.current);
    this.linesUsed = 0;
    this.cap = MAX_LINES_PER_FILE;
  }

  private setOldPath(value: string): void {
    this.closeHunk();
    const path = stripPrefix(value);
    if (this.current !== null && path !== '/dev/null') this.current.oldPath = path;
  }

  private setNewPath(value: string): void {
    this.closeHunk();
    if (this.current === null) return;
    const path = stripPrefix(value);
    if (path !== '/dev/null') this.current.path = path;
    else if (this.current.oldPath !== null) this.current.path = this.current.oldPath;
    this.cap = isNoise(this.current.path) ? MAX_LINES_PER_NOISE_FILE : MAX_LINES_PER_FILE;
  }

  private startHunk(raw: string, header: RegExpExecArray): void {
    this.closeHunk();
    if (this.current === null) return;
    if (this.linesUsed >= this.cap) {
      this.current.truncated = true;
      return;
    }
    this.oldNo = Number(header[1]);
    this.newNo = Number(header[3]);
    const oldCount = header[2] === undefined ? 1 : Number(header[2]);
    const newCount = header[4] === undefined ? 1 : Number(header[4]);
    this.oldRemaining = oldCount;
    this.newRemaining = newCount;
    this.hunk = {
      id: '',
      header: raw,
      old_start: this.oldNo,
      old_count: oldCount,
      new_start: this.newNo,
      new_count: newCount,
      additions: 0,
      deletions: 0,
      lines: [],
    };
  }

  private addLine(raw: string): void {
    if (this.hunk === null || this.current === null) return;
    if (raw.startsWith('\\')) return; // "\ No newline at end of file"
    // A blank context line is " " in a patch; "" only ever comes from the
    // trailing newline of the diff as a whole.
    if (raw === '') return;
    if (this.linesUsed >= this.cap) {
      this.current.truncated = true;
      this.closeHunk();
      return;
    }
    const line = this.buildLine(raw[0] ?? ' ', raw.slice(1));
    if (line === null) return;
    this.hunk.lines.push(line);
    this.linesUsed += 1;
  }

  private buildLine(marker: string, text: string): DiffLine | null {
    if (this.hunk === null) return null;
    const i = this.hunk.lines.length;
    if (marker === '+') {
      this.hunk.additions += 1;
      this.newRemaining -= 1;
      return { i, type: 'add', old: null, new: this.newNo++, text };
    }
    if (marker === '-') {
      this.hunk.deletions += 1;
      this.oldRemaining -= 1;
      return { i, type: 'del', old: this.oldNo++, new: null, text };
    }
    if (marker !== ' ') return null;
    this.oldRemaining -= 1;
    this.newRemaining -= 1;
    return { i, type: 'context' as LineType, old: this.oldNo++, new: this.newNo++, text };
  }

  private closeHunk(): void {
    if (this.hunk !== null && this.current !== null) this.current.hunks.push(this.hunk);
    this.hunk = null;
    this.oldRemaining = 0;
    this.newRemaining = 0;
  }
}

export function parseUnifiedDiff(patch: string): ParsedFile[] {
  return new DiffParser().parse(patch);
}

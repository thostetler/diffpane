# diffpane — data & API contract

Frozen interface between the Node backend (`src/`) and the browser UI (`ui/`).
Neither side may change a shape here without updating this file.

Backend owns: diff parsing, JSON persistence, HTTP serving.
UI owns: everything rendered in the browser.

The JSON wire format is snake_case throughout, including where the TypeScript
that produces it is not.

## Runtime layout

Session data lives in `~/.cache/diffpane/<repo>/<slug>/` (or under
`$XDG_CACHE_HOME`):

```
meta.json       generated  scope + repo info
hunks.json      generated  parsed diff
review.json     authored   optional narrative, supplied with --review
comments.json   mutable    written by the UI via the API
```

The UI is served from `ui/`. `index.html` is served at `/`, everything else in
`ui/` is served verbatim under `/assets/<name>`. The UI is a static page: no
build step, no bundler, no CDN or network fetches at runtime. Vanilla JS
(ES2022 modules are fine), vanilla CSS. It must work in current Firefox and
Chrome.

## meta.json

```json
{
  "repo": "nectar",
  "repo_root": "/home/tim/code/nectar",
  "slug": "2026-08-27-search-cache",
  "scope": "branch",
  "base": "origin/main",
  "head": "HEAD",
  "diff_cmd": "git diff origin/main...HEAD",
  "generated_at": "2026-08-27T14:02:11Z",
  "totals": { "files": 14, "additions": 402, "deletions": 88 }
}
```

`scope` is one of `branch`, `working`, `staged`, `range`, `commit`.

## hunks.json

```json
{
  "files": [
    {
      "id": "f0",
      "path": "src/search/cache.ts",
      "old_path": "src/search/cache.ts",
      "status": "modified",
      "additions": 61,
      "deletions": 4,
      "binary": false,
      "noise": false,
      "language": "typescript",
      "truncated": false,
      "hunks": [
        {
          "id": "f0h0",
          "header": "@@ -12,7 +12,9 @@ export function get(",
          "old_start": 12,
          "old_count": 7,
          "new_start": 12,
          "new_count": 9,
          "additions": 3,
          "deletions": 1,
          "lines": [
            { "i": 0, "type": "context", "old": 12, "new": 12, "text": "  const k = key(q);" },
            { "i": 1, "type": "del",     "old": 13, "new": null, "text": "  return map.get(k);" },
            { "i": 2, "type": "add",     "old": null, "new": 13, "text": "  const hit = map.get(k);" }
          ]
        }
      ]
    }
  ]
}
```

Notes for the UI:

- `status` ∈ `added`, `modified`, `deleted`, `renamed`, `copied`.
- `noise: true` marks lockfiles, snapshots, generated output, `dist/`, minified
  files. Render these collapsed and visually de-emphasised.
- `binary: true` files have `hunks: []`. Show a one-line stub, no diff body.
- `truncated: true` means the file exceeded the line cap and only the first
  hunks are present. Show a "diff truncated" note.
- `line.type` ∈ `context`, `add`, `del`. `old`/`new` are 1-based or `null`.
- `line.i` is the index within the hunk and is stable. Use it for DOM ids.
- `text` is the raw line **without** the leading `+`/`-`/space marker, tabs
  intact. The UI is responsible for escaping and tab rendering.
- Files and hunks are already in a sensible order; do not re-sort.

### Known deviations from `git diff`

diffpane produces *a* correct diff, not always the *same* diff your `git` would
print. The differences are all cases where more than one answer is valid, and
they are pinned deliberately so a review shows the same thing on every machine
regardless of the reader's git config.

- **Algorithm.** Histogram, always. `gix-imara-diff`'s Myers does not match
  git's, and no git config reproduces it.
- **Rename tracking.** 50% similarity, no copy detection, always — git's
  defaults, but not read from `diff.renames`.
- **Edit-script ties.** Histogram sometimes picks a different, equally short
  edit script than git's. Seen once in 469 commits: one add/del pair moved.
- **Rename ties.** When one deleted file is a plausible source for several added
  ones, gix and git may pick different destinations. Seen once in 469 commits.
- **Rename similarity.** git scores candidates with `diffcore-delta` hashing,
  gix with the diff itself, so a pair near the 50% threshold can land on
  opposite sides. Seen once in 469 commits — git said 66%, gix said no.
- **Function context.** Reimplemented to git's default `xfuncname`. A file with
  a `diff=<lang>` gitattribute gets that default instead of its language driver.
- **Type change to a submodule.** Rendered as the `Subproject commit` side only;
  git shows both sides.

`spike/parity.sh` asserts byte-equality against the TypeScript pipeline (which
shells out to real git) everywhere except the three commits listed in its
allow-list, which are the three cases above.

## review.json

Authored by Claude. The UI must degrade gracefully if it is missing or if
chapters reference hunks that do not exist.

```json
{
  "title": "Search result caching",
  "story": "Adds an LRU in front of the search endpoint...",
  "chapters": [
    {
      "id": "c1",
      "title": "Cache layer",
      "intent": "New LRU keyed on normalised query.",
      "why": "Repeat queries were hitting Solr on every keystroke.",
      "hunks": ["f0h0", "f0h1", "f2h0"],
      "size": "+61/-4",
      "flags": ["No eviction test yet."]
    }
  ],
  "file_notes": { "src/search/cache.ts": "New module, read top-down." }
}
```

- Chapters are the primary organising unit. Render in array order.
- A chapter's `hunks` may span multiple files; group by file within a chapter,
  preserving the order given.
- Any hunk in `hunks.json` not claimed by a chapter goes into a synthetic
  trailing chapter with id `unsorted`, title "Everything else".
- `flags` is an optional array of short strings — render as warnings.
- `file_notes` is optional; key is `file.path`.

## comments.json

The UI never writes this file directly; it goes through the API. Shape:

```json
{
  "comments": [
    {
      "id": "c-7f3a1b",
      "anchor": {
        "kind": "line",
        "file": "src/search/cache.ts",
        "hunk": "f0h0",
        "side": "new",
        "line": 13,
        "chapter": "c1"
      },
      "verdict": "fix",
      "body": "Unbounded — needs a max size.",
      "created_at": "2026-08-27T14:11:02Z",
      "updated_at": "2026-08-27T14:11:02Z",
      "resolved": false
    }
  ],
  "progress": { "c1": "reviewed" },
  "overall": { "verdict": "fix", "body": "Mostly good, two blockers." },
  "submitted": false,
  "submitted_at": null
}
```

- `anchor.kind` ∈ `line`, `file`, `chapter`, `overall`.
  - `line` requires `file`, `hunk`, `side`, `line`.
  - `file` requires `file`. `chapter` requires `chapter`.
- `anchor.side` ∈ `old`, `new`. For a `del` line use `old`, otherwise `new`.
- `verdict` ∈ `ok`, `fix`, `question`.
- `progress` values ∈ `unreviewed`, `reviewed`.

## HTTP API

JSON in, JSON out, `Content-Type: application/json`. Errors return
`{ "error": "<message>" }` with a 4xx/5xx status.

| Method | Path | Body | Returns |
|---|---|---|---|
| GET | `/api/review` | — | `{ meta, hunks, review, comments }` — one shot, load everything |
| POST | `/api/comments` | `{ anchor, verdict, body }` | the created comment |
| PATCH | `/api/comments/<id>` | any of `{ verdict, body, resolved }` | the updated comment |
| DELETE | `/api/comments/<id>` | — | `{ ok: true }` |
| PUT | `/api/progress` | `{ chapter, state }` | `{ progress }` |
| PUT | `/api/overall` | `{ verdict, body }` | `{ overall }` |
| POST | `/api/submit` | `{ overall? }` | `{ submitted: true, submitted_at }` |
| GET | `/api/state` | — | `comments.json` contents (cheap poll) |

`PUT /api/progress` only accepts a `chapter` that `review.json` declares, plus
the synthetic `unsorted`; anything else is a 400.

Every mutating call returns after the write has hit disk, so the UI can treat a
2xx as durable. On any non-2xx the UI must surface the error inline and keep the
user's text — never silently drop a comment.

### Authentication

The server binds `127.0.0.1` only and mints a random token per run. Loopback
alone is not access control: any page in any tab can reach `127.0.0.1`, so the
token is what actually gates the review.

- `GET /` requires `?t=<token>` and responds with a
  `diffpane_token` cookie (`Path=/`, `SameSite=Strict`).
- `/assets/*` accepts the token from either the query string or that cookie,
  because the browser requests `app.css` and `app.js` on its own.
- `/api/*` requires the token in an `X-Diffpane-Token` header, and nothing else.
  A custom header cannot be set cross-origin without a preflight, which is what
  blocks drive-by requests. **The cookie must never authorise the API** —
  cookies ride along automatically and would reintroduce CSRF.
- Mutating calls additionally require a JSON `Content-Type`, closing the
  simple-request path.
- Any request whose `Host` header is not a loopback literal is rejected, which
  defeats DNS rebinding.

There is no offline mode and no retry queue. The server owns the review and
exits for good once it ends, so there is never anything to reconnect to: a
change the server did not accept did not happen. The UI surfaces the failure,
keeps the user's text, and says so plainly. It must not mirror the payload into
`localStorage` — that is the repo's source on a shared `127.0.0.1` origin.


## UI requirements

Layout: sticky left sidebar (chapter nav + progress), scrolling main column.

1. **Header** — title, story, scope line (`origin/main → HEAD`), totals,
   overall progress `3/7 reviewed`. A missing or unreadable `review.json` is a
   warning notice under the metadata, never the `<h1>`: the title names the
   thing being reviewed, not the state of the narrative file.
2. **Chapters** — each is a section: title, intent, why, size badge, flags.
   Chapter-level comment button. Mark-reviewed toggle that hits `PUT /api/progress`.
3. **Diff** — per file within a chapter: path header with status badge and
   ± counts, file-level comment button, collapse toggle. The header sticks
   under the page header while the file is on screen. Then hunks: monospace,
   two gutter columns (old / new line numbers), `add`/`del`/`context` colouring.
   Row backgrounds stay low-saturation and the code keeps the normal
   foreground; the gutter carries the stronger tint. Syntax highlighting is
   *not* required; correct, readable, aligned diff is.
4. **Folding** — collapsed by default, with a one-line summary and an expand
   control, when: `noise` is true; `status` is `added` or `deleted`; a hunk
   exceeds 40 lines (fold the middle, keep 8 lines of head/tail); the file
   exceeds 400 diff lines. Anything with a comment on it auto-expands.
   One global control that toggles between expand-all and collapse-all — its
   label follows the last action taken, not derived state, because collapse-all
   deliberately leaves commented files expanded.
5. **Commenting** — hovering a diff line reveals a `+` in the gutter; clicking
   it (or clicking the line number) opens an inline composer under that line:
   verdict segmented radio group (ok / fix / question), textarea, Save / Cancel,
   all on one row. Saved comments render inline under their anchor line, with
   edit / delete / resolve. Composers and comments are indented to the code
   column and connected to the line above with a coloured rule, so a comment
   visibly belongs to its line. `Cmd/Ctrl+Enter` saves, `Esc` cancels.
6. **Submit** — a footer bar: overall verdict as a segmented control
   (Approve / Request changes / Question, wire values `ok`/`fix`/`question`),
   notes, count of open comments, "Finish" button → `POST /api/submit`, then a
   done state telling the user to return to Claude. Confirm before submitting
   if any chapter is unreviewed.
7. **Sidebar** — chapter navigation doubles as review state: reviewed,
   has open feedback, or untouched, plus an open-comment count. The glyph is
   decoration; the button's accessible name carries the state.
8. **Keyboard** — `j`/`k` next/prev hunk, `n`/`p` next/prev chapter, `c` comment
   on focused line, `e` expand/collapse focused file, `?` shortcut help overlay.
9. **Density** — terse by default. The descriptions are short on purpose; do not
   pad the layout with whitespace that makes the page long to scan. Dark theme,
   monospace diff, system UI font for prose. The diff is the dominant surface:
   sidebar, header, file headers and footer are chrome and stay quiet.

Accessibility: real `<button>`s, visible focus rings, comment composer focus
trapped while open, `aria-expanded` on collapse toggles.

- The diff is a roving tabindex: exactly one row is a tab stop, and `j`/`k` move
  it. A 400-line diff must not be 400 tab stops in front of the submit footer.
- Line-number gutters are presentational (`aria-hidden`, not focusable). The
  named `+` button is the control, so it carries the line's accessible name.
- Add/del is never conveyed by colour alone: each row has a `+`/`-`/space marker
  column and an `aria-label` naming the line type.
- Errors are announced with `role="alert"` on insertion. `#app` must not carry
  `aria-live` — `render()` replaces that whole subtree and would re-announce the
  page on every save.
- Focus is carried across `render()`. Anything focusable that survives a
  re-render needs a stable `id` or `data-fk`.
- Segmented controls are real radio groups with the inputs visually hidden but
  still focusable. The focus ring is drawn on the group. Never hide a focusable
  control without moving its focus indicator somewhere visible.

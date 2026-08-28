![diffpane](assets/banner.png)

Review a git diff in your browser, comment on the lines, hand the feedback back.

`diffpane` opens a diff as a local web page, lets you click any line to leave a
comment, and prints the feedback to whatever called it — markdown for a human,
JSON for an agent. Nothing leaves your machine: no account, no service, no
runtime dependencies.

## Install

Not packaged yet — build from source:

```sh
git clone https://github.com/thostetler/diffpane
cd diffpane && cargo install --path .
```

Requires Rust 1.92+ and `git`.

## Use

From inside a repo:

```sh
diffpane                      # current branch vs its base
diffpane --working            # uncommitted changes
diffpane --staged             # staged changes
diffpane --range main..feat   # an explicit range
diffpane --commit a1b2c3d     # a single commit
diffpane -- src/search        # limit to a pathspec
```

It parses the diff, serves a page on `127.0.0.1`, opens your browser, and waits.
Click a line number to comment. Each comment takes a verdict — `ok`, `fix`, or
`question`. **Finish review** prints the report and exits.

```
diffpane  14 files, +402/-88
review    http://127.0.0.1:7777/?t=3f9c...
```

### Options

```
--base <ref>       base ref for the branch diff
--title <text>     human title for the review
--review <file>    narrative JSON (chapters + descriptions) to render
--out <file>       write the markdown report to a file
--json             print machine-readable feedback to stdout
--port <n>         preferred port (default 7777, walks forward if taken)
--no-open          do not open a browser
--timeout <sec>    give up waiting after N seconds (default 3600, 0 = never)
```

### Exit codes

| Code | Meaning |
|---|---|
| 0 | approved — submitted with no open `fix` comments, or nothing to review |
| 1 | changes requested |
| 2 | abandoned — timed out, or you quit without submitting |
| 3 | error |

```sh
diffpane --staged || { echo "review not clean"; exit 1; }
```

### Long diffs

Collapsed by default, with a one-line summary and an expand control:

- lockfiles, snapshots, `dist/`, minified and generated files
- added and deleted files in their entirety
- hunks over 40 lines fold their middle, keeping head and tail
- files over 400 diff lines

Anything you have commented on auto-expands. `j`/`k` move by hunk, `n`/`p` by
chapter, `c` comments on the focused line, `?` lists the shortcuts.

## Driving it from an agent

### Claude Code

```sh
diffpane --install-skill
```

Writes a skill to `~/.claude/skills/diffpane/` — `--skill-dir` puts it
elsewhere. Restart Claude Code, then:

```
/diffpane
```

The agent writes the chapter narrative, serves the diff, waits for you, and
works through your comments: questions answered before edits, then a report of
what it fixed, answered or skipped.

### Any other agent

Run with `--json` and read stdout:

```sh
diffpane --working --json
```

```json
{
  "outcome": "changes-requested",
  "overall": { "verdict": "fix", "body": "two blockers" },
  "comments": [
    {
      "verdict": "fix",
      "body": "Unbounded — needs a max size.",
      "file": "src/search/cache.ts",
      "line": 13,
      "kind": "line",
      "code": "+  const hit = map.get(k);"
    }
  ]
}
```

Each comment carries the file, the line, and the code it was pinned to.

### Chapters

`--review` takes a narrative that regroups the hunks into chapters:

```json
{
  "title": "Search result caching",
  "story": "Adds an LRU in front of the search endpoint.",
  "chapters": [
    {
      "id": "c1",
      "title": "Cache layer",
      "intent": "New LRU keyed on normalised query.",
      "why": "Repeat queries hit Solr on every keystroke.",
      "hunks": ["f0h0", "f0h1", "f2h0"],
      "size": "+61/-4",
      "flags": ["No eviction test yet."]
    }
  ]
}
```

Hunk ids are `f<file>h<hunk>`, in the order the diff reports them. Unclaimed
hunks land in a trailing "Everything else". Without `--review` the page is a
plain file-ordered diff.

## Security

Each run mints a random token: the page needs it in the URL, the API needs it in
an `X-Diffpane-Token` header, which cross-origin callers cannot set without a
preflight. Requests with a non-loopback `Host` are rejected, and mutations must
be JSON. Loopback binding alone is not access control: any page in any tab
can reach `127.0.0.1`.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The browser suite runs a real Chromium against the built binary:

```sh
pnpm install
pnpm exec playwright install chromium
pnpm test:ui
pnpm typecheck
```

Layout:

- `server/` — CLI and HTTP server. The manifest is at the repo root because
  the binary embeds `ui/` and `skills/`, which cargo packages only from there.
- `ui/` — vanilla HTML/CSS/JS, no build step, compiled into the binary.
  `DIFFPANE_UI_DIR=ui` serves it from disk instead.
- `parity/` — `parity.sh check-golden` asserts the diff output still matches
  `parity/golden/`.

Wire shapes are in `server/src/model.rs` and are frozen, snake_case.

Open the UI against fixture data without a repo:

```sh
diffpane --working --no-open   # then append &fixture=1 to the URL
```

## License

MIT

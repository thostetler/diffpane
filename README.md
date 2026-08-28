![diffpane](docs/banner.png)

Review a git diff in your browser, comment on the lines, hand the feedback back.

Coding agents produce diffs faster than anyone wants to read them in a terminal.
`diffpane` opens the diff as a local web page, lets you click any line to leave
a comment, and prints your feedback back to whatever called it — as markdown for
a human, or JSON for an agent.

Nothing leaves your machine. No account, no service, no runtime dependencies.

## Install

```sh
npx diffpane
```

Or install it properly:

```sh
npm install -g diffpane
```

Requires Node 22.6+ and `git`.

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
Click a line number to comment. Each comment gets a verdict — `ok`, `fix`, or
`question`. Hit **Finish review** and `diffpane` prints the report and exits.

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

So it composes:

```sh
diffpane --staged || { echo "review not clean"; exit 1; }
```

## Driving it from an agent

Run it with `--json` and read stdout:

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

The agent gets the comment, the file, the line, and the code it was pinned to.

### Chapters

A raw diff is in alphabetical file order, which is nobody's reading order. Pass
`--review` with a narrative and `diffpane` will regroup the hunks into chapters
and describe each one:

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

Hunk ids are `f<file>h<hunk>`, in the order the diff reports them. Anything a
chapter doesn't claim lands in a trailing "Everything else".

This is the natural job for the agent that wrote the code: it knows why it made
each change. Without `--review` the page still works, just as a plain
file-ordered diff.

## What it does with long diffs

Collapsed by default, with a one-line summary and an expand control:

- lockfiles, snapshots, `dist/`, minified and generated files
- added and deleted files in their entirety
- hunks over 40 lines fold their middle, keeping head and tail
- files over 400 diff lines

Anything you've commented on auto-expands. `j`/`k` move by hunk, `n`/`p` by
chapter, `c` comments on the focused line, `?` lists the shortcuts.

## Security

The server binds loopback only, but that is not access control on its own — any
page in any tab can reach `127.0.0.1`. So each run mints a random token: the
page needs it in the URL, and the API needs it in an `X-Diffpane-Token` header,
which cross-origin callers cannot set without a preflight. Requests with a
non-loopback `Host` are rejected, and mutations must be JSON.

Details in [docs/contract.md](docs/contract.md).

## Development

```sh
pnpm install
pnpm test          # node:test, no test framework
pnpm typecheck
pnpm build
```

Zero runtime dependencies. `src/` is the CLI and server; `ui/` is vanilla
HTML/CSS/JS with no build step. `docs/contract.md` is the frozen interface
between them — change it there first.

Open the UI against fixture data without a repo:

```sh
diffpane --working --no-open   # then append &fixture=1 to the URL
```

## License

MIT

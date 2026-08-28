---
name: diffpane
description: Human-in-the-loop code review — opens a diff as a local web page, chaptered and described, where the user clicks lines to leave comments, then feeds their feedback back into the session. Use when the user invokes /diffpane, asks to review changes themselves, wants to leave comments on a diff, or says "let me review this".
user-invocable: true
---

# diffpane

Hand the user the diff as a web page, wait for their comments, then act on them.

This is *human* review. If the user wants a second model's opinion, or a
narrated walkthrough in chat, this is the wrong skill.

The tool is a standalone CLI. Your job is the narrative and the follow-through;
the CLI does everything else. If `diffpane` is not on PATH, stop and tell the
user to `npm install -g github:thostetler/diffpane` — do not try to review the
diff yourself instead.

## 1. Write the narrative first

Read the diff you are about to serve — `git diff <base>...HEAD`, or whatever
scope the user asked for — plus enough surrounding code to explain *why*.

Hunk ids are `f<N>h<M>`: files numbered from 0 in the order `git diff` emits
them, hunks from 0 within each file. Count them from the diff you just read.

Write the narrative to a temp file, e.g. `/tmp/diffpane-review.json`:

```json
{
  "title": "Search result caching",
  "story": "Two or three sentences: what this change set is for, which seams it touches.",
  "chapters": [
    {
      "id": "c1",
      "title": "Cache layer",
      "intent": "New LRU keyed on normalised query.",
      "why": "Repeat queries hit Solr on every keystroke.",
      "hunks": ["f0h0", "f0h1", "f2h0"],
      "size": "+61/-4",
      "flags": ["No eviction test."]
    }
  ],
  "file_notes": { "src/search/cache.ts": "New module, read top-down." }
}
```

Rules — this is the part that makes the skill worth more than bare `diffpane`:

- **Chapters over files.** Regroup hunks into logical units and order them for
  reading: data model → core logic → callers/UI → tests → chore. Git's
  alphabetical file order is not a reading order.
- **Terse.** `intent` is one line, `why` one or two sentences. The diff is right
  there; never restate what it plainly shows.
- **Flags earn their place.** Mixed concerns, logic changed but tests didn't,
  risky or subtle spots. Not "this adds a function".
- Every hunk should land in exactly one chapter. Unclaimed hunks sweep into a
  trailing "Everything else" — fine for lockfiles, a smell for real code.
- Sweep `noise: true` files into one chore chapter; don't narrate them.
- Describe, don't defend. The review belongs to the user, not to you.

## 2. Serve it in the background

The CLI blocks until the user submits, which may be a long time. **Always run it
with `run_in_background: true`** so the session stays free and you are notified
on exit:

```bash
diffpane --working --json \
  --review /tmp/diffpane-review.json \
  --title "Search result caching" \
  > /tmp/diffpane-out.json 2> /tmp/diffpane-url.txt
```

Scope flags: `--working`, `--staged`, `--range a..b`, `--commit <sha>`,
`--base <ref>`, or nothing for branch-vs-base. Append `-- <pathspec>` to narrow.

It opens a browser itself. Wait a moment, then read `/tmp/diffpane-url.txt` and
give the user the URL in case it did not — then **stop talking**. Do not poll,
do not block, do not spawn a waiter.

## 3. Collect when it exits

You will be notified when the command finishes. Then read
`/tmp/diffpane-out.json`:

```json
{
  "outcome": "changes-requested",
  "overall": { "verdict": "fix", "body": "two blockers" },
  "comments": [
    { "verdict": "fix", "body": "...", "file": "src/a.ts", "line": 13,
      "kind": "line", "chapter": "c1", "code": "+  const hit = map.get(k);" }
  ]
}
```

`outcome` is `approved`, `changes-requested`, or `abandoned`. Exit codes: 0
approved, 1 changes requested, 2 abandoned, 3 error. An `abandoned` outcome
means they closed it without submitting — ask before assuming anything.

## 4. Act on it

Work `fix` comments first. **Answer `question` comments in chat before editing**
— a question is not necessarily a change request. Restate anything ambiguous
rather than guessing.

When done, report per comment: fixed / answered / skipped-and-why. Never
silently drop one.

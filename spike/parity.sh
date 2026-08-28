#!/usr/bin/env bash
# Assert the Rust diff layer produces the TypeScript pipeline's hunks.json.
#
#   parity.sh fixtures            every fixture repo and scope
#   parity.sh repo <dir> [n]      the last n non-merge commits of a real repo
#   parity.sh working <dir> [n]   index-vs-worktree over a real repo, n back
#   parity.sh staged <dir> [n]    tree-vs-index over a real repo, n back
#
# The comparison is exact after `jq -S` (key order only). Any difference is a
# parity failure, including hunk ids, which are positional.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=${PARITY_DIR:-$here/repos}
rust=$root/rust/target/release/dump-hunks

# The Rust side pins histogram, because gix's Myers does not match git's. So
# the reference has to run git on histogram too, or the assertion is measuring
# the algorithm difference instead of the port.
ts() {
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.algorithm GIT_CONFIG_VALUE_0=histogram \
    node --experimental-strip-types "$here/dump-ts.ts" "$@"
}

pass=0
fail=0

compare() { # $1 label, $2 repo, $3 ts args, $4 rust args (space separated)
  local label=$1 repo=$2 a b
  # Word splitting is wanted; globbing is not — a pathspec like `renamed*` must
  # reach git intact rather than being expanded against the current directory.
  set -o noglob
  # shellcheck disable=SC2206
  local ts_args=($3) rust_args=($4)
  set +o noglob
  a=$(ts "$repo" "${ts_args[@]}" | jq -S .)
  b=$("$rust" "$repo" "${rust_args[@]}" | jq -S .)
  if [ "$a" = "$b" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL $label"
    diff <(echo "$a") <(echo "$b") | head -40 || true
  fi
}

build() {
  (cd "$root/rust" && cargo build --release --bin dump-hunks -q)
}

run_fixtures() {
  local dir
  for mode in working staged committed; do
    dir=$work/fixture-$mode
    "$here/fixtures.sh" "$mode" "$dir" >/dev/null
    # shellcheck disable=SC1090
    . "$dir/FIXTURE_REFS"
    case $mode in
      working)
        compare "fixture working" "$dir" "" "--working"
        # Pathspecs: a plain path, a directory whose entry has a colon in it,
        # and a glob. The rename's two halves land on opposite sides of the
        # third, which is where git's filter-then-detect order shows.
        compare "fixture working path" "$dir" "-- plain.txt" "--working -- plain.txt"
        compare "fixture working dir" "$dir" "-- src" "--working -- src"
        compare "fixture working glob" "$dir" "-- renamed*" "--working -- renamed*"
        ;;
      staged)
        compare "fixture staged" "$dir" "--cached" "--staged"
        compare "fixture staged path" "$dir" "--cached -- plain.txt" "--staged -- plain.txt"
        compare "fixture staged glob" "$dir" "--cached -- renamed*" "--staged -- renamed*"
        ;;
      committed)
        compare "fixture committed" "$dir" "$base $head" "--range $base..$head"
        compare "fixture merge" "$dir" "$merge^ $merge" "--commit $merge"
        compare "fixture symmetric" "$dir" "$base...$head" "--range $base...$head"
        compare "fixture branch" "$dir" "$base...HEAD" "--base $base"
        # A root commit has no parent, so its base is git's empty tree.
        compare "fixture root commit" "$dir" \
          "4b825dc642cb6eb9a060e54bf8d69288fbee4904 $base" "--commit $base"
        ;;
    esac
  done
}

# Working scope over real content: clone, drop an older tree into the worktree,
# reset the index back to HEAD. The diff is then index-vs-worktree over content
# nobody wrote for this test.
run_working() { # $1 repo, $2 commits back
  local repo=$1 back=${2:-20} clone depth
  clone=$work/working-$(basename "$repo")
  # `local` expands all its arguments before assigning any of them, so anything
  # derived from $repo has to come after.
  depth=$(git -C "$repo" rev-list --count --first-parent HEAD)
  [ "$back" -lt "$depth" ] || back=$((depth - 1))
  rm -rf "$clone"
  git clone -qs "$repo" "$clone"
  git -C "$clone" checkout -q "HEAD~$back" -- .
  git -C "$clone" reset -q
  compare "$(basename "$repo") working ~$back" "$clone" "" "--working"
}

# Staged scope over real content: same clone trick, but leave the older tree in
# the index instead of unstaging it.
run_staged() { # $1 repo, $2 commits back
  local repo=$1 back=${2:-20} clone depth
  clone=$work/staged-$(basename "$repo")
  depth=$(git -C "$repo" rev-list --count --first-parent HEAD)
  [ "$back" -lt "$depth" ] || back=$((depth - 1))
  rm -rf "$clone"
  git clone -qs "$repo" "$clone"
  git -C "$clone" checkout -q "HEAD~$back" -- .
  compare "$(basename "$repo") staged ~$back" "$clone" "--cached" "--staged"
}

run_repo() { # $1 repo, $2 count
  local repo=$1 count=${2:-20} sha parent
  while read -r sha; do
    parent=$(git -C "$repo" rev-parse --verify --quiet "$sha^" || echo 4b825dc642cb6eb9a060e54bf8d69288fbee4904)
    compare "$(basename "$repo") $sha" "$repo" "$parent $sha" "--commit $sha"
  done < <(git -C "$repo" log --no-merges --format=%H -n "$count")
}

mkdir -p "$work"
build

case ${1:-fixtures} in
  fixtures) run_fixtures ;;
  repo) run_repo "${2:?usage: parity.sh repo <dir> [n]}" "${3:-20}" ;;
  working) run_working "${2:?usage: parity.sh working <dir> [n]}" "${3:-20}" ;;
  staged) run_staged "${2:?usage: parity.sh staged <dir> [n]}" "${3:-20}" ;;
  *) echo "usage: parity.sh <fixtures|repo|working|staged>" >&2; exit 2 ;;
esac

echo "parity: $pass passed, $fail failed"
[ "$fail" -eq 0 ]

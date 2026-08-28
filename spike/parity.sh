#!/usr/bin/env bash
# Assert the Rust diff layer produces the TypeScript pipeline's hunks.json.
#
#   parity.sh fixtures            both fixture repos, working and committed
#   parity.sh repo <dir> [n]      the last n non-merge commits of a real repo
#   parity.sh working <dir> [n]   index-vs-worktree over a real repo, n back
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
  # Word splitting is wanted: these are refs and mode words, never paths.
  # shellcheck disable=SC2206
  local ts_args=($3) rust_args=($4)
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
  for mode in working committed; do
    dir=$work/fixture-$mode
    "$here/fixtures.sh" "$mode" "$dir" >/dev/null
    # shellcheck disable=SC1090
    . "$dir/FIXTURE_REFS"
    if [ "$mode" = working ]; then
      compare "fixture working" "$dir" "" "working"
    else
      compare "fixture committed" "$dir" "$base $head" "tree $base $head"
      compare "fixture merge" "$dir" "$merge^ $merge" "tree $merge^ $merge"
    fi
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
  compare "$(basename "$repo") working ~$back" "$clone" "" "working"
}

run_repo() { # $1 repo, $2 count
  local repo=$1 count=${2:-20} sha parent
  while read -r sha; do
    parent=$(git -C "$repo" rev-parse --verify --quiet "$sha^" || echo 4b825dc642cb6eb9a060e54bf8d69288fbee4904)
    compare "$(basename "$repo") $sha" "$repo" "$parent $sha" "tree $parent $sha"
  done < <(git -C "$repo" log --no-merges --format=%H -n "$count")
}

mkdir -p "$work"
build

case ${1:-fixtures} in
  fixtures) run_fixtures ;;
  repo) run_repo "${2:?usage: parity.sh repo <dir> [n]}" "${3:-20}" ;;
  working) run_working "${2:?usage: parity.sh working <dir> [n]}" "${3:-20}" ;;
  *) echo "usage: parity.sh <fixtures|repo|working>" >&2; exit 2 ;;
esac

echo "parity: $pass passed, $fail failed"
[ "$fail" -eq 0 ]

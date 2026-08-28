#!/usr/bin/env bash
# Assert the Rust diff layer produces the TypeScript pipeline's hunks.json.
#
#   parity.sh fixtures            every fixture repo and scope
#   parity.sh repo <dir> [n]      the last n non-merge commits of a real repo
#   parity.sh working <dir> [n]   index-vs-worktree over a real repo, n back
#   parity.sh staged <dir> [n]    tree-vs-index over a real repo, n back
#   parity.sh golden              rewrite the fixture golden files from Rust
#   parity.sh check-golden        assert the fixtures still match the goldens
#
# The comparison is exact after `jq -S` (key order only). Any difference is a
# parity failure, including hunk ids, which are positional.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=${PARITY_DIR:-$here/repos}
golden_dir=$here/golden
rust=$root/rust/target/release/dump-hunks

# The Rust side pins histogram, because gix's Myers does not match git's. So
# the reference has to run git on histogram too, or the assertion is measuring
# the algorithm difference instead of the port.
ts() {
  GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.algorithm GIT_CONFIG_VALUE_0=histogram \
    node --experimental-strip-types "$here/dump-ts.ts" "$@"
}

# Commits where gix and git both produce a valid diff but not the same one:
# an equally short histogram edit script, a rename tie broken the other way,
# and a rename git scores at 66% that gix scores under 50%. Each is explained
# in docs/contract.md under "Known deviations". A divergence anywhere else is a
# bug, so these are named individually rather than waved through by category.
known_divergence() { # $1 label
  case $1 in
    *2e75969ac1910179c7f4db6af17921b1acd230f0) return 0 ;;
    *9ab2fa1d8ba9a1b907a6db537039adfd4bc57297) return 0 ;;
    *83e1c849473044e4a1262708351d3b8475bb7bee) return 0 ;;
    *) return 1 ;;
  esac
}

pass=0
fail=0
allowed=0

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
  elif known_divergence "$label"; then
    allowed=$((allowed + 1))
    echo "ALLOWED $label"
  else
    fail=$((fail + 1))
    echo "FAIL $label"
    diff <(echo "$a") <(echo "$b") | head -40 || true
  fi
}

build() {
  (cd "$root/rust" && cargo build --release --bin dump-hunks -q)
}

# The TypeScript is the reference implementation, and it is going away. Golden
# files freeze what parity established while it was still here, so the fixture
# matrix keeps failing loudly on an unintended output change afterwards.
golden_file() { # $1 label
  echo "$golden_dir/$(echo "$1" | tr ' ' '-').json"
}

golden_write() { # $1 label, $2 repo, $3 ts args (unused), $4 rust args
  local file
  file=$(golden_file "$1")
  set -o noglob
  # shellcheck disable=SC2206
  local rust_args=($4)
  set +o noglob
  "$rust" "$2" "${rust_args[@]}" | jq -S . >"$file"
  echo "wrote $(basename "$file")"
}

golden_check() { # $1 label, $2 repo, $3 ts args (unused), $4 rust args
  local label=$1 file
  file=$(golden_file "$label")
  set -o noglob
  # shellcheck disable=SC2206
  local rust_args=($4)
  set +o noglob
  if [ ! -f "$file" ]; then
    fail=$((fail + 1))
    echo "FAIL $label (no golden file; run parity.sh golden)"
    return
  fi
  if "$rust" "$2" "${rust_args[@]}" | jq -S . | diff -q "$file" - >/dev/null; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL $label"
    "$rust" "$2" "${rust_args[@]}" | jq -S . | diff "$file" - | head -40 || true
  fi
}

run_fixtures() { # $1 per-case function, default `compare`
  local dir case_fn=${1:-compare}
  for mode in working staged committed; do
    dir=$work/fixture-$mode
    "$here/fixtures.sh" "$mode" "$dir" >/dev/null
    # shellcheck disable=SC1090
    . "$dir/FIXTURE_REFS"
    case $mode in
      working)
        "$case_fn" "fixture working" "$dir" "" "--working"
        # Pathspecs: a plain path, a directory whose entry has a colon in it,
        # and a glob. The rename's two halves land on opposite sides of the
        # third, which is where git's filter-then-detect order shows.
        "$case_fn" "fixture working path" "$dir" "-- plain.txt" "--working -- plain.txt"
        "$case_fn" "fixture working dir" "$dir" "-- src" "--working -- src"
        "$case_fn" "fixture working glob" "$dir" "-- renamed*" "--working -- renamed*"
        ;;
      staged)
        "$case_fn" "fixture staged" "$dir" "--cached" "--staged"
        "$case_fn" "fixture staged path" "$dir" "--cached -- plain.txt" "--staged -- plain.txt"
        "$case_fn" "fixture staged glob" "$dir" "--cached -- renamed*" "--staged -- renamed*"
        ;;
      committed)
        "$case_fn" "fixture committed" "$dir" "$base $head" "--range $base..$head"
        "$case_fn" "fixture merge" "$dir" "$merge^ $merge" "--commit $merge"
        "$case_fn" "fixture symmetric" "$dir" "$base...$head" "--range $base...$head"
        "$case_fn" "fixture branch" "$dir" "$base...HEAD" "--base $base"
        # A root commit has no parent, so its base is git's empty tree.
        "$case_fn" "fixture root commit" "$dir" \
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

mkdir -p "$work" "$golden_dir"
build

case ${1:-fixtures} in
  fixtures) run_fixtures ;;
  golden) run_fixtures golden_write; exit 0 ;;
  check-golden) run_fixtures golden_check ;;
  repo) run_repo "${2:?usage: parity.sh repo <dir> [n]}" "${3:-20}" ;;
  working) run_working "${2:?usage: parity.sh working <dir> [n]}" "${3:-20}" ;;
  staged) run_staged "${2:?usage: parity.sh staged <dir> [n]}" "${3:-20}" ;;
  *)
    echo "usage: parity.sh <fixtures|golden|check-golden|repo|working|staged>" >&2
    exit 2
    ;;
esac

echo "parity: $pass passed, $fail failed, $allowed known divergences"
[ "$fail" -eq 0 ]

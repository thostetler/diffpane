#!/usr/bin/env bash
# Assert the Rust diff layer still produces the hunks.json it produced when the
# TypeScript was here to be compared against.
#
#   parity.sh check-golden        assert the fixtures still match the goldens
#   parity.sh golden              rewrite the golden files from Rust
#
# The comparison is exact after `jq -S` (key order only). Any difference is a
# failure, including hunk ids, which are positional. The TypeScript reference
# is gone; `parity/golden/` is what it left behind.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=${PARITY_DIR:-$here/repos}
golden_dir=$here/golden
manifest=$root/Cargo.toml

pass=0
fail=0

# Ask cargo where the build landed: CARGO_TARGET_DIR moves it, and assuming
# `target/release` would spawn a binary that is not there.
build() {
  cargo build --release --bin dump-hunks -q --manifest-path "$manifest"
  local target
  target=$(cargo metadata --no-deps --format-version 1 --manifest-path "$manifest" |
    jq -r .target_directory)
  dump_hunks=$target/release/dump-hunks
}

golden_file() { # $1 label
  echo "$golden_dir/$(echo "$1" | tr ' ' '-').json"
}

golden_write() { # $1 label, $2 repo, $3 dump-hunks args (space separated)
  local file
  file=$(golden_file "$1")
  set -o noglob
  # shellcheck disable=SC2206
  local hunk_args=($3)
  set +o noglob
  "$dump_hunks" "$2" "${hunk_args[@]}" | jq -S . >"$file"
  echo "wrote $(basename "$file")"
}

golden_check() { # $1 label, $2 repo, $3 dump-hunks args (space separated)
  local label=$1 file
  file=$(golden_file "$label")
  set -o noglob
  # shellcheck disable=SC2206
  local hunk_args=($3)
  set +o noglob
  if [ ! -f "$file" ]; then
    fail=$((fail + 1))
    echo "FAIL $label (no golden file; run parity.sh golden)"
    return
  fi
  if "$dump_hunks" "$2" "${hunk_args[@]}" | jq -S . | diff -q "$file" - >/dev/null; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL $label"
    "$dump_hunks" "$2" "${hunk_args[@]}" | jq -S . | diff "$file" - | head -40 || true
  fi
}

run_fixtures() { # $1 per-case function, default `golden_check`
  local dir case_fn=${1:-golden_check}
  for mode in working staged committed; do
    dir=$work/fixture-$mode
    "$here/fixtures.sh" "$mode" "$dir" >/dev/null
    # shellcheck disable=SC1090
    . "$dir/FIXTURE_REFS"
    case $mode in
      working)
        "$case_fn" "fixture working" "$dir" "--working"
        # Pathspecs: a plain path, a directory whose entry has a colon in it,
        # and a glob. The rename's two halves land on opposite sides of the
        # third, which is where git's filter-then-detect order shows.
        "$case_fn" "fixture working path" "$dir" "--working -- plain.txt"
        "$case_fn" "fixture working dir" "$dir" "--working -- src"
        "$case_fn" "fixture working glob" "$dir" "--working -- renamed*"
        ;;
      staged)
        "$case_fn" "fixture staged" "$dir" "--staged"
        "$case_fn" "fixture staged path" "$dir" "--staged -- plain.txt"
        "$case_fn" "fixture staged glob" "$dir" "--staged -- renamed*"
        ;;
      committed)
        "$case_fn" "fixture committed" "$dir" "--range $base..$head"
        "$case_fn" "fixture merge" "$dir" "--commit $merge"
        "$case_fn" "fixture symmetric" "$dir" "--range $base...$head"
        "$case_fn" "fixture branch" "$dir" "--base $base"
        "$case_fn" "fixture root commit" "$dir" "--commit $base"
        ;;
    esac
  done
}

mkdir -p "$work" "$golden_dir"
build

case ${1:-check-golden} in
  check-golden) run_fixtures golden_check ;;
  golden) run_fixtures golden_write; exit 0 ;;
  *)
    echo "usage: parity.sh <check-golden|golden>" >&2
    exit 2
    ;;
esac

echo "parity: $pass passed, $fail failed"
[ "$fail" -eq 0 ]

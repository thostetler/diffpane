#!/usr/bin/env bash
# Build a fixture repo exercising every diff shape diffpane has to survive.
#
#   fixtures.sh working   <dir>   changes left uncommitted (index vs worktree)
#   fixtures.sh staged    <dir>   changes staged, not committed (tree vs index)
#   fixtures.sh committed <dir>   changes committed on top of base (tree vs tree)
#
# Every mode applies the same edits, so the scopes are directly comparable.
set -euo pipefail

mode=${1:?usage: fixtures.sh <working|staged|committed> <dir>}
dir=${2:?usage: fixtures.sh <working|staged|committed> <dir>}

case $mode in
  working | staged | committed) ;;
  *) echo "usage: fixtures.sh <working|staged|committed> <dir>" >&2; exit 2 ;;
esac

rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"

git init -q -b main .
git config user.name 'Fixture'
git config user.email 'fixture@example.com'
git config commit.gpgsign false
# A clean/smudge filter, so the worktree bytes differ from the stored blob.
git config filter.redact.clean 'sed s/SECRET/REDACTED/'
git config filter.redact.smudge 'cat'

write_binary() { # $1 path, $2 seed byte
  printf '\x89PNG\r\n\x1a\n' >"$1"
  head -c 64 /dev/zero | tr '\0' "$2" >>"$1"
}

cat >.gitattributes <<'EOF'
*.crlf text eol=crlf
*.flt filter=redact
*.bin binary
EOF

cat >plain.txt <<'EOF'
alpha
bravo
charlie
delta
echo
foxtrot
golf
EOF

cat >doomed.txt <<'EOF'
this file gets deleted
second line
EOF

cat >moved.txt <<'EOF'
one
two
three
four
five
six
seven
eight
nine
ten
EOF

# Deliberately unlike moved.txt: identical sources would make git's rename
# pairing arbitrary, and an arbitrary pairing cannot be a parity assertion.
cat >moved-and-edited.txt <<'EOF'
uno
dos
tres
cuatro
cinco
seis
siete
ocho
nueve
diez
EOF

# A rename candidate that stays just under git's 50% similarity default.
cat >barely-similar.txt <<'EOF'
keep this line
alpha alpha alpha
bravo bravo bravo
charlie charlie charlie
delta delta delta
echo echo echo
EOF

printf 'first\r\nsecond\r\nthird\r\nfourth\r\n' >crlf.crlf

cat >filtered.flt <<'EOF'
token = SECRET
untouched line
EOF

cat >queries.sql <<'EOF'
-- lookup by identifier
SELECT * FROM docs WHERE id = 1;
-- legacy path, unused
SELECT * FROM docs_v1 WHERE id = 1;
SELECT count(*) FROM docs;
EOF

mkdir -p src
cat >'src/weird:name.txt' <<'EOF'
colon in the path
line two
EOF

cat >pnpm-lock.yaml <<'EOF'
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.0.0
        version: 1.3.0
EOF

cat >exec.sh <<'EOF'
#!/bin/sh
echo hello
EOF

write_binary logo.bin '\001'

git add -A
git commit -qm 'base'
base=$(git rev-parse HEAD)

if [ "$mode" = committed ]; then
  git checkout -q -b side
  printf 'side branch line\n' >side.txt
  git add side.txt
  git commit -qm 'side: add side.txt'
  git checkout -q main
  printf 'main branch line\n' >main-only.txt
  git add main-only.txt
  git commit -qm 'main: add main-only.txt'
  git merge -q --no-ff -m 'merge side into main' side
  merge=$(git rev-parse HEAD)
fi

apply_edits() {
  # text change, both directions, plus a hunk at the file head and tail
  cat >plain.txt <<'EOF'
alpha
bravo modified
charlie
delta
echo
foxtrot
golf
hotel
EOF

  printf 'brand new file\nwith two lines\n' >added.txt

  rm doomed.txt

  git mv moved.txt renamed.txt

  git mv moved-and-edited.txt renamed-and-edited.txt
  cat >renamed-and-edited.txt <<'EOF'
uno
dos CHANGED
tres
cuatro
cinco
seis
siete
ocho
nueve
diez
once
EOF

  # Rewrite most of the content: git should call this delete+add, not a rename.
  git rm -q barely-similar.txt
  cat >rewritten.txt <<'EOF'
keep this line
totally different content here
nothing in common with before
another unrelated line
yet another unrelated line
and one more for good measure
EOF
  git add rewritten.txt

  printf 'first\r\nsecond CHANGED\r\nthird\r\nfourth\r\nfifth\r\n' >crlf.crlf

  cat >filtered.flt <<'EOF'
token = SECRET
untouched line
appended line
EOF

  # Deleted lines starting with `--`, which serialise as `---` in a patch.
  cat >queries.sql <<'EOF'
-- lookup by identifier
SELECT * FROM docs WHERE id = 1;
SELECT count(*) FROM docs;
EOF

  cat >'src/weird:name.txt' <<'EOF'
colon in the path
line two rewritten
EOF

  cat >pnpm-lock.yaml <<'EOF'
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.1.0
        version: 1.3.0
      right-pad:
        specifier: ^2.0.0
        version: 2.0.1
EOF

  chmod +x exec.sh

  write_binary logo.bin '\002'
}

apply_edits

case $mode in
  committed)
    git add -A
    git commit -qm 'edits'
    {
      echo "base=$base"
      echo "head=$(git rev-parse HEAD)"
      echo "merge=${merge:-}"
    } >FIXTURE_REFS
    ;;
  staged)
    # `git diff --cached` is HEAD vs index, so every edit has to be staged.
    git add -A
    echo "base=$base" >FIXTURE_REFS
    ;;
  working)
    # `git diff` with no args is index vs worktree. `git mv` and `git rm` stage,
    # so unstage everything to put the whole change set in the worktree.
    git reset -q
    echo "base=$base" >FIXTURE_REFS
    ;;
esac

echo "$dir"

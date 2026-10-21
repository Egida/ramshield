#!/usr/bin/env bash
# Regression test for scripts/check_dev_paths.sh.
#
# The checker must reject a tracked file that hard-codes a developer home
# path, and accept the portable forms that replace one. The cases run
# against a throwaway git repo so the real tree is never modified.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
checker="$root/scripts/check_dev_paths.sh"
tmp=$(mktemp -d)
fail() { echo "FAIL: $*" >&2; exit 1; }

repo="$tmp/repo"
mkdir -p "$repo"
git -C "$repo" init -q
git -C "$repo" config user.name 'Dev Path Contract Test'
git -C "$repo" config user.email 'dev-path@example.invalid'
mkdir -p "$repo/scripts" "$repo/systemd/user"

commit_file() { # path content
  printf '%s\n' "$2" > "$repo/$1"
  git -C "$repo" add "$1"
  git -C "$repo" commit -qm "add $1"
}

# A clean tree passes.
commit_file scripts/ok.sh 'DIR="$HOME/project"'
(cd "$repo" && bash "$checker" "$repo") >/dev/null || fail 'clean tree was rejected'
echo 'PASS: portable path accepted'

# A developer home path in a script is rejected.
commit_file scripts/bad.sh 'DIR=/home/m/someone/project'
if (cd "$repo" && bash "$checker" "$repo") >"$tmp/bad.log" 2>&1; then
  fail 'developer path in a script was accepted'
fi
grep -Fq 'developer-specific absolute path' "$tmp/bad.log" || fail 'diagnostic missing'
grep -Fq 'scripts/bad.sh' "$tmp/bad.log" || fail 'offending file not named'
echo 'PASS: developer path in a script rejected'

# A developer home path in a systemd unit is rejected.
commit_file systemd/user/bad.service 'ExecStart=/home/m/someone/project/run.sh'
if (cd "$repo" && bash "$checker" "$repo") >"$tmp/unit.log" 2>&1; then
  fail 'developer path in a unit was accepted'
fi
grep -Fq 'systemd/user/bad.service' "$tmp/unit.log" || fail 'offending unit not named'
echo 'PASS: developer path in a unit rejected'

# The systemd specifier form is portable and must be accepted. This runs in
# a clean repo: the files that were rejected above are still tracked there.
clean="$tmp/clean"
mkdir -p "$clean/systemd/user" "$clean/scripts"
git -C "$clean" init -q
git -C "$clean" config user.name 'Dev Path Contract Test'
git -C "$clean" config user.email 'dev-path@example.invalid'
printf 'Environment=DIR=%%h/project\n' > "$clean/systemd/user/good.service"
printf 'DIR="$HOME/project"\n' > "$clean/scripts/good.sh"
git -C "$clean" add -A
git -C "$clean" commit -qm 'portable paths only'
(cd "$clean" && bash "$checker" "$clean") >/dev/null || fail 'systemd %h specifier was rejected'
echo 'PASS: systemd %h specifier accepted'

# An untracked file must not trip the checker: the scan is over tracked files.
printf 'DIR=/home/m/someone/project\n' > "$clean/scripts/untracked.sh"
(cd "$clean" && bash "$checker" "$clean") >/dev/null || fail 'untracked file tripped the checker'
echo 'PASS: untracked file ignored'

echo 'PASS: developer-path checker contract holds'

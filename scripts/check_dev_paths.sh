#!/usr/bin/env bash
# Fail if a tracked file hard-codes a developer-specific absolute path.
#
# A path like /home/m/vehicle_of_rationalism/... only ever works on the
# machine of the developer who wrote it. It leaks a personal path into a
# shipped unit file, breaks every other machine, and says nothing about
# what the file is supposed to do. The fix is always a path relative to
# the file, to $HOME, to a systemd specifier (%h), or an environment
# variable the operator sets.
#
# Only tracked files are scanned, and only the paths that are shipped or
# executed: units, scripts, manifests and source. Documentation may quote
# a path as a historical record.
set -euo pipefail
# The tree to scan. Defaults to this repository; the contract test passes a
# throwaway repo so the real tree is never modified.
root=${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
cd "$root"

# $HOME itself is not a defect: it is how a path becomes per-user.
pattern='/home/[a-zA-Z0-9._-]+/'

# The checker and its own contract test contain example paths on purpose:
# they are the fixtures the test feeds the checker. They are excluded, or
# the gate would fail on itself.
#
# Every tracked file is scanned except documentation. The earlier version
# listed file types by hand and so missed the 51 .py files, plus .html,
# .nft, .mjs, .c and .h - a path in any of them is just as broken. A
# pathspec list that has to be kept in step with the tree is a list that
# will fall behind the tree.
hits=$(git grep -nIE "$pattern" -- \
  . ':!*.md' \
  ':!scripts/check_dev_paths.sh' ':!scripts/test_check_dev_paths.sh' 2>/dev/null || true)

if [[ -n "$hits" ]]; then
  echo "ERROR: developer-specific absolute path(s) found in tracked files:" >&2
  echo "$hits" >&2
  echo "" >&2
  echo "Use a path relative to the file, \$HOME, a systemd specifier (%h), or an" >&2
  echo "environment variable the operator sets. See scripts/install.sh and" >&2
  echo "systemd/user/ramshield-backlog-dispatcher.service for the pattern." >&2
  exit 1
fi

echo "PASS: no developer-specific absolute path in tracked files (documentation excluded)"

#!/usr/bin/env bash
# Batch 8: Release/CI consistency gate.
# Run before every release candidate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PASS=0; FAIL=0
pass() { PASS=$((PASS+1)); echo "  ✅ $*"; }
fail() { FAIL=$((FAIL+1)); echo "  ❌ $*"; }

echo "═══ Release Consistency Gates ═══"

# 1. Workspace builds clean
if cargo build --release --locked --features full 2>/dev/null; then
  pass "cargo build (release, full features)"
else
  fail "cargo build failed"
fi

# 2. All tests pass
if cargo test --workspace --locked --features full 2>/dev/null; then
  pass "cargo test (entire workspace)"
else
  fail "cargo test failed"
fi

# 3. Benchmarks compile
if cargo bench --bench hot_paths 2>/dev/null; then
  pass "benchmarks compile"
else
  fail "benchmarks failed"
fi

# 4. CHANGELOG exists and has entry for this version
VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
if grep -q "$VERSION" CHANGELOG.md 2>/dev/null; then
  pass "CHANGELOG has v$VERSION entry"
else
  fail "CHANGELOG missing v$VERSION entry"
fi

# 5. Binary produced
if [[ -x target/release/ramshield ]]; then
  pass "release binary present"
  file target/release/ramshield
else
  fail "release binary missing"
fi

# 6. Config baseline is valid TOML
if python3 -c "import tomllib; tomllib.load(open('config.baseline.toml', 'rb'))" 2>/dev/null; then
  pass "config.baseline.toml valid"
else
  fail "config.baseline.toml malformed"
fi

# 7. Git is clean
if [[ -z "$(git status --porcelain 2>/dev/null)" ]]; then
  pass "working tree clean"
else
  fail "uncommitted changes — commit before release"
fi

echo
echo "═══ Results: $PASS pass, $FAIL fail ═══"
[[ "$FAIL" -eq 0 ]]
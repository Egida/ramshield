#!/usr/bin/env bash
# Authoritative production release gate for RamShield.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PASS=0; FAIL=0
pass(){ PASS=$((PASS+1)); echo "  ✅ $*"; }
fail(){ FAIL=$((FAIL+1)); echo "  ❌ $*"; }
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
echo "═══ RamShield release gate v$VERSION ═══"

if cargo fmt --all -- --check; then pass "cargo fmt"; else fail "cargo fmt"; fi
if cargo check --workspace --locked --all-targets --features full; then pass "cargo check"; else fail "cargo check"; fi
if cargo clippy --workspace --locked --all-targets --features full -- -D warnings; then pass "cargo clippy"; else fail "cargo clippy"; fi
if cargo test --workspace --locked --features full; then pass "workspace tests"; else fail "workspace tests"; fi

if grep -q "^version = \"$VERSION\"" Cargo.toml && grep -q "$VERSION" CHANGELOG.md; then pass "source/changelog version"; else fail "source/changelog version"; fi

# Active release artifacts must all carry the same version. Historical docs are excluded.
for f in deploy/k8s/deployment.yaml deploy/k8s/daemonset.yaml deploy/k8s/README.md docker/Dockerfile scripts/install.sh scripts/build_docker.sh; do
  if grep -q "$VERSION" "$f"; then pass "version $VERSION in $f"; else fail "stale version in $f"; fi
done
if grep -R -nE 'ghcr.io/grep999/ramshield:(0\.2|0\.3\.[012])([[:space:]"`]|$)' deploy docker scripts --exclude='*.md' >/dev/null 2>&1; then
  fail "stale active container tag found"
else
  pass "no stale active container tags"
fi

# K8s control surfaces are deliberately loopback-only; there is no ClusterIP Service.
if [[ -e deploy/k8s/service.yaml ]]; then fail "legacy ClusterIP service still present"; else pass "no service exposing loopback-only control plane"; fi
if grep -q 'tcp_addr = "127.0.0.1:7890"' deploy/k8s/configmap.yaml && grep -q 'http_addr = "127.0.0.1:9999"' deploy/k8s/configmap.yaml; then pass "server control surfaces loopback-only"; else fail "server control surfaces are not loopback-only"; fi
if grep -q 'enabled = true' deploy/k8s/configmap-node.yaml && grep -q 'mode = "drv"' deploy/k8s/configmap-node.yaml; then pass "node guard XDP config enabled"; else fail "node guard XDP config missing"; fi

if python3 - <<'PY'
import pathlib, tomllib
for p in pathlib.Path('.').rglob('*.toml'):
    tomllib.loads(p.read_text())
PY
then pass "all TOML parses"; else fail "TOML parse"; fi

if python3 - <<'PY'
import pathlib, yaml
for p in pathlib.Path('deploy/k8s').glob('*.yaml'):
    list(yaml.safe_load_all(p.read_text()))
PY
then pass "Kubernetes YAML parses"; else fail "Kubernetes YAML parse"; fi

if bash -n scripts/*.sh; then pass "shell syntax"; else fail "shell syntax"; fi
if python3 -m compileall -q scripts; then pass "Python syntax"; else fail "Python syntax"; fi

if [[ -x target/release/ramshield ]]; then pass "release binary present"; else fail "release binary missing"; fi
if [[ -z "$(git status --porcelain 2>/dev/null)" ]]; then pass "working tree clean"; else fail "working tree dirty"; fi

echo "═══ Result: $PASS pass, $FAIL fail ═══"
[[ "$FAIL" -eq 0 ]]

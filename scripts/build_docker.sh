#!/usr/bin/env bash
# Assemble a clean Docker build context (prebuilt binary + baseline config
# ONLY — the prod config never enters the context) and build the image.
# Usage: scripts/build_docker.sh [tag]   (default tag: 0.2.0)
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${1:-0.2.0}"
BIN=target/release/ramshield
[ -x "$BIN" ] || { echo "no release binary at $BIN — cargo build --release --locked --features full first"; exit 1; }

CTX="$(mktemp -d)"
trap 'rm -rf "$CTX"' EXIT
cp "$BIN" "$CTX/ramshield"
cp config.baseline.toml "$CTX/"
cp LICENSE "$CTX/" 2>/dev/null || true
cp docker/Dockerfile "$CTX/"

# Never let a repo-root build leak the prod config
cp /dev/null "$CTX/.dockerignore" && echo 'config.prod*' >> "$CTX/.dockerignore"

docker build -q -t "ramshield:$TAG" -f "$CTX/Dockerfile" "$CTX"
docker tag "ramshield:$TAG" "idunnoman/ramshield:$TAG"
echo "built: ramshield:$TAG (idunnoman/ramshield:$TAG)"
# push only when explicitly asked and logged in:
if [ "${PUSH:-0}" = "1" ]; then
  docker push "idunnoman/ramshield:$TAG"
fi

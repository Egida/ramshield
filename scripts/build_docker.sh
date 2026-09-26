#!/usr/bin/env bash
# Build a slim runtime Docker image from the host-built release binary.
# Usage: scripts/build_docker.sh [tag] [--no-xdp]
#   [tag]           — Docker image tag (default 0.3.0)
#   --no-xdp        — build with XDP disabled (config.toml patched at build time)
set -euo pipefail

cd "$(dirname "$0")/.."

TAG="${1:-0.3.0}"
shift 1 || true

XDP_ENABLED="true"
if [[ "${1:-}" == "--no-xdp" ]]; then
    XDP_ENABLED="false"
fi

BIN=target/release/ramshield
[ -x "$BIN" ] || { echo "no release binary at $BIN — cargo build --release --locked --features full first"; exit 1; }

CTX="$(mktemp -d)"
trap 'rm -rf "$CTX"' EXIT

cp "$BIN" "$CTX/ramshield"
cp config.baseline.toml "$CTX/"
cp docker/Dockerfile "$CTX/Dockerfile"
cp LICENSE "$CTX/" 2>/dev/null || true

# Never let a repo-root build leak the prod config or target/ cache
cat > "$CTX/.dockerignore" <<'EOF'
config.prod.toml
config.debug.toml
target/
*.log
EOF

echo "Building ramshield:$TAG (git=$(git rev-parse HEAD), xdp=$XDP_ENABLED)"
docker build \
  --network=host \
  --build-arg XDP_ENABLED="$XDP_ENABLED" \
  -t "ramshield:$TAG" \
  -t "grep999/ramshield:$TAG" \
  -f "$CTX/Dockerfile" \
  "$CTX"
echo "built: ramshield:$TAG (grep999/ramshield:$TAG)"
# push only when explicitly asked and logged in:
if [ "${PUSH:-0}" = "1" ]; then
  docker push "grep999/ramshield:$TAG"
fi
#!/usr/bin/env bash
# Compatibility entrypoint; the canonical enterprise installer is scripts/install.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
exec "$ROOT/scripts/install.sh" "$@"

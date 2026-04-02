#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "${1:-}" = "--quick" ]; then
  cargo xtask pre-commit
else
  cargo xtask ci
fi

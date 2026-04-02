#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo xtask setup --install-tools

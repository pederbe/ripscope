#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --quiet --locked --manifest-path tools/release-changelog/Cargo.toml -- \
  finalize "${1:?Usage: bash scripts/prepare-release.sh VERSION}" "$(date -u +%F)"

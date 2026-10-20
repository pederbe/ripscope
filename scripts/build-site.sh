#!/usr/bin/env bash
# Build locally. Release builds require an exact, clean version tag; neither mode deploys.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --quiet --locked --manifest-path tools/site/Cargo.toml -- "$@"

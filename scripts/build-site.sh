#!/usr/bin/env bash
# Build locally. Deployment builds require a clean checkout of a released version; neither mode deploys.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --quiet --locked --manifest-path tools/site/Cargo.toml -- "$@"

#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --manifest-path tools/release-changelog/Cargo.toml -- --check
cargo clippy --locked --manifest-path tools/release-changelog/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path tools/release-changelog/Cargo.toml

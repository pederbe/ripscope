#!/usr/bin/env bash
# Build locally. Deployment builds require a clean checkout of a released version; neither mode deploys.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "${1:-}" = --cloudflare ]; then
    # Bootstrap the tools used by the existing site build in Workers Builds.
    if ! command -v cargo >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o /tmp/ripscope-rustup.sh
        sh /tmp/ripscope-rustup.sh -y --profile minimal
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    mkdir -p target/site-tools
    curl --proto '=https' -fsSL \
        https://github.com/rust-lang/mdBook/releases/download/v0.5.4/mdbook-v0.5.4-x86_64-unknown-linux-gnu.tar.gz \
        -o target/site-tools/mdbook.tar.gz
    echo '3f28de05dafca9d0f2eab99c662116b0e37b89b1d96a08f8f430b9eeae958cd7  target/site-tools/mdbook.tar.gz' | sha256sum -c -
    tar -xzf target/site-tools/mdbook.tar.gz -C target/site-tools mdbook
    export MDBOOK="$PWD/target/site-tools/mdbook"
    git fetch --tags --depth=1
    # The release branch moves only after publication. Keep download links on
    # that version while documentation continues to build from main.
    git fetch --depth=1 origin refs/heads/release:refs/remotes/origin/release
    RIPSCOPE_SITE_RELEASE_TAG=$(git describe --tags --exact-match refs/remotes/origin/release)
    export RIPSCOPE_SITE_RELEASE_TAG
    npm ci --ignore-scripts --no-audit --no-fund --prefix tools/site
    set -- --deploy
fi
exec cargo run --quiet --locked --manifest-path tools/site/Cargo.toml -- "$@"

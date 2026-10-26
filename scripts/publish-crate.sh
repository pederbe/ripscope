#!/bin/bash
# Verify the source package before authentication, then resume an immutable upload.
# usage: bash scripts/publish-crate.sh VERSION (dry-run|publish)
set -euo pipefail
version=${1:?expected version}
mode=${2:?expected dry-run or publish}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$mode" = dry-run || "$mode" = publish ]]
cargo metadata --no-deps --format-version 1 --locked |
    jq -e --arg version "$version" '.packages[] | select(.name == "ripscope") | .version == $version' >/dev/null
work=$(mktemp -d)
trap 'rm -r "$work"' EXIT
status=$(curl --proto '=https' --retry 2 --silent --show-error --location \
    --connect-timeout 10 --max-time 60 \
    --user-agent 'ripscope-release (https://github.com/pederbe/ripscope)' \
    --output "$work/version.json" --write-out '%{http_code}' \
    "https://crates.io/api/v1/crates/ripscope/$version")
case "$status" in
    200)
        checksum=$(jq -er --arg version "$version" '.version | select(.num == $version and .yanked == false) | .checksum' "$work/version.json")
        cargo package --locked --registry crates-io --no-verify -p ripscope
        printf '%s  %s\n' "$checksum" "${CARGO_TARGET_DIR:-target}/package/ripscope-$version.crate" | sha256sum -c -
        echo "ripscope $version is already published with matching source bytes."
        ;;
    404)
        files=$(cargo package --list --locked --registry crates-io -p ripscope)
        for required in Cargo.toml Cargo.lock README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE src/main.rs; do
            grep -qxF "$required" <<< "$files" || { echo "Source package lacks $required" >&2; exit 1; }
        done
        options=()
        [ "$mode" = publish ] || options+=(--dry-run)
        cargo publish --locked --registry crates-io -p ripscope "${options[@]}"
        ;;
    *) echo "Could not determine registry state for ripscope $version (HTTP $status)." >&2; exit 1 ;;
esac

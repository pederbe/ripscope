#!/usr/bin/env bash
# Prepare coupled release inputs before Renovate pushes the combined update.
set -euo pipefail
cd "$(dirname "$0")/../.."
old_pins=$(git show HEAD:.github/dependency-pins.json)
pins=.github/dependency-pins.json
replace() {
  OLD=$1 NEW=$2 perl -pi -e 's/\Q$ENV{OLD}\E/$ENV{NEW}/g' "${@:3}"
}
for key in zig zigbuild mdbook node lychee; do
  old=$(jq -er --arg key "$key" '.[$key]' <<< "$old_pins")
  new=$(jq -er --arg key "$key" '.[$key]' "$pins")
  [[ $new =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
  if [[ $old == "$new" ]]; then continue; fi
  case $key in
    zig|zigbuild)
      if [[ $key == zig ]]; then
        release=$(curl --proto '=https' --tlsv1.2 -fsSL https://ziglang.org/download/index.json | jq -er --arg version "$new" '.[$version]["x86_64-linux"]')
        url=$(jq -er .tarball <<< "$release")
        digest=$(jq -er '.shasum | select(test("^[0-9a-f]{64}$"))' <<< "$release")
      else
        url="https://github.com/rust-cross/cargo-zigbuild/releases/download/v$new/cargo-zigbuild-x86_64-unknown-linux-gnu.tar.xz"
        digest=$(curl --proto '=https' --tlsv1.2 -fsSL "https://api.github.com/repos/rust-cross/cargo-zigbuild/releases/tags/v$new" | jq -er '.assets[] | select(.name == "cargo-zigbuild-x86_64-unknown-linux-gnu.tar.xz") | .digest | select(test("^sha256:[0-9a-f]{64}$")) | ltrimstr("sha256:")')
      fi
      tmp=$(mktemp)
      jq --arg key "$key" --arg version "$new" --arg url "$url" --arg digest "$digest" '.[$key + "_version"] = $version | .[$key + "_url"] = $url | .[$key + "_archive_sha256"] = $digest' tools/release/linux-release-profile.json > "$tmp"
      cat "$tmp" > tools/release/linux-release-profile.json
      rm "$tmp"
      ;;
    node)
      replace "node-version: '$old'" "node-version: '$new'" .github/workflows/docs.yml
      ;;
    lychee)
      replace "lychee@$old" "lychee@$new" .github/workflows/docs.yml
      ;;
    mdbook)
      replace "$old" "$new" tools/site/README.md
      digest=$(curl --proto '=https' --tlsv1.2 -fsSL "https://api.github.com/repos/rust-lang/mdBook/releases/tags/v$new" | jq -er --arg name "mdbook-v$new-x86_64-unknown-linux-gnu.tar.gz" '.assets[] | select(.name == $name) | .digest | select(test("^sha256:[0-9a-f]{64}$")) | ltrimstr("sha256:")')
      replace "mdbook v$old" "mdbook v$new" tools/site/src/main.rs
      replace "/v$old/mdbook-v$old-" "/v$new/mdbook-v$new-" scripts/build-site.sh
      DIGEST=$digest perl -pi -e 'if (/mdbook\.tar\.gz.*sha256sum/) { s/[0-9a-f]{64}/$ENV{DIGEST}/ }' scripts/build-site.sh
      replace "mdbook@$old" "mdbook@$new" .github/workflows/*.yml
      ;;
  esac
done

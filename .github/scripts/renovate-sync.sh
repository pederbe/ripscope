#!/usr/bin/env bash
# Prepare coupled release inputs before Renovate pushes the combined update.
set -euo pipefail
cd "$(dirname "$0")/../.."
old_pins=$(git show HEAD:.github/dependency-pins.json)
pins=.github/dependency-pins.json
replace() {
  OLD=$1 NEW=$2 perl -pi -e 's/\Q$ENV{OLD}\E/$ENV{NEW}/g' "${@:3}"
}
for key in rust mold mdbook; do
  old=$(jq -er --arg key "$key" '.[$key]' <<< "$old_pins")
  new=$(jq -er --arg key "$key" '.[$key]' "$pins")
  [[ $new =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
  if [[ $old == "$new" ]]; then continue; fi
  case $key in
    rust)
      manifest=$(curl --proto '=https' --tlsv1.2 -fsSL "https://static.rust-lang.org/dist/channel-rust-$new.toml")
      commit=$(awk '/^\[pkg.rustc\]$/ { selected=1; next } /^\[/ { selected=0 } selected && /^git_commit_hash = / { gsub(/"/, "", $3); print $3 }' <<< "$manifest")
      [[ $commit =~ ^[0-9a-f]{40}$ ]]
      replace "$old" "$new" .github/workflows/*.yml tools/release/*-release-profile.json docs/linux-release.md docs/windows-release.md
      for profile in tools/release/*-release-profile.json; do
        tmp=$(mktemp)
        jq --arg commit "$commit" '.rust_commit = $commit' "$profile" > "$tmp"
        cat "$tmp" > "$profile"
        rm "$tmp"
      done
      ;;
    mold)
      url="https://github.com/rui314/mold/releases/download/v$new/mold-$new-x86_64-linux.tar.gz"
      digest=$(curl --proto '=https' --tlsv1.2 -fsSL "https://api.github.com/repos/rui314/mold/releases/tags/v$new" | jq -er --arg name "mold-$new-x86_64-linux.tar.gz" '.assets[] | select(.name == $name) | .digest | select(test("^sha256:[0-9a-f]{64}$")) | ltrimstr("sha256:")')
      replace "$old" "$new" tools/release/src/linux.rs docs/linux-release.md
      tmp=$(mktemp)
      jq --arg version "$new" --arg url "$url" --arg digest "$digest" '.mold_version = $version | .mold_url = $url | .mold_archive_sha256 = $digest' tools/release/linux-release-profile.json > "$tmp"
      cat "$tmp" > tools/release/linux-release-profile.json
      rm "$tmp"
      ;;
    mdbook)
      digest=$(curl --proto '=https' --tlsv1.2 -fsSL "https://api.github.com/repos/rust-lang/mdBook/releases/tags/v$new" | jq -er --arg name "mdbook-v$new-x86_64-unknown-linux-gnu.tar.gz" '.assets[] | select(.name == $name) | .digest | select(test("^sha256:[0-9a-f]{64}$")) | ltrimstr("sha256:")')
      replace "mdbook v$old" "mdbook v$new" tools/site/src/main.rs
      replace "/v$old/mdbook-v$old-" "/v$new/mdbook-v$new-" scripts/build-site.sh
      DIGEST=$digest perl -pi -e 'if (/mdbook\.tar\.gz.*sha256sum/) { s/[0-9a-f]{64}/$ENV{DIGEST}/ }' scripts/build-site.sh
      replace "mdbook@$old" "mdbook@$new" .github/workflows/*.yml
      ;;
  esac
done

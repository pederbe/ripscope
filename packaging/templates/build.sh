#!/bin/sh
set -eu
sha256sum -c MANIFEST.sha256
mkdir -p "$PREFIX/lib/ripscope" "$PREFIX/bin"
# Copy only the archive payload, never the recipe's build or test files.
for entry in bin contrib licenses BUILD.json DEPENDENCIES.json INSTALL.md \
    LICENSE-MIT LICENSE-APACHE MANIFEST.sha256 README.md; do
    cp -a "$entry" "$PREFIX/lib/ripscope/"
done
ln -s ../lib/ripscope/bin/ripscope "$PREFIX/bin/ripscope"

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
cd "$PREFIX/lib/ripscope"
# Find the environment's GCC runtime without changing global library lookup.
patchelf --set-rpath '$ORIGIN/../..' bin/ripscope
mv MANIFEST.sha256 ORIGINAL-ARCHIVE-MANIFEST.sha256
printf 'Conda added a relative GCC runtime path. Original archive SHA-256: @ARCHIVE_SHA256@\n' > CONDA-RELOCATION.txt
sha256sum bin/ripscope >> CONDA-RELOCATION.txt
find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 sha256sum > MANIFEST.sha256

#!/bin/sh
# Check an installed archive through Ripscope's public command line.
set -eu
version=${1:?version required}
binary=$(realpath "${2:?executable required}")
support=$(realpath "${3:?archive support directory required}")
test -x "$binary"
for document in BUILD.json DEPENDENCIES.json INSTALL.md LICENSE-MIT LICENSE-APACHE \
    licenses/SQLITE-NOTICE.txt licenses/rust/COPYRIGHT-library.html \
    contrib/vim/plugin/ripscope.vim contrib/vim/lua/ripscope.lua contrib/emacs/ripscope.el; do
    test -s "$support/$document"
done
test -n "$(find "$support/licenses/dependencies" -type f -size +0c -print -quit)"
test -n "$(find "$support/licenses/rust" -type f -size +0c -print -quit)"
(cd "$support" && sha256sum -c MANIFEST.sha256 >/dev/null)
if [ -f "$support/ORIGINAL-ARCHIVE-MANIFEST.sha256" ]; then
    # Only the executable may differ after runtime relocation.
    (cd "$support" && sed '\|  bin/ripscope$|d' ORIGINAL-ARCHIVE-MANIFEST.sha256 | sha256sum -c - >/dev/null)
fi
cmp "$binary" "$support/bin/ripscope"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
cd "$work"
"$binary" --version > actual
printf 'ripscope %s\n' "$version" > expected
cmp expected actual
printf 'int target(void) { return 7; }\nint caller(void) { return target(); }\n' > sample.c
"$binary" -bk -f index sample.c
"$binary" -dL -f index -1 target > actual
printf 'sample.c target 1 int target(void ) { return 7; }\n' > expected
cmp expected actual
"$binary" -dL -f index -3 target > actual
printf 'sample.c caller 2 int caller(void ) { return target(); }\n' > expected
cmp expected actual
rm sample.c
"$binary" -dL -f index -1 target > actual
printf 'sample.c target 1 int target(void ) { return 7; }\n' > expected
cmp expected actual
"$binary" -dL -f index -4 'return 7' > actual 2> diagnostic
test ! -s actual
grep -F 'cannot read sample.c' diagnostic >/dev/null
printf 'int replacement(void) { return 9; }\n' > sample.c
printf 'r\nq\n' | "$binary" -dl -f index > session
"$binary" -dL -f index -1 replacement > actual
printf 'sample.c replacement 1 int replacement(void ) { return 9; }\n' > expected
cmp expected actual
"$binary" -dL -f index -1 target > actual
test ! -s actual
"$binary" --verify -d -f index > before
printf 'r\nq\n' | "$binary" -dl -f index > session
"$binary" --verify -d -f index > after
cmp before after
printf 'Installed version, notices, stored/live queries and Refresh passed\n'

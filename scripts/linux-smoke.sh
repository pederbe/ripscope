#!/bin/sh
# Exercise the exact archive on a runtime-only local filesystem.
set -eu
cd /work
archive=${1:?archive filename required}
tar -xzf "/input/$archive"
package=/work/${archive%.tar.gz}
cd "$package"
sha256sum -c MANIFEST.sha256
test -s contrib/emacs/vendor/COPYING
test -s contrib/emacs/vendor/xcscope.el
for tool in rustc cargo cc gcc clang mold sqlite3; do
    if command -v "$tool" >/dev/null 2>&1; then
        echo "Unexpected development/database tool: $tool" >&2
        exit 1
    fi
done
cat /etc/os-release
ldd --version
ldd ./bin/ripscope
binary=$package/bin/ripscope
"$binary" --help
for ending in lf crlf; do
    mkdir "/work/$ending"
    cd "/work/$ending"
    if [ "$ending" = lf ]; then eol='\n'; else eol='\r\n'; fi
    printf "int target(void) { return 7; }$eol" > sample.c
    printf "int caller(void) { return target(); }$eol" >> sample.c
    printf "int extra(void) { return 7; }$eol" > extra.c
    "$binary" -bk -f index sample.c extra.c
    "$binary" -dL -f index -1 target > definition
    printf 'sample.c target 1 int target(void ) { return 7; }\n' > expected
    cmp definition expected
    "$binary" -dL -f index -4 'return 7' > text
    printf "extra.c <unknown> 1 int extra(void) { return 7; }$eol" > expected
    printf "sample.c <unknown> 1 int target(void) { return 7; }$eol" >> expected
    cmp text expected
    rm sample.c
    "$binary" -dL -f index -1 target > definition
    printf 'sample.c target 1 int target(void ) { return 7; }\n' > expected
    cmp definition expected
    "$binary" -dL -f index -4 'return 7' > text 2> warning
    printf "extra.c <unknown> 1 int extra(void) { return 7; }$eol" > expected
    cmp text expected
    grep -F 'cannot read sample.c' warning
    printf "int replacement(void) { return 9; }$eol" > sample.c
    printf '1target\nr\n1replacement\nr\nq\n' | "$binary" -dl -f index > session
    printf '>> cscope: 1 lines\nsample.c target 1 int target(void ) { return 7; }\n>> \n>> cscope: 1 lines\nsample.c replacement 1 int replacement(void ) { return 9; }\n>> \n>> ' > expected
    cmp session expected
    "$binary" --verify -d -f index > before
    printf 'r\nq\n' | "$binary" -dl -f index > unchanged
    "$binary" --verify -d -f index > after
    cmp before after
done
echo 'Archive LF/CRLF stored/live/warning/Refresh smoke passed'

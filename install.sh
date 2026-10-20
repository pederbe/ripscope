#!/bin/sh
# curl -fsSL https://ripscope.com/install.sh | sh
# Uninstall: curl -fsSL https://ripscope.com/install.sh | sh -s -- --uninstall
# Settings: RIPSCOPE_VERSION, RIPSCOPE_INSTALL_DIR, RIPSCOPE_RELEASES (mirror),
# RIPSCOPE_EDITORS (all, the default, or none to leave the editors alone).
# The complete function must arrive before installation can begin.
ripscope_install() (
    set -eu
    umask 077
    say() { printf '%s\n' "$*"; }
    fail() { printf 'ripscope install: %s\n' "$*" >&2; exit 1; }
    has() { command -v "$1" >/dev/null 2>&1; }
    safe_path() {
        case $1 in
            ''|/*|.*|*[!A-Za-z0-9_+./-]*|*/../*|*/..|*/./*|*/.) return 1 ;;
        esac
    }
    [ -n "${HOME:-}" ] || fail 'HOME is not set.'
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    case $data in /*) ;; *) data=$HOME/.local/share ;; esac
    # Where Vim and Neovim load packages from on their own, and Emacs's integration.
    vim_home=$HOME/.vim
    if [ ! -e "$HOME/.vimrc" ] && [ ! -e "$HOME/.vim/vimrc" ] &&
        [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/vim/vimrc" ]; then
        vim_home=${XDG_CONFIG_HOME:-$HOME/.config}/vim
    fi
    vim_package=$vim_home/pack/ripscope
    neovim_package=$data/nvim/site/pack/ripscope
    emacs_directory=$data/ripscope/emacs
    case ${1:-} in
        --uninstall)
            dir=${RIPSCOPE_INSTALL_DIR:-$HOME/.local/bin}
            if [ -f "$dir/ripscope" ] && [ ! -L "$dir/ripscope" ]; then
                rm -f "$dir/ripscope"
                say "Removed $dir/ripscope"
            fi
            for directory in "$dir"/.ripscope-notices-* "$vim_package" "$HOME/.vim/pack/ripscope" \
                "$neovim_package" "$emacs_directory"; do
                if [ -d "$directory" ] && [ ! -L "$directory" ]; then
                    rm -rf "$directory"
                    say "Removed $directory"
                fi
            done
            rmdir "$data/ripscope" 2>/dev/null || :
            case ${RIPSCOPE_CACHE_DIR:-} in
                /*) cache=$RIPSCOPE_CACHE_DIR ;;
                *) cache=${XDG_CACHE_HOME:-$HOME/.cache}/ripscope ;;
            esac
            say "Indexes the editors made remain in $cache (RIPSCOPE_CACHE_DIR names another folder when set). Delete them too if you no longer need them."
            exit 0 ;;
        '') ;;
        *) fail "unknown option: $1" ;;
    esac
    editors=${RIPSCOPE_EDITORS:-all}
    case $editors in all|none) ;; *) fail 'set RIPSCOPE_EDITORS to all or none.' ;; esac
    case $(uname -s):$(uname -m) in
        Linux:x86_64) ;;
        *) fail 'the archive supports x86-64 Linux with glibc 2.28 or newer.' ;;
    esac
    if has ldd && ldd --version 2>&1 | grep -qi musl; then
        fail 'the prebuilt archive requires glibc; build from source on musl.'
    fi
    for tool in curl tar gzip sha256sum awk find sort uniq cmp cp mv mktemp; do
        has "$tool" || fail "please install $tool first."
    done
    releases=${RIPSCOPE_RELEASES:-https://github.com/pederbe/ripscope/releases}
    case $releases in
        https://*) fetch() { curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL "$1" -o "$2"; } ;;
        *) [ -n "${RIPSCOPE_RELEASES:-}" ] || fail 'a release mirror must be explicit.'
           fetch() { curl -fsSL "$1" -o "$2"; } ;;
    esac
    version=${RIPSCOPE_VERSION:-}
    if [ -z "$version" ]; then
        case $releases in
            https://*) latest=$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSLI -o /dev/null -w '%{url_effective}' "$releases/latest") ;;
            *) latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$releases/latest") ;;
        esac
        case $latest in */tag/v*) version=${latest##*/tag/v} ;; esac
    fi
    version=${version#v}
    case $version in ''|.*|*[!A-Za-z0-9.-]*) fail 'set RIPSCOPE_VERSION to a release version, such as 0.1.0.' ;; esac
    dir=${RIPSCOPE_INSTALL_DIR:-$HOME/.local/bin}
    mkdir -p "$dir"
    dir=$(cd "$dir" && pwd -P)
    destination=$dir/ripscope
    [ ! -L "$destination" ] && { [ ! -e "$destination" ] || [ -f "$destination" ]; } ||
        fail "refusing nonregular or symbolic-link destination: $destination"
    name=ripscope-$version-x86_64-linux-gnu
    download=$(mktemp -d)
    staging=
    commit_attempted=no
    cleanup() {
        status=$?
        trap - 0
        if [ -n "$staging" ]; then
            if [ "$commit_attempted" = no ]; then
                rm -rf "$staging" || say "Could not remove owned staging: $staging"
            else
                say "Retained notices and editor files: $staging"
            fi
        fi
        rm -rf "$download" || say "Could not remove download directory: $download"
        exit "$status"
    }
    trap cleanup 0
    trap 'exit 130' INT
    trap 'exit 143' TERM
    say "Downloading Ripscope $version..."
    fetch "$releases/download/v$version/$name.tar.gz" "$download/archive.tar.gz" || fail 'archive download failed.'
    fetch "$releases/download/v$version/$name.tar.gz.sha256" "$download/checksum" || fail 'checksum download failed.'
    expected=$(awk -v file="$name.tar.gz" 'NF == 2 && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ && $2 == file { count++; hash=tolower($1) } END { if (count == 1 && NR == 1) print hash; else exit 1 }' "$download/checksum") || fail 'invalid archive checksum record.'
    actual=$(sha256sum "$download/archive.tar.gz"); actual=${actual%% *}
    [ "$actual" = "$expected" ] || fail 'archive checksum mismatch; previous installation preserved.'
    tar --quoting-style=escape -tzf "$download/archive.tar.gz" > "$download/entries" || fail 'cannot list archive.'
    while IFS= read -r entry; do
        safe_path "$entry" || fail "unsafe archive path: $entry"
        case $entry in "$name"|"$name/"|"$name/"*) ;; *) fail 'unexpected archive root.' ;; esac
    done < "$download/entries"
    [ -z "$(sort "$download/entries" | uniq -d)" ] || fail 'duplicate archive entry.'
    tar --quoting-style=escape -tvzf "$download/archive.tar.gz" > "$download/types" || fail 'cannot inspect archive.'
    awk 'substr($0,1,1) != "-" && substr($0,1,1) != "d" { exit 1 }' "$download/types" || fail 'archive contains a linked or special entry.'
    mkdir "$download/unpacked"
    tar --no-same-owner --no-same-permissions -xzf "$download/archive.tar.gz" -C "$download/unpacked"
    package=$download/unpacked/$name
    for file in bin/ripscope LICENSE-MIT LICENSE-APACHE DEPENDENCIES.json BUILD.json MANIFEST.sha256 \
        licenses/SQLITE-NOTICE.txt licenses/rust/COPYRIGHT-library.html \
        contrib/emacs/ripscope.el contrib/emacs/ripscope-xcscope.el contrib/emacs/vendor/xcscope.el \
        contrib/emacs/vendor/COPYING contrib/vim/plugin/ripscope.vim contrib/vim/plugin/ripscope.lua \
        contrib/vim/lua/ripscope.lua contrib/vim/lua/ripscope/session.lua \
        contrib/vim/lua/ripscope/project.lua; do
        [ -f "$package/$file" ] && [ ! -L "$package/$file" ] && [ -s "$package/$file" ] || fail "missing mandatory archive file: $file"
    done
    for tree in rust dependencies; do
        notices=$package/licenses/$tree
        [ -d "$notices" ] && [ ! -L "$notices" ] || fail "missing $tree licence notices."
        [ -n "$(find "$notices" -type f ! -name COPYRIGHT-library.html -size +0c -print -quit)" ] || fail "missing nonempty $tree licence payloads."
        [ -z "$(find "$notices" -type f -empty -print -quit)" ] || fail "empty $tree licence payload."
    done
    awk 'length($1) != 64 || $1 ~ /[^0-9a-f]/ || $0 !~ /^[0-9a-f]+  [A-Za-z0-9_+./-]+$/ { exit 1 }' "$package/MANIFEST.sha256" || fail 'invalid archive manifest.'
    : > "$download/manifest-files"
    while IFS= read -r record; do
        entry=${record#*  }
        safe_path "$entry" || fail 'unsafe manifest path.'
        printf '%s\n' "$entry" >> "$download/manifest-files"
    done < "$package/MANIFEST.sha256"
    (cd "$package" && find . -type f ! -path './MANIFEST.sha256' -printf '%P\n' | sort) > "$download/actual-files"
    sort "$download/manifest-files" > "$download/expected-files"
    cmp -s "$download/actual-files" "$download/expected-files" || fail 'archive inventory mismatch.'
    (cd "$package" && sha256sum -c MANIFEST.sha256 >/dev/null) || fail 'archive file checksum mismatch.'
    binary_hash=$(sha256sum "$package/bin/ripscope"); binary_hash=${binary_hash%% *}
    grep -F "\"binary_sha256\": \"$binary_hash\"" "$package/BUILD.json" >/dev/null || fail 'binary identity differs from BUILD.json.'
    staging=$(mktemp -d "$dir/.ripscope-notices-$version.XXXXXX")
    cp -R "$package/." "$staging/"
    [ -z "$(find "$staging" ! -type f ! -type d -print)" ] || fail 'copied support material contains a link or special entry.'
    (cd "$staging" && sha256sum -c MANIFEST.sha256 >/dev/null) || fail 'staged file checksum mismatch.'
    chmod 755 "$staging/bin/ripscope"
    [ "$("$staging/bin/ripscope" --version)" = "ripscope $version" ] || fail 'staged executable version mismatch.'
    mv "$staging/MANIFEST.sha256" "$staging/ORIGINAL-ARCHIVE-MANIFEST.sha256"
    printf 'Original archive manifest: bin/ripscope moved to %s\nExecutable SHA-256: %s\n' "$destination" "$binary_hash" > "$staging/INSTALLATION.txt"
    [ ! -L "$destination" ] && { [ ! -e "$destination" ] || [ -f "$destination" ]; } || fail "destination changed: $destination"
    # From this point an interrupt can leave a committed executable. Keep its notices.
    commit_attempted=yes
    mv -fT "$staging/bin/ripscope" "$destination" || fail "replacement failed; inspect $destination and $staging"
    actual=$(sha256sum "$destination"); actual=${actual%% *}
    [ "$actual" = "$binary_hash" ] || fail "replacement completed but verification failed; inspect $destination and $staging"
    rmdir "$staging/bin" || say "Installed successfully; empty staging directory retained: $staging/bin"
    say "Installed Ripscope $version: $destination"
    # Replace editor directory $2 with a copy of $1, staged two levels up, where no
    # editor loads it.
    place() {
        [ ! -L "$2" ] && [ ! -L "$(dirname "$2")" ] || fail "refusing symbolic-link editor directory: $2"
        mkdir -p "$(dirname "$2")"
        next=$(mktemp -d "$(dirname "$(dirname "$2")")/.ripscope-editor.XXXXXX")
        cp -R "$1/." "$next/"
        rm -rf "$2"
        mv "$next" "$2"
    }
    if [ "$editors" = all ]; then
        place "$staging/contrib/vim" "$vim_package/start/ripscope"
        place "$staging/contrib/vim" "$neovim_package/start/ripscope"
        place "$staging/contrib/emacs" "$emacs_directory"
        case $emacs_directory in
            "$HOME"/*) shown="~${emacs_directory#"$HOME"}" ;;
            *) shown=$emacs_directory ;;
        esac
        shown=$(printf '%s' "$shown" | sed 's/[\\"]/\\&/g')
        say "Vim and Neovim load Ripscope by themselves. Open a C file in your project and"
        say "search with :cs find g NAME in Vim, or :RipscopeFind g NAME in Neovim."
        say "For Emacs, add this line to your init file:"
        say "  (when (load \"$shown/ripscope\" t) (ripscope-setup))"
        if [ -d "$data/nvim/lazy/lazy.nvim" ]; then
            # lazy.nvim resets the package path, so Neovim loads only what its list names.
            say "Neovim with lazy.nvim loads only the plugins in its list. Add this to it:"
            say "  { dir = vim.fn.stdpath('data') .. '/site/pack/ripscope/start/ripscope' },"
        fi
    fi
    say "Add $dir to PATH in your shell configuration if needed."
)
ripscope_install "$@"

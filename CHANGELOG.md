# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the
index format or command-line behavior, and its entry says so.

## [Unreleased]

### Editors

- The Emacs package is Ripscope's own and searches through xref: `M-.` finds a
  definition and `M-?` the uses of a symbol in C buffers, and results open in the xref
  buffer. All nine searches are commands, such as `ripscope-find-callers`, and
  `ripscope-command-map` holds them on xcscope's keys for a prefix such as `C-c s`.
  Result paths may hold spaces. It replaces the bundled xcscope, which no longer ships,
  and needs Emacs 28.1 or newer and no other package.
- The Emacs settings are now `ripscope-include-directories`, `ripscope-auto-refresh`,
  `ripscope-executable` and `ripscope-timeout`. `M-x ripscope-index-project` indexes a
  project again and `M-x ripscope-stop-sessions` stops the sessions.
- The Emacs package is licensed MIT or Apache-2.0, as the rest of Ripscope is, so
  release archives and system packages are too.

## [0.1.1] - 2026-10-20

### Freshness

- On Windows, a session's next search after a build writes hundreds of files into the
  project no longer waits about half a second. When the file watcher reports lost
  events, the session refreshes at once.

### Editors

- A project reached through a symbolic link and by its own path shares one index.
- `:RipscopeInstall` and `M-x ripscope-install` leave the decision to the installer. Run
  with `--plugin` (`-Plugin` on Windows) from a checkout, it installs the checkout's
  version unless the executable found is that version or newer, or one it did not put
  there. A prerelease now counts as older than its release in every editor.
- Neovim settings that choose an index or sources build it on the first search until
  it is published with those sources, not only when the file is missing.
- `ripscope --project PATH` takes an index build's `-f`, `-k`, `-R`, `-i` and files, and
  judges publication by them.
- The Windows uninstaller names the cache folder Ripscope uses when
  `RIPSCOPE_CACHE_DIR` is `\cache` or another path without a drive.

## [0.1.0] - 2026-10-20

The first release. Ripscope answers cscope's searches over C source from the command
line, Vim, Neovim and Emacs, on x86-64 Linux and Windows.

### Searches

- All of cscope's line-mode searches: symbols, definitions, functions called, callers,
  text, egrep patterns, files, includers and assignments. They run as one-shot queries
  (`-L`) or in line-oriented sessions (`-l`), with cscope's output format and session
  protocol. `-C` matches ASCII letters in either case.
- Each name takes the function or macro it sits in as its context, within its own
  file, so callers and callees come from where each call is written.
- Corrections read constructs that cscope misreads, such as `extern "C"` blocks,
  macros with CR LF line endings and pointer declarators.
- Assignment search also finds array initializers, subscript stores, and `++` and `--`
  on a named variable.
- Text and egrep searches read the files as they are now.
- An optional JSON interface (`--json`) keeps paths with spaces and non-ASCII bytes
  exact.

See [differences from cscope](https://ripscope.com/docs/reference/differences-from-cscope)
and the [benchmarks](https://ripscope.com/docs/reference/benchmarks).

### Editors

- Integrations for Vim, through its built-in cscope support, for Neovim, and for
  Emacs, through xcscope.
- One-step setup. A plugin-manager line (lazy.nvim, vim-plug, or `use-package :vc` on
  Emacs 30 and newer) installs the integration and the executable of the same version.
- Each search finds its project: the nearest directory with `cscope.files`, else the
  Git repository, else the editor's current directory. The first search builds the
  project's index in your cache directory, and later searches keep it current.
  `ripscope --project PATH` reports the project and its index, and
  `RIPSCOPE_CACHE_DIR` moves the cache.

### Freshness

- Index builds from explicit files, file lists (`-i`), `cscope.files` or directory
  discovery (`-R`), following literal includes through `-I` directories. Each update
  applies the selection again, so added, removed and renamed files are picked up.
- An update analyses again only the files whose size, times or identity changed, and
  commits in one SQLite transaction. Queries read a complete revision while another
  process updates.
- A session watches the files its index was made from and refreshes after a change,
  before the next query. No build or compilation database is needed.
- `--verify` checks the index and reads every file again, and `-bu` rebuilds a damaged
  index.

### Platforms

- x86-64 Linux with glibc 2.28 or newer, and x86-64 Windows.
- The index lives on local storage. The
  [limitations](https://ripscope.com/docs/reference/limitations) list the details.

### Installation

- `install.sh` and `install.ps1` verify and install the executable, set up Vim and
  Neovim, and print the line Emacs needs. `--uninstall` (`-Uninstall` in PowerShell)
  removes them again.
- Release archives for Linux and Windows, and Debian and RPM packages, each with a
  SHA-256 checksum.
- Source builds with Cargo.

[Unreleased]: https://github.com/pederbe/ripscope/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/pederbe/ripscope/releases/tag/v0.1.1
[0.1.0]: https://github.com/pederbe/ripscope/releases/tag/v0.1.0

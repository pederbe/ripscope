# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the
index format or command-line behavior, and its entry says so.

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

[0.1.0]: https://github.com/pederbe/ripscope/releases/tag/v0.1.0

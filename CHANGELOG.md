# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the
index format or command-line behavior, and its entry says so.

## [Unreleased]

## [0.2.1] - 2026-10-26

### Other changes

- Automate weekly releases and generated changelogs (#41) ([ab70727](https://github.com/pederbe/ripscope/commit/ab70727))
- Update quay.io/pypa/manylinux_2_28_x86_64:latest Docker digest to e280835 (#36) ([eb23569](https://github.com/pederbe/ripscope/commit/eb23569))
- Update daily dependencies ([7b61e01](https://github.com/pederbe/ripscope/commit/7b61e01))
- Configure Renovate dependency updates with automerge (#32) ([13c9dbf](https://github.com/pederbe/ripscope/commit/13c9dbf))
- Update dependency wrangler to v4.152.0 ([73b0444](https://github.com/pederbe/ripscope/commit/73b0444))
- Update daily dependencies ([e9ac7ce](https://github.com/pederbe/ripscope/commit/e9ac7ce))
- Track Node Lychee and hinted Action pins in daily updates ([a11b137](https://github.com/pederbe/ripscope/commit/a11b137))
- Update daily dependencies ([ec98f7d](https://github.com/pederbe/ripscope/commit/ec98f7d))
- Synchronize release guide commands with dependency tool pins ([9674660](https://github.com/pederbe/ripscope/commit/9674660))
- Complete mdBook pin synchronization and constrain notify exception ([10e9999](https://github.com/pederbe/ripscope/commit/10e9999))
- Read compiler identity from the rustc manifest section ([ea3eb75](https://github.com/pederbe/ripscope/commit/ea3eb75))
- Batch dependency ecosystems and release tool pins with Renovate ([3447882](https://github.com/pederbe/ripscope/commit/3447882))
- Link the production website build settings ([c640fc0](https://github.com/pederbe/ripscope/commit/c640fc0))
- Deploy website changes automatically through Cloudflare Builds ([e774558](https://github.com/pederbe/ripscope/commit/e774558))
- Ask a Windows early stop's include questions on several threads ([8665aac](https://github.com/pederbe/ripscope/commit/8665aac))
- Add GitHub Sponsors funding link (#22) ([fc0489f](https://github.com/pederbe/ripscope/commit/fc0489f))
- Keep the racy metadata test inside its window on slow runners ([ecb035d](https://github.com/pederbe/ripscope/commit/ecb035d))
- Redirect /favicon.ico to the SVG icon ([af63ad9](https://github.com/pederbe/ripscope/commit/af63ad9))
- Merge Dependabot pull requests automatically once CI passes ([88e58b6](https://github.com/pederbe/ripscope/commit/88e58b6))
- Format sha2 0.11 digests as hex with base16ct ([490104f](https://github.com/pederbe/ripscope/commit/490104f))
- Simplify the pull request template ([7bd922e](https://github.com/pederbe/ripscope/commit/7bd922e))
- Bump the cargo group across 4 directories with 4 updates ([bcda8e1](https://github.com/pederbe/ripscope/commit/bcda8e1))
- Bump the github-actions group with 3 updates ([adee858](https://github.com/pederbe/ripscope/commit/adee858))
- Bump wrangler from 4.141.0 to 4.151.1 in /tools/site in the npm group ([ee5cf77](https://github.com/pederbe/ripscope/commit/ee5cf77))
- Wait for the killed descendant in the reference timeout test ([9e56430](https://github.com/pederbe/ripscope/commit/9e56430))
- Enable monthly grouped Dependabot version updates ([7cff0dd](https://github.com/pederbe/ripscope/commit/7cff0dd))
- State the Emacs package's licence above its commentary ([6fbbed1](https://github.com/pederbe/ripscope/commit/6fbbed1))

[Full comparison](https://github.com/pederbe/ripscope/compare/v0.2.0...v0.2.1)

## [0.2.0] - 2026-10-20

This release changes only the Emacs integration. Emacs users need to update their
settings and, for xcscope's keys, add one line, as described below. The index format,
the command line, the JSON interface and the Vim and Neovim plugins are unchanged.

### Editors

- The Emacs package is Ripscope's own and searches through xref: `M-.` finds a
  definition and `M-?` the uses of a symbol in C buffers, and results open in the xref
  buffer. All nine searches are commands, such as `ripscope-find-callers`, and
  `ripscope-command-map` holds them on xcscope's keys. To keep `C-c s g` and the rest,
  add `(define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)` to your init
  file. Result paths may hold spaces. It replaces the bundled xcscope, which no longer ships,
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

[Unreleased]: https://github.com/pederbe/ripscope/compare/v0.2.1...HEAD
[0.2.0]: https://github.com/pederbe/ripscope/releases/tag/v0.2.0
[0.1.1]: https://github.com/pederbe/ripscope/releases/tag/v0.1.1
[0.1.0]: https://github.com/pederbe/ripscope/releases/tag/v0.1.0

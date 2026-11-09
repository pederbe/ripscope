# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the
index format or command-line behavior, and its entry says so.

## [0.2.7] - 2026-11-09

### Changed

- Document stable source installation with `cargo install --locked ripscope`.
  Prepare cargo-binstall mappings for the Linux and Windows release archives.
  These mappings become available when the updated crate is published.

### Development

- Check generated release notes in the recording workflow, avoiding duplicate CI
  approval failures and a separate dispatched check workflow.

- Run existing release-tool checks without application rebuilds for release-tool-only
  changes, retaining broader coverage for application and shared-fixture changes.

- Use Windows PowerShell's default module paths for loaded-module inspection.

- Dispatch package delivery, Windows package checks, website refresh and release
  notes directly after automated publication, using their existing workflows.

### Other changes

- Update taiki-e/install-action digest to cfdb7ed (#116) ([1b336f5](https://github.com/pederbe/ripscope/commit/1b336f5))
- ci: Synchronize the WinGet fork with a restricted credential (#115) ([3346843](https://github.com/pederbe/ripscope/commit/3346843))
- ci: Submit current WinGet schemas with Komac (#114) ([11fe2df](https://github.com/pederbe/ripscope/commit/11fe2df))
- ci: Preserve current schemas through stable WinGetCreate (#113) ([40b633a](https://github.com/pederbe/ripscope/commit/40b633a))
- ci: Download WinGetCreate through the authenticated CLI (#112) ([63b5828](https://github.com/pederbe/ripscope/commit/63b5828))
- ci: Submit and complete WinGet releases automatically (#111) ([ca97dba](https://github.com/pederbe/ripscope/commit/ca97dba))
- Update quay.io/pypa/manylinux_2_28_x86_64:latest Docker digest to 9555ba6 (#110) ([caa4ea9](https://github.com/pederbe/ripscope/commit/caa4ea9))
- Select release-tool checks only for their inputs (#109) ([a05593c](https://github.com/pederbe/ripscope/commit/a05593c))
- Update daily dependencies (#108) ([c0b29ce](https://github.com/pederbe/ripscope/commit/c0b29ce))
- Update daily dependencies (#106) ([00a5c8b](https://github.com/pederbe/ripscope/commit/00a5c8b))
- Update daily dependencies (#105) ([c4413a2](https://github.com/pederbe/ripscope/commit/c4413a2))
- Prepare cargo-binstall discovery and stable Cargo installation (#104) ([f687856](https://github.com/pederbe/ripscope/commit/f687856))
- Remove redundant reads from release recovery (#103) ([1e8cd3b](https://github.com/pederbe/ripscope/commit/1e8cd3b))
- Classify CI changes in one paths-filter pass (#102) ([cc11973](https://github.com/pederbe/ripscope/commit/cc11973))
- Simplify generated changelog checks and suppress duplicate CI (#100) ([b9f1954](https://github.com/pederbe/ripscope/commit/b9f1954))
- Select existing release-tool checks without application rebuilds (#99) ([536fe06](https://github.com/pederbe/ripscope/commit/536fe06))
- Record published changelog for v0.2.6 (#98) ([fa3f6fa](https://github.com/pederbe/ripscope/commit/fa3f6fa))
- Dispatch automatic release follow-ups explicitly (#97) ([83f37c6](https://github.com/pederbe/ripscope/commit/83f37c6))
- Record published changelog for v0.2.6 (#96) ([9084765](https://github.com/pederbe/ripscope/commit/9084765))

[Full comparison](https://github.com/pederbe/ripscope/compare/v0.2.6...v0.2.7)

## [0.2.6] - 2026-11-04

### Changed

- Reduce repeated copying and decoding when building indexes, running editor
  sessions and writing search results.

- Update package and archive installation examples to the published 0.2.4
  release, and remove the fixed version from the installer-option explanation.

- Link the README's status section to the latest releases instead of naming a
  fixed current version.

- Use plain Linux and Windows names throughout the README, website and
  documentation. Supported build targets are unchanged.

### Development

- Check Windows package installation, upgrades and removal through WinGet
  against the published archives.

- Include workflow fixtures in release test sources and initialize the Windows
  compiler environment directly through PowerShell.

- Create executable test scripts in an isolated process so parallel launches
  cannot retain writable handles and cause intermittent "Text file busy" failures.

- Release builds follow stable Rust and the hosted Windows developer toolchain,
  recording the actual versions in each archive instead of requiring old versions.

- Follow rolling stable Rust and install the standard analysis, source, formatting and lint components with the project toolchain.

### Added

- Signed apt and dnf repositories provide Linux installation and updates through
  the system package manager.

- `ripscope lsp` is a language server for C. Helix, Neovim's built-in client and other
  editors that speak the Language Server Protocol can go to definitions, find
  references, show incoming and outgoing calls and search workspace symbols with no
  plugin. It shares each project's index with the Vim, Neovim and Emacs integrations.

### Fixed

- Editor searches work when your home directory is on NFS or another network file
  system. The project index now goes to `/var/tmp/ripscope-UID`. Before, the first
  search failed unless `RIPSCOPE_CACHE_DIR` was set.

### Other changes

- Complete replacement release notes (#95) ([dd8e7a5](https://github.com/pederbe/ripscope/commit/dd8e7a5))
- Repair Linux and Windows release verification (#93) ([2e4981f](https://github.com/pederbe/ripscope/commit/2e4981f))
- Update Rust crate toml to v1.2.0 (#94) ([6f59a71](https://github.com/pederbe/ripscope/commit/6f59a71))
- Update daily dependencies (#88) ([2b96eea](https://github.com/pederbe/ripscope/commit/2b96eea))
- Isolate executable test-script creation (#92) ([c024419](https://github.com/pederbe/ripscope/commit/c024419))
- Update installation examples to the published release (#91) ([1326c65](https://github.com/pederbe/ripscope/commit/1326c65))
- Make README release status version-neutral (#90) ([eab6cbb](https://github.com/pederbe/ripscope/commit/eab6cbb))
- Use plain Linux and Windows names in documentation (#89) ([256c341](https://github.com/pederbe/ripscope/commit/256c341))
- Use stable Rust and installed Windows release tools (#86) ([c434dac](https://github.com/pederbe/ripscope/commit/c434dac))
- Update daily dependencies (#87) ([a83cc32](https://github.com/pederbe/ripscope/commit/a83cc32))
- Follow rolling stable Rust for development and CI (#85) ([fad7229](https://github.com/pederbe/ripscope/commit/fad7229))
- Check the language server through Neovim's built-in client (#84) ([f4237a7](https://github.com/pederbe/ripscope/commit/f4237a7))
- Serve editors over the Language Server Protocol (#83) ([4dda991](https://github.com/pederbe/ripscope/commit/4dda991))
- Keep the editor index local when the home directory is on a network file system (#80) ([4bd1ead](https://github.com/pederbe/ripscope/commit/4bd1ead))
- Update quay.io/pypa/manylinux_2_28_x86_64:latest Docker digest to 43f7ce1 (#79) ([42c9420](https://github.com/pederbe/ripscope/commit/42c9420))
- Update daily dependencies (#78) ([cb2fccc](https://github.com/pederbe/ripscope/commit/cb2fccc))
- Record published changelog for v0.2.4 (#76) ([6c55338](https://github.com/pederbe/ripscope/commit/6c55338))
- Report validated changelog checks on the checked commit (#77) ([9419cfe](https://github.com/pederbe/ripscope/commit/9419cfe))
- Finalize release notes and record publications on main (#75) ([0c41c6f](https://github.com/pederbe/ripscope/commit/0c41c6f))
- Simplify stored selection rule JSON conversions (#74) ([83ee57d](https://github.com/pederbe/ripscope/commit/83ee57d))
- Account for changelog coverage in changes and release preparation (#73) ([19491dd](https://github.com/pederbe/ripscope/commit/19491dd))
- Use swap removal for unordered scanner probe arguments (#71) ([8c76576](https://github.com/pederbe/ripscope/commit/8c76576))
- Repair reference cleanup and Windows module probe CI failures (#72) ([89913ef](https://github.com/pederbe/ripscope/commit/89913ef))
- Document the current production website build command ([14b7d3b](https://github.com/pederbe/ripscope/commit/14b7d3b))
- Align glossary, website metadata and delivery checks ([fe251b7](https://github.com/pederbe/ripscope/commit/fe251b7))
- Check a line's names for repeats with the fast hasher (#68) ([d6790ff](https://github.com/pederbe/ripscope/commit/d6790ff))
- Order the splitter's listed names without comparing their bytes repeatedly (#66) ([0f09837](https://github.com/pederbe/ripscope/commit/0f09837))
- Update daily dependencies (#65) ([67d7b6c](https://github.com/pederbe/ripscope/commit/67d7b6c))
- Document signed package-manager installation and verify website sources (#64) ([215bdac](https://github.com/pederbe/ripscope/commit/215bdac))
- Use each package client certificate bundle path in live checks (#63) ([7a45b41](https://github.com/pederbe/ripscope/commit/7a45b41))
- Activate the signed apt and dnf repository (#62) ([7a88b71](https://github.com/pederbe/ripscope/commit/7a88b71))
- Update daily dependencies (#58) ([979ebd7](https://github.com/pederbe/ripscope/commit/979ebd7))
- Include package repository checks in required CI (#61) ([db17186](https://github.com/pederbe/ripscope/commit/db17186))
- Prepare a signed apt and dnf repository for Linux packages (#59) ([715741e](https://github.com/pederbe/ripscope/commit/715741e))
- Gate native dependency auto-merge on all applicable checks (#60) ([a02918c](https://github.com/pederbe/ripscope/commit/a02918c))
- Borrow scanner spans from the source instead of copying them (#57) ([efdd813](https://github.com/pederbe/ripscope/commit/efdd813))
- Generate and check WinGet manifests after each release (#56) ([21450db](https://github.com/pederbe/ripscope/commit/21450db))
- Separate website refresh from package and editor delivery (#52) ([10d4963](https://github.com/pederbe/ripscope/commit/10d4963))
- Decode each revision's selection rule once in a Session (#51) ([3cc942d](https://github.com/pederbe/ripscope/commit/3cc942d))
- Write Line protocol rows with less work per byte (#50) ([35f0a7a](https://github.com/pederbe/ripscope/commit/35f0a7a))

[Full comparison](https://github.com/pederbe/ripscope/compare/v0.2.4...v0.2.6)

## [0.2.4] - 2026-10-26

### Other changes

- Publish verified weekly releases to crates.io and pin website npm (#49) ([25342f5](https://github.com/pederbe/ripscope/commit/25342f5))

[Full comparison](https://github.com/pederbe/ripscope/compare/v0.2.3...v0.2.4)

## [0.2.3] - 2026-10-26

### Other changes

- Separate host module inspection from archive runtime checks (#44) ([2395eff](https://github.com/pederbe/ripscope/commit/2395eff))
- Use GNU timeout for native release reference checks (#43) ([4934321](https://github.com/pederbe/ripscope/commit/4934321))
- Bump dompurify (#34) ([7cd88d5](https://github.com/pederbe/ripscope/commit/7cd88d5))
- Preserve release command failure diagnostics (#42) ([a8c8bab](https://github.com/pederbe/ripscope/commit/a8c8bab))
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

[Full comparison](https://github.com/pederbe/ripscope/compare/v0.2.0...v0.2.3)

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
line, Vim, Neovim and Emacs, on Linux and Windows.

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

- Linux with glibc 2.28 or newer, and Windows.
- The index lives on local storage. The
  [limitations](https://ripscope.com/docs/reference/limitations) list the details.

### Installation

- `install.sh` and `install.ps1` verify and install the executable, set up Vim and
  Neovim, and print the line Emacs needs. `--uninstall` (`-Uninstall` in PowerShell)
  removes them again.
- Release archives for Linux and Windows, and Debian and RPM packages, each with a
  SHA-256 checksum.
- Source builds with Cargo.

[Unreleased]: https://github.com/pederbe/ripscope/compare/v0.2.7...HEAD
[0.2.0]: https://github.com/pederbe/ripscope/releases/tag/v0.2.0
[0.1.1]: https://github.com/pederbe/ripscope/releases/tag/v0.1.1
[0.1.0]: https://github.com/pederbe/ripscope/releases/tag/v0.1.0

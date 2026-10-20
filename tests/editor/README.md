# Editor workflow checks

These drivers exercise the shipped Vim, Neovim and Emacs integrations with real
editor and Ripscope processes on Linux and native Windows. They use disposable
projects and isolated editor configuration; they do not install anything into
the user's editor. Each driver is a Rust Cargo example; the scripts they run
inside the editors are in the editors' own languages.

The behavior under test is described in
[Results, warnings and failures](../../docs/editors/diagnostics.md). Each driver
records the executable and adapter identities, commands, output and process
status. Record the backend's build revision with the report: its hash alone does
not identify the source that produced it. Retain reports outside the tracked tree.

Linux checks must run as an unprivileged user so permission tests are meaningful.
Put temporary projects on supported local storage, such as `/var/tmp` on ext4 or
Btrfs, rather than a WSL-to-Windows mount. A passing profile covers that editor
version and platform; it does not establish every editor setup or version.

## Vim

The `check-vim` Cargo example checks Vim's built-in cscope reader. Add `--workflow`
to test the shipped `plugin/ripscope.vim` with explicit settings, both explicit
sources and a named file list. Vim needs cscope, quickfix and jobs; Windows
workflows also need timers and `interrupt()`.

```sh
cargo run --locked --example check-vim -- --backend /absolute/path/ripscope \
  --vim /usr/bin/vim --vim-sha256 SHA256 --workflow \
  --output /absolute/path/vim-linux.json
```

Select the installed editor's identity explicitly when it differs from the
driver's default pinned Ubuntu Vim. Windows also requires its DLL hash:

```powershell
cargo run --locked --example check-vim -- --backend target/release/ripscope.exe `
  --vim C:/path/to/vim.exe --vim-sha256 SHA256 --vim-dll-sha256 DLL_SHA256 `
  --workflow --output C:/path/to/vim-windows.json
```

The persistent script covers all nine searches, ordered rows and actual jumps,
valid empty answers, invalid patterns, warnings with available rows and with no
rows, source repair, index updates visible to an open session, disconnects,
restart and failed startup. Queries after recoverable errors keep the same
backend process. A deleted source is skipped by text searches with a warning;
stored queries still use its indexed facts. A copied index uses the supplied
`-P` directory for live text queries.

`--vim-errors` makes invalid queries and answer warnings visible through E609.
Vim acknowledges the RETURN prompt itself. Under `silent!`, E259 can remain in
`v:errmsg`, so tests also inspect captured messages and ordinary exceptions.
Raw session behavior, including refresh failures and acknowledgement handling,
is covered separately by `tests/pipeline_session_cli.rs`.

The delivered workflows run default automatic and intentional manual profiles with
both explicit sources and a named list. They check saved edits followed immediately by
displayed results and actual jumps, external changes, header-only edits, fixed-list
limits, changed list membership and same-PID continuity. A failed automatic Refresh
keeps navigable rows and a visible E609 warning. Manual profiles retain the existing
build failure, retry and newly indexed content checks. Both preserve unrelated cscope
connections and process cleanup. Windows
also checks native argument handling, path rejection, interrupted builds and
unchanged shell options. The driver enforces a 30-second deadline per process
and checks backend termination. Keep Vimscript LF; `.gitattributes` enforces it.

## Neovim

The `check-neovim` Cargo example runs the owned Lua adapter in headless Neovim 0.10
or newer:

```sh
cargo run --locked --example check-neovim -- --backend /absolute/path/ripscope \
  --nvim /absolute/path/nvim --output /absolute/path/neovim-linux.json
```

```powershell
cargo run --locked --example check-neovim -- --backend target/release/ripscope.exe `
  --nvim 'C:/Program Files/Neovim/bin/nvim.exe' `
  --output C:/path/to/neovim-windows.json
```

The workflows cover all nine searches and quickfix navigation, captured
working directories, discovery and include options, paths with spaces and
Unicode, index build failures, and one session reused across queries, errors,
warnings and index replacement. Warnings preserve available results and remain
visible with zero results. Malformed or incomplete responses, process exit and
deadlines fail the query without publishing partial quickfix results. Recovery,
late callbacks, reconfiguration and editor exit are checked too.

Default automatic checks save an editor buffer and immediately search and jump without
an index command. They also cover external edits, header changes, discovery additions,
removal and rename, explicit-source and file-list limits, and list updates. The same
backend PID survives. A missing required list leaves navigable last-revision rows and
an unconfirmed-freshness warning, then recovers on the next query. Manual profiles
retain updates-skipped diagnostics and explicit build coverage.

The Windows driver isolates HOME/AppData/XDG paths, restricts PATH to System32,
uses a deny-write ACL and reserves an unused SUBST drive for working-directory
checks, then restores both. Linux retains separate shell and signal controls.

## One-step setup

The `check-editor-setup` Cargo example checks the setup a new user makes, in fresh
user profiles: HOME, the XDG directories or USERPROFILE, LOCALAPPDATA and APPDATA
of their own, and no `ripscope` on PATH unless a route puts it there. For each
editor and route it opens a C file, finds a definition, saves an edit and searches
again, in a project with `cscope.files`, a Git repository found recursively and a
plain directory. Neovim also searches from the quickfix window, which shows no file
and must keep the same project. It checks the results and jumps, that the project and
the index are the ones `ripscope --project` reports and the index is published in the
cache directory, that nothing but the saved edit changed in the project, and that the
editor left no process behind.

| Route | Setup |
| --- | --- |
| `path` | The plugin as Vim's and Neovim's own packages, or on Emacs's load path, with `ripscope` on PATH, and for Vim on Linux a shell whose startup prints |
| `plugin` | A repository copy whose install command installs the executable, as a build hook does, over an older one on Linux |
| `installer` | The install script and its printed Emacs line, then `--uninstall`. Emacs must not run a decoy installer two folders above its placed copy |
| `installer-lazy` | The install script where lazy.nvim is set up, and the lazy.nvim line it prints |
| `interrupted` | As `path`, after a first index build that published nothing |
| `home` | As `path`, with the projects in the home directory and the editor started there |
| `spaces` | As `path`, in a profile whose paths hold spaces. Vim, on Windows only, takes them through 8.3 names, with `RIPSCOPE_CACHE_DIR` naming a cache directory without spaces |
| `vim-plug`, `lazy`, `use-package`, `package-vc` | The documented plugin manager lines, following the `release` branch |

The installing routes install a development archive of the given executable from a
loopback mirror. The plugin managers clone a local Git repository of this checkout's
editor files, with a `release` branch and a later commit on `main` they must not
take, so the check needs Git, and `--vim-plug` and `--lazy` checkouts for those
routes.

```sh
cargo run --locked --example check-editor-setup -- --backend /absolute/path/ripscope \
  --vim /usr/bin/vim --nvim /usr/bin/nvim --emacs /usr/bin/emacs \
  --vim-plug /path/to/vim-plug --lazy /path/to/lazy.nvim --work /var/tmp/new-directory
```

Routes and editors left out are skipped and reported as such. `--route` selects
routes. The editor-side steps are `setup.vim`, `setup.lua` and `setup.el`. On
Windows, the same command takes the native executables. Where no user manager can
prove process cleanup, as in an SSH login, the check accepts an empty process group.

## Emacs

The `check-emacs` Cargo example exercises the shipped pinned xcscope and owned adapter with
GNU Emacs 27.1 or newer. Linux also needs Bash, GNU-compatible find and sed;
Windows discovery uses the native backend with no shell helpers.

```sh
cargo run --locked --example check-emacs -- --backend /absolute/path/ripscope \
  --emacs /usr/bin/emacs --output /absolute/path/emacs-linux.json
```

```powershell
cargo run --locked --example check-emacs -- --backend target/release/ripscope.exe `
  --emacs 'C:/Program Files/Emacs/emacs-31.1/bin/emacs.exe' `
  --output C:/path/to/emacs-windows.json
```

The shared ERT cases cover all nine searches and actual jumps, missing and
foreign indexes, valid empty answers, invalid-pattern diagnostics, warnings
with navigable results and with no results, repair, relative project roots,
remote-database rejection and vendor-source checks. As cscope does, a one-shot
invalid pattern exits 0 with a diagnostic; xcscope's completion banner alone
does not establish a valid answer.

Automatic checks save and immediately search, then navigate saved and externally
changed definitions, callers and headers. They cover file creation, removal and rename
under the platform's persisted Selection rule, with explicit list edits where needed.
Every search uses a completed One-shot process. Normal saves and searches preserve
the generated Linux list. Manual mode retains skipped updates and explicit builds.

Linux additionally checks helper prerequisites and failures, permissions,
selection rules, failed process exits and signals after streamed rows,
concurrent index builds, and diagnostic placement when rerunning an older
search. Windows checks native discovery without shell helpers, preserving
manual lists, custom lists, include paths, argument encoding and concurrent
build refusal. The driver enforces a 90-second deadline for the editor process
and records whether its inputs changed during the run. Version discovery and
platform cleanup helpers need an external timeout for the complete invocation.

Ordinary `cargo test` checks the drivers' exit-status handling, captured output and
process-tree timeouts using child-process fixtures; it does not start an editor. The
three drivers share that process handling. They are development tooling and are not
installed with the product. On Linux their process cleanup uses the existing `rustix`
dependency's development-only process feature.

Use `--repo /absolute/checkout` when running a cross-built `check-vim`, `check-neovim`
or `check-emacs` executable against a native checkout at a different path. The default
is its build-time checkout. Frozen project replay uses the separate
[assess-workflows example](../../scripts/assess-workflows.md); its operational
and diagnostic observations remain separate from source accuracy scoring.

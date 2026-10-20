# Ripscope with Emacs

This directory is an Emacs package. It includes unmodified xcscope at
`e27bf918978ac9d5e3383997dcb49d88d6135d91` and the local adaptation, so no other
package or upstream patch is needed. On Emacs 30 or newer, one line in your init file
installs it from the repository:

```elisp
(use-package ripscope :vc (:url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs" :rev :newest) :config (ripscope-setup))
```

On Emacs 29, use `package-vc-install` with the same `:url`, `:branch` and
`:lisp-dir`, then call `(ripscope-setup)`. Or copy this entire directory to a stable location and load it:

```elisp
(when (load "/absolute/path/emacs/ripscope" t) (ripscope-setup))
```

`ripscope-setup` makes xcscope's usual keys search with Ripscope. Where xcscope finds
no database of its own, a search's project is the nearest directory upward holding
`cscope.files`, else the nearest Git repository, else the search's directory, and
the first search there indexes it into your cache directory (`~/.cache/ripscope`, or
`%LOCALAPPDATA%\ripscope` on Windows); `ripscope --project` reports both. It finds
`ripscope` on `exec-path` or where the installer puts it, and installs this copy's
version with `M-x ripscope-install` when it is missing, or when the one the installer
put there is older. Load the package before any other xcscope package.

The earlier explicit form still works, and uses only xcscope's own databases:

```elisp
(load "/absolute/path/emacs/ripscope-xcscope.el")
(ripscope-xcscope-setup "/absolute/path/ripscope")
```

Prerequisites: GNU Emacs 27.1 or newer on Linux or Windows and a runnable Ripscope
binary, which `ripscope-setup` can install. On Windows, name the native `.exe` with
an absolute path, such as `C:/tools/ripscope.exe`; no shell helpers are needed.
xcscope's own index command on Linux also needs Bash,
GNU-compatible find (`-maxdepth`, `-iname`, `-false`) and sed. Workflow checks pass
with Emacs 31.1 on both platforms. Use a supported local disk for the index (see
[Limitations](../../docs/reference/limitations.md#platforms-and-storage)).

To keep an index beside a project's sources instead, run `M-x cscope-index-files`
and choose the project root. It builds `cscope.out`, a
Ripscope index in one file. `cscope-database-file` controls this name; xcscope passes
it explicitly, so it differs from the CLI's `ripscope.db` default.

On Windows, Ripscope [discovers files](../../docs/guide/building.md#choosing-the-files)
directly. It reads `cscope.files` when available, otherwise scans the project root,
recursively when `cscope-index-recursively` is enabled. It never creates or overwrites
a file list. Set `cscope-index-file` to another name to require that explicit list;
a missing custom list fails the build and preserves the previous index. Ripscope's
discovery suffixes apply; xcscope's suffix and ignored-directory settings apply only
on Linux. Use a manual list to choose a different set of Windows sources.
`cscope-create-list-of-files-to-index` is unavailable on Windows; use the index
command for discovery or edit your list directly.

On Linux, the index command creates `cscope.files` using xcscope's suffix,
ignored-directory and recursion rules, then builds the index. The list name is
configurable. `cscope-create-list-of-files-to-index` regenerates only the list.
Both commands overwrite the generated list; keep manual lists under another name
and build them with the CLI.

After the initial index build, save a source edit and use the normal search commands.
Searches go to a Ripscope [session](../../docs/guide/sessions.md) for each index, which
starts with the first search and stays running. It watches the sources and refreshes the
index incrementally before the first search after a change, including changes made
outside Emacs and header-only edits, so searches between changes answer at once. An
index build stops the sessions, and the next search starts a new one. On Windows a
running session keeps the project directory in use: run
`M-x ripscope-xcscope-stop-sessions` before moving or removing a project. Normal saves and
searches never run the list generator or rewrite a file list. On Linux, newly created or renamed files
must be added to the persisted list, either by editing it or explicitly regenerating
it with `cscope-create-list-of-files-to-index`. Removed list entries disappear at the
next query. Windows directory discovery follows additions, deletions and renames
unless an explicit file list is in use.

A failed index build is shown in `*cscope-indexing-buffer*`; an
old readable index does not mean the index build succeeded. Repair the reported problem
and retry. A query failure is shown in `*cscope*` instead of successful completion.
A query that cannot complete its update reports failure. A concurrent writer can leave
queries answering from the last published revision with a diagnostic. Text searches
read the current sources and skip any that cannot be read. A damaged index is rebuilt with `-bu`
(see [Checking and repairing an index](../../docs/guide/recovery.md)).

Use the normal xcscope commands and bindings for all nine searches, then select a
result with Enter. Optional index build settings:

```elisp
(setq ripscope-xcscope-include-directories '("include" "../shared"))
;; Optional manual updates through cscope-index-files.
(setq ripscope-xcscope-auto-refresh nil)
```

The adapter owns common backend options. It supplies `-k` and include/analyzer options
only to index builds; the upstream `cscope-option-*` custom backend flags are not used,
except `cscope-option-do-not-update-database`, which may still add `-d`.
`ripscope-xcscope-auto-refresh` defaults to true and explicitly overrides that `-d`.
Set it to nil for intentional updates-skipped searches until the next index command.
Per-database options in
`cscope-database-regexps` still reach queries: only Ripscope-supported query flags are
valid, and a session that refuses them fails its search visibly. Do not configure index build flags
there. Select one backend per Emacs session; restart to change it.

This is the local legacy-row integration. Remote/TRAMP projects are unsupported;
use local databases and local entries in `cscope-database-regexps`. Filename rows
cannot contain spaces because xcscope splits rows on whitespace. On Windows, the
executable, index and option paths must be representable in the active Windows ANSI
code page, and the adapter refuses characters it cannot pass intact. Search patterns
and results pass as UTF-8.
The native workflow tests cover accented paths and search text, spaces and shell
metacharacters in the project and executable paths, all nine searches, navigation,
selection changes, missing lists, failures, warnings and concurrent build refusal.

The adapter verifies and explicitly loads the shipped `.el` source, never a cached
`.elc`, and stays uncompiled itself, so a package manager that compiles the package
changes neither. Do not replace the pinned vendor file. SHA-256:
`fc2700ccc37773c820b1c9f0b782c15cd64058fa009f606d8d24b515da2fac8e`.
Upstream: https://github.com/dkogan/xcscope.el/tree/e27bf918978ac9d5e3383997dcb49d88d6135d91.
The vendor source retains its copyright and GPL-2.0-or-later notice; the adapter is
also GPL-2.0-or-later. The license is included in `vendor/COPYING`. This bundled
integration has its own license; it does not change Ripscope's Rust license.

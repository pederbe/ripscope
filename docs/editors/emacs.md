# Emacs

Ripscope ships a pinned copy of [xcscope](https://github.com/dkogan/xcscope.el) with a
small adapter that runs Ripscope instead of cscope and reports index build and query
failures that xcscope alone would hide. xcscope's usual keys then search any C
project, with no configuration.

**Requirements:** GNU Emacs 27.1 or newer on Linux or Windows. Workflow checks pass
with Emacs 31.1 on both platforms, the `use-package` and `package-vc-install` lines
below included. Those lines need Emacs 30 and 29 or newer.

## Set up in one step

On Emacs 30 or newer, add this line to your init file:

```elisp
(use-package ripscope :vc (:url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs" :rev :newest) :config (ripscope-setup))
```

On Emacs 29, use this line instead:

```elisp
(unless (package-installed-p 'ripscope) (package-vc-install '(ripscope :url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs"))) (ripscope-setup)
```

The `release` branch holds the latest release. `:rev :newest` takes that branch as it
is: `use-package`'s own release lookup cannot find the package in `contrib/emacs` on
Emacs 31.1. The next time Emacs starts, it installs the package, and
`ripscope-setup` installs the `ripscope` executable of the same version in the
background when it is missing, or when the one the installer put there is older.
`M-x ripscope-install` does that too.

Or run the [installer](../getting-started/installation.md). It installs the
executable and the Emacs integration and prints the one line to add to your init
file, such as:

```elisp
(when (load "~/.local/share/ripscope/emacs/ripscope" t) (ripscope-setup))
```

Load the integration before any other xcscope package. Do not byte-compile or
replace the vendored `xcscope.el`, which the adapter verifies.

## First search

Open a C file in your project and press `C-c s g`, then enter a function name to
find its definition. The first time, Emacs says `Ripscope: indexing ... for the
first time` while it indexes the project. Press Enter on a result to open its
source. Save an edit and search again: the results include it.

Ripscope picks the project and keeps its index for you:

- **The project** is the nearest directory above the file that holds a
  `cscope.files`, else the nearest Git repository, else the file's own directory.
- **The sources** are the files `cscope.files` names, when there is one. Otherwise
  Ripscope finds the C sources in the project and its subdirectories.
- **The index** lives in your cache directory, `~/.cache/ripscope` on Linux and
  `%LOCALAPPDATA%\ripscope` on Windows, named after the project. On Windows a search
  briefly makes a marker file in the project directory.

A database xcscope finds by itself still wins: a `cscope.out` index built with
`M-x cscope-index-files`, or an entry in `cscope-database-regexps`. A Git
repository in your home directory, such as one for dotfiles, is not taken as a
project. Where the project is your home directory or a drive, only the files
directly in it are indexed. `ripscope --project FILE` shows what Ripscope picks for
a file.

## Searches

Search with the usual xcscope bindings and press Enter on a result to jump to it:

| Keys | Finds |
| --- | --- |
| `C-c s s` | Symbol occurrences |
| `C-c s g` or `C-c s d` | Definitions |
| `C-c s c` | Callers |
| `C-c s C` | Functions called by a function |
| `C-c s t` | Text |
| `C-c s e` | Regular expression |
| `C-c s f` | Files |
| `C-c s i` | Files including a file |
| `C-c s =` | Assignments |

These optional bindings search for the symbol at point without a prompt:

```elisp
(with-eval-after-load 'xcscope
  (define-key cscope-minor-mode-keymap (kbd "M-.") #'cscope-find-global-definition-no-prompting)
  (define-key cscope-minor-mode-keymap (kbd "M-,") #'cscope-pop-mark))
```

Searches go to one Ripscope session per index, which stays running and refreshes
changed sources incrementally before the first search after a change, including
external and header-only edits. Searches between changes answer at once. Normal saves
and searches never rewrite `cscope.files`.

Emacs finds `ripscope` on `exec-path`, else where the installer puts it
(`~/.local/bin`, or `%USERPROFILE%\.local\bin` on Windows).

## Settings

```elisp
(setq ripscope-xcscope-include-directories '("include"))
;; Optional manual updates until cscope-index-files is run again.
(setq ripscope-xcscope-auto-refresh nil)
```

Include directories apply to every index Ripscope builds. Automatic Refresh is
enabled by default, and its explicit backend control overrides the `-d` xcscope
supplies.
`(ripscope-setup "/path/to/ripscope")` names the executable instead of finding it.

## Building an index in the project

xcscope's own index command still works, for a project whose index you want beside
its sources:

- `M-x cscope-index-files` asks for the project root and builds `cscope.out`, a
  Ripscope index in one SQLite file. Set `cscope-database-file` to use another name.
  On Windows, Ripscope discovers files directly, honoring an existing `cscope.files`
  without overwriting it. `cscope-index-recursively` controls recursion, and a custom
  `cscope-index-file` is passed as a required explicit list. On Linux, xcscope
  generates and overwrites `cscope.files` using its suffix and exclusion rules, and
  needs Bash, GNU find and sed. Keep manual lists under another name and build
  those with the CLI.
- Linux's generated list stays the Selection rule. Add new or renamed files to it,
  or explicitly regenerate it with `cscope-create-list-of-files-to-index`. Windows
  discovery follows additions, removals and renames unless an explicit list is used.
  Failed builds are shown in `*cscope-indexing-buffer*`.

The adapter can also be set up on its own with
`(ripscope-xcscope-setup "/path/to/ripscope")`, which uses only xcscope's own databases.

## Good to know

- Query diagnostics stay in `*cscope*`, and a search that could not be answered, such
  as one with an invalid pattern, shows `Ripscope search failed`. See
  [results, warnings and failures](diagnostics.md).
- Remote (TRAMP) projects are not supported, and result paths cannot contain spaces.
- On Windows, the executable, index and option paths must fit the active Windows ANSI
  code page, and unsupported characters are refused. Search patterns may hold any
  character. The list-only generation command is unavailable there.
- On Windows a running session keeps its project directory in use: run
  `M-x ripscope-xcscope-stop-sessions` before moving or removing a project.

The vendored xcscope and the adapter are licensed GPL-2.0-or-later, separately from
Ripscope. See the
[integration README](https://github.com/pederbe/ripscope/blob/main/contrib/emacs/README.md).

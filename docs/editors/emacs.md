# Emacs

Ripscope's Emacs package searches C through Emacs's built-in xref. `M-.` finds a
definition and `M-?` the uses of a symbol in any C project, with no configuration, and
results open in the usual xref buffer. All nine searches are commands, with optional
xcscope-style keys. The package needs no other package.

**Requirements:** GNU Emacs 28.1 or newer on Linux or Windows. Workflow checks pass
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
executable and the Emacs package and prints the one line to add to your init file,
such as:

```elisp
(when (load "~/.local/share/ripscope/emacs/ripscope" t) (ripscope-setup))
```

## First search

Open a C file in your project, put the cursor on a function name and press `M-.`. The
first time, Emacs says `Ripscope: indexing ... for the first time` while it indexes
the project, then jumps to the definition. `M-,` takes you back. Save an edit and
search again: the results include it.

Ripscope picks the project and keeps its index for you:

- **The project** is the nearest directory above the file that holds a
  `cscope.files`, else the nearest Git repository, else the file's own directory.
- **The sources** are the files `cscope.files` names, when there is one. Otherwise
  Ripscope finds the C sources in the project and its subdirectories.
- **The index** lives in your cache directory, `~/.cache/ripscope` on Linux and
  `%LOCALAPPDATA%\ripscope` on Windows, named after the project. On Windows a search
  briefly makes a marker file in the project directory.

A Git repository in your home directory, such as one for dotfiles, is not taken as a
project. Where the project is your home directory or a drive, only the files directly
in it are indexed. `ripscope --project FILE` shows what Ripscope picks for a file.

## Searches

`ripscope-setup` turns on `ripscope-mode` in C buffers, which makes Ripscope their
xref backend:

| Keys | Finds |
| --- | --- |
| `M-.` | Definitions of the symbol at point |
| `M-?` | Uses of the symbol |
| `M-,` | Goes back |

Every search is also a command, which asks for a pattern with the symbol at point as
its default. `ripscope-command-map` holds them under the keys xcscope gives them. To
use xcscope's `C-c s` prefix, add:

```elisp
(define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)
```

| Command | Keys | Finds |
| --- | --- | --- |
| `ripscope-find-symbol` | `C-c s s` | Symbol occurrences |
| `ripscope-find-definition` | `C-c s g` or `C-c s d` | Definitions |
| `ripscope-find-callers` | `C-c s c` | Callers |
| `ripscope-find-callees` | `C-c s C` | Functions called by a function |
| `ripscope-find-text` | `C-c s t` | Text |
| `ripscope-find-egrep` | `C-c s e` | Regular expression |
| `ripscope-find-file` | `C-c s f` | Files |
| `ripscope-find-includers` | `C-c s i` | Files including a file |
| `ripscope-find-assignments` | `C-c s =` | Assignments |
| `ripscope-index-project` | `C-c s I` | Indexes the project again |

Searches go to one Ripscope session per index, which stays running and refreshes
changed sources incrementally before the first search after a change, including
external and header-only edits. Searches between changes answer at once. Normal saves
and searches never rewrite `cscope.files`.

Emacs finds `ripscope` on `exec-path`, else where the installer puts it
(`~/.local/bin`, or `%USERPROFILE%\.local\bin` on Windows).

## Settings

```elisp
(setq ripscope-include-directories '("include"))
;; Index edits only when you run M-x ripscope-index-project.
(setq ripscope-auto-refresh nil)
;; Name the executable instead of finding it.
(setq ripscope-executable "/path/to/ripscope")
```

Include directories are relative to the project, or absolute, and apply to every
index Ripscope builds. `ripscope-timeout` sets how many seconds a search waits for
its answer, 120 by default.

## Good to know

- A search that finds nothing, or cannot be answered, such as one with an invalid
  pattern, says why in the echo area. A warning that comes with results, such as a
  source a text search could not read, appears there too and stays in `*Messages*`.
  See [results, warnings and failures](diagnostics.md).
- Paths with spaces and any other characters work.
- Remote (TRAMP) files are left to other xref backends.
- On Windows, the executable, index and include paths must fit the active Windows
  code page, and unsupported characters are refused. Search patterns may hold any
  character.
- On Windows a running session keeps its project directory in use: run
  `M-x ripscope-stop-sessions` before moving or removing a project.

The Emacs package is under the MIT or the Apache-2.0 licence, as Ripscope is. See the
[package README](https://github.com/pederbe/ripscope/blob/main/contrib/emacs/README.md).

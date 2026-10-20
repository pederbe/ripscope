# Ripscope for Emacs

This directory is the Emacs package for [Ripscope](https://ripscope.com), which finds
definitions, callers, called functions, assignments, text and files in C projects. It
works through Emacs's built-in xref, so `M-.` finds a definition and `M-?` the uses of
a symbol in any C buffer, with no configuration. It needs no other package.

On Emacs 30 or newer, one line in your init file installs it from the repository:

```elisp
(use-package ripscope :vc (:url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs" :rev :newest) :config (ripscope-setup))
```

On Emacs 29, use `package-vc-install` with the same `:url`, `:branch` and
`:lisp-dir`, then call `(ripscope-setup)`. Or copy this directory to a stable place
and load it:

```elisp
(when (load "/absolute/path/emacs/ripscope" t) (ripscope-setup))
```

`ripscope-setup` turns on `ripscope-mode` in C buffers. Installed from the repository,
it also installs the `ripscope` executable of the same version when it is missing, or
when the one the installer put there is older. `M-x ripscope-install` does that too.
Otherwise run the one-line installer from [ripscope.com](https://ripscope.com).

## Searching

The first search in a project indexes it into your cache directory
(`~/.cache/ripscope`, or `%LOCALAPPDATA%\ripscope` on Windows). The project is the
nearest directory upward holding `cscope.files`, else the nearest Git repository, else
the file's directory. `ripscope --project FILE` shows what Ripscope picks.

Each index has one Ripscope session, which starts with the first search and stays
running. It watches the sources and brings the index up to date before the next
search after a change, including changes made outside Emacs, so your saved edits
appear in the results.

All nine searches are commands, each asking for a pattern with the symbol at point
as its default:

| Command | Finds | xcscope key |
| --- | --- | --- |
| `ripscope-find-symbol` | Uses of a symbol | `s` |
| `ripscope-find-definition` | Definitions | `g` or `d` |
| `ripscope-find-callees` | Functions a function calls | `C` |
| `ripscope-find-callers` | Callers of a function | `c` |
| `ripscope-find-text` | Lines holding some text | `t` |
| `ripscope-find-egrep` | Lines matching an extended regular expression | `e` |
| `ripscope-find-file` | Files whose path holds a name | `f` |
| `ripscope-find-includers` | Files including a file | `i` |
| `ripscope-find-assignments` | Assignments to a symbol | `=` |

`ripscope-command-map` holds them under the keys xcscope uses, with `I` for
`ripscope-index-project`. To use xcscope's `C-c s` prefix:

```elisp
(define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)
```

Results open in the xref buffer. A search that finds nothing, or cannot be answered,
says why in the echo area. A warning that came with results, such as a source a text
search could not read, appears there too and stays in `*Messages*`.

## Settings

```elisp
;; Headers to index from outside the project, relative to it or absolute.
(setq ripscope-include-directories '("include" "../shared"))
;; Index edits only with M-x ripscope-index-project.
(setq ripscope-auto-refresh nil)
;; Name the executable instead of finding it.
(setq ripscope-executable "/path/to/ripscope")
```

Emacs finds `ripscope` on `exec-path`, else where the installer puts it
(`~/.local/bin`, or `%USERPROFILE%\.local\bin` on Windows). On Windows the
executable, index and include paths must fit the active Windows code page, and
search patterns may hold any character.

`M-x ripscope-stop-sessions` stops the running sessions. On Windows a session keeps
its project directory in use, so stop them before you move or remove a project.

Remote (TRAMP) files are left to other xref backends.

## Requirements and licence

GNU Emacs 28.1 or newer, and the ripscope executable on Linux or Windows. Workflow
checks pass with Emacs 31.1 on both.

This package is under the MIT or the Apache-2.0 licence at your option, as Ripscope is.

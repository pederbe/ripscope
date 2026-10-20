# Installation

Install Ripscope for **x86-64 Linux or Windows** in one step, then open a C file and
search. Pick the way you prefer.

## The installer

The installer verifies and installs Ripscope in `~/.local/bin`
(`%USERPROFILE%\.local\bin` on Windows) and sets up Vim and Neovim, which load
Ripscope by themselves from then on. Neovim set up with lazy.nvim needs the one line
the installer prints, as below.

### Linux

In your terminal, run:

```sh
curl -fsSL https://ripscope.com/install.sh | sh
```

### Windows

In PowerShell 5.1 or newer, run:

```powershell
irm https://ripscope.com/install.ps1 | iex
```

### Emacs and lazy.nvim

The installer also prints one line to add to your Emacs init file, such as:

```elisp
(when (load "~/.local/share/ripscope/emacs/ripscope" t) (ripscope-setup))
```

Neovim set up with lazy.nvim (as LazyVim, kickstart, AstroNvim and NvChad are) loads
only the plugins in its list. Where the installer finds lazy.nvim, it prints the
line to add there too.

## Your plugin manager

One line installs the editor plugin, and its build step installs the `ripscope`
executable.

| Editor | Add this |
| --- | --- |
| Neovim, with lazy.nvim | `{ 'pederbe/ripscope', branch = 'release', build = ':RipscopeInstall' },` |
| Vim, with vim-plug | `Plug 'pederbe/ripscope', { 'branch': 'release', 'do': ':RipscopeInstall' }` |
| Emacs 30 or newer | `(use-package ripscope :vc (:url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs" :rev :newest) :config (ripscope-setup))` |

The `release` branch holds the latest release, and the build step installs the
executable of that same version.

The [Vim](../editors/vim.md), [Neovim](../editors/neovim.md) and
[Emacs](../editors/emacs.md) guides cover other plugin managers and Emacs 29.

## Start searching

Open a C file in your project and search: `:cs find g main` in Vim,
`:RipscopeFind g main` in Neovim, or `C-c s g` in Emacs. The first search indexes
the project into your cache directory, and later searches see your saved edits.

To search from the command line too, check that your terminal finds Ripscope:

```sh
ripscope --version
```

If the command is not found, [add Ripscope to PATH](../guide/installation-options.md#add-ripscope-to-path).
The editors find it either way. Then continue to the [quick start](quick-start.md).

For system packages, manual downloads or source builds, see
[other installation methods](../guide/installation-options.md).

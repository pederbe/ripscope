# Installation

One command installs Ripscope on **Linux or Windows** and sets up your editor.

## Linux

In your terminal, run:

```sh
curl -fsSL https://ripscope.com/install.sh | sh
```

## Windows

In PowerShell 5.1 or newer, run:

```powershell
irm https://ripscope.com/install.ps1 | iex
```

## What the installer does

The installer verifies its download and puts Ripscope in `~/.local/bin`
(`%USERPROFILE%\.local\bin` on Windows). It also sets up Vim and Neovim, which load
Ripscope by themselves from then on.

Two setups need one more line, which the installer prints for you.

- **Emacs.** Add the printed line to your init file. It looks like this:

  ```elisp
  (when (load "~/.local/share/ripscope/emacs/ripscope" t) (ripscope-setup))
  ```

- **Neovim with lazy.nvim.** lazy.nvim, which LazyVim, kickstart, AstroNvim and
  NvChad use, loads only the plugins in its list. Where the installer finds
  lazy.nvim, it prints the line to add to that list.

## Check that it works

In your terminal, run:

```sh
ripscope --version
```

It prints the name and the version. If the command is not found,
[add Ripscope to PATH](../guide/installation-options.md#add-ripscope-to-path).
The editors find Ripscope either way.

Then open a C file in your project and search: `:cs find g main` in Vim,
`:RipscopeFind g main` in Neovim, or `M-.` on `main` in Emacs. The first search indexes
the project into your cache directory, and later searches see your saved edits.

Continue to the [quick start](quick-start.md).

## Your Linux package manager

On Debian, Ubuntu, Fedora and RHEL-family systems, the
[signed apt or dnf repository](../guide/installation-options.md#apt-and-dnf-repositories)
installs Ripscope and updates it with the rest of your system. Packages include
the executable and editor sources. To enable editor integration, follow the
[plugin-manager setup](#your-plugin-manager) below, which reuses the executable
installed by your package manager.

## Your plugin manager

One line installs the editor plugin. If Ripscope is not already installed, its
build step also downloads the latest executable.

| Editor | Add this |
| --- | --- |
| Neovim, with lazy.nvim | `{ 'pederbe/ripscope', branch = 'release', build = ':RipscopeInstall' },` |
| Vim, with vim-plug | `Plug 'pederbe/ripscope', { 'branch': 'release', 'do': ':RipscopeInstall' }` |
| Emacs 30 or newer | `(use-package ripscope :vc (:url "https://github.com/pederbe/ripscope" :branch "release" :lisp-dir "contrib/emacs" :rev :newest) :config (ripscope-setup))` |

The `release` branch holds the latest release. An existing package-managed
executable is kept, so continue using your package manager to update it.

The [Vim](../editors/vim.md), [Neovim](../editors/neovim.md) and
[Emacs](../editors/emacs.md) guides cover other plugin managers and Emacs 29.

## More ways to install

[Other installation methods](../guide/installation-options.md) covers standalone
packages, archives, Cargo, source builds and the installer's options.

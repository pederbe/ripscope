# Ripscope ships its own editor integrations, set up in one step

[ADR 0015](0015-native-emacs-package.md), "Emacs searches through xref, with no
package dependencies", supersedes the vendored xcscope and its adapter described here.

Ripscope is used from Vim, Neovim and Emacs. Their cscope integrations expect an
executable named `cscope`, ignore its exit status or swallow its failures, and each
finds its database in its own way. Someone trying Ripscope decides in the first few
minutes whether to keep it.

## Decision

Ripscope ships its own integrations, pinned to its releases: a Vim plugin over Vim's
built-in cscope support, a Neovim Lua plugin over the
[JSON interface](0013-json-interface.md), and for Emacs a vendored, pinned xcscope
with a small adapter that reports the backend failures xcscope would ignore.

Setting an editor up takes one step. A plugin-manager line installs the integration
from the repository's `release` branch. The integration then installs the executable
of the same version through the checksum-verifying installer, from a build hook in Vim
and Neovim and from `ripscope-setup` in Emacs. Run from the checkout with `--plugin`,
the installer alone decides whether to: it installs the checkout's version unless the
executable the editors find is that version or newer, or one it did not put there.
Emacs asks it only when the executable is missing or the installer has not yet checked
it for the checkout's `Cargo.toml`, so that starting Emacs runs no installer. Alternatively the installer
installs the executable, places the Vim and Neovim plugins where those editors load
packages, and prints the line Emacs needs. Each release moves the `release` branch to
its tag.

Without configuration, the executable decides where a search belongs, and
`ripscope --project PATH` reports it. All three editors share this one implementation,
and so one index per project.

- **Project.** The nearest directory upward from the file that holds `cscope.files`,
  else the nearest Git repository, else the editor's current directory when the file
  is inside it, else the nearest existing directory holding the file. The home directory and a file system
  root are never taken as a Git repository or a current directory, and a project
  there is read without its subdirectories.
- **Index.** In the user's cache directory, named after the project directory and the
  first 16 hexadecimal digits of the SHA-256 of its path with links resolved, so that a
  project reached through a link shares its index. On Windows the path has forward
  slashes and is lower-cased first. The project gets no Ripscope file, though
  on Windows a session briefly makes its marker file there
  ([ADR 0008](0008-sessions-refresh-on-watched-changes.md)).
- **First search.** The editor builds the index when nothing is published in it yet,
  then searches through a session, which keeps the index current. xcscope runs a
  process per search and a one-shot query refreshes first, so the Emacs adapter keeps
  one session per index.

Every explicit setting overrides these defaults. `ripscope --project PATH` followed by
an index build's `-f` and selection options judges publication by that index and those
sources, for an editor whose settings choose them.

## Consequences

- Changing how a project or its index name is worked out makes every editor build a
  new index and leaves the old ones in the cache.
- The Emacs adapter and vendored xcscope are GPL-2.0-or-later, separate from Ripscope's
  MIT or Apache-2.0 licence.
- The Emacs adapter stays uncompiled, so a package manager cannot turn its dynamic
  bindings of xcscope's variables into lexical ones.
- On Windows, Vim cannot pass an index path with a space to its cscope support, so
  `RIPSCOPE_CACHE_DIR` can move the cache to a folder without one.

## Considered options

- **Rename the executable to `cscope`, or add a shim on PATH.** It hides which tool is
  running and cannot repair integrations that ignore exit status.
- **Adapt an existing Neovim plugin such as cscope_maps.nvim.** It accepts only
  executables named `cscope` or `gtags-cscope`, and swallows failures.
- **An index in the project, as cscope's `cscope.out`.** It writes into a tree the user
  may not own or want changed, and shows up in version control. Explicit settings still
  allow it.
- **Each adapter finds the project itself.** Every adapter would have to agree byte for
  byte to share an index.
- **Plugin managers follow the default branch.** Users would run unreleased integration
  code against a released executable.

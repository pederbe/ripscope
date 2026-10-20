# Emacs searches through xref, with no package dependencies

This supersedes the Emacs part of
[ADR 0014](0014-own-editor-integrations.md), "Ripscope ships its own editor
integrations, set up in one step". The rest of that decision stands.

Ripscope's Emacs integration was a vendored, pinned xcscope with an adapter that
overrode its internals in about 200 places, because xcscope starts a process for every
search and ignores failures. That tied Ripscope to another package's private code,
shipped GPL code beside Ripscope, stopped anyone who already had xcscope installed,
and kept the package out of MELPA, which expects a package to depend on other
packages rather than bundle them.

## Decision

The Emacs package is Ripscope's own, in one file, `contrib/emacs/ripscope.el`, as the
Neovim plugin is:

- It keeps one long-running session per index over the
  [JSON interface](0013-json-interface.md), so paths with any characters work and
  failures are reported.
- It is an xref backend. `ripscope-mode`, which `ripscope-setup` turns on in C buffers,
  makes `M-.` find definitions and `M-?` uses, and results open in the standard xref
  buffer.
- All nine searches are commands. `ripscope-command-map` holds them on xcscope's keys,
  for the user to bind to a prefix such as `C-c s`, since a package does not take a
  `C-c` letter key by itself.
- A search waits for its whole answer before xref shows it, up to `ripscope-timeout`,
  and an interrupted or late answer ends the session. The next search starts another.
- The first search in a project builds its index, and `ripscope-setup` installs the
  executable as ADR 0014 describes.
- It needs GNU Emacs 28.1 or newer, for the xref and minibuffer functions it
  uses, and no other package.
- It is licensed MIT or Apache-2.0, as the rest of Ripscope is.

## Consequences

- xcscope's own commands, its `cscope.out` index in the project and its database
  settings no longer reach Ripscope. Its users keep xcscope for cscope and bind
  Ripscope's commands as they like.
- An index in the project is no longer built from Emacs. `ripscope -b` builds one, and
  the automatic project finds the one in the cache.
- The package can be byte-compiled, checked with `package-lint` and `checkdoc`, and
  listed on MELPA.
- Release archives, installers and system packages carry no GPL source or notice.

## Considered options

- **Keep the xcscope adapter as a second option.** Two integrations would need their
  own tests and documentation, and the old one would keep the GPL code and the clash
  with an installed xcscope.
- **Depend on xcscope from MELPA instead of vendoring it.** The adapter would still
  override xcscope's internals, and break whenever they changed.
- **Show results asynchronously, as Neovim does.** xref asks its backend for results
  and waits for them, and a search answers in milliseconds once its index is current.

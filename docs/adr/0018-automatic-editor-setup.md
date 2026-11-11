# Editors discover projects and refresh automatically

Vim, Neovim and Emacs use the current file's project and refresh saved changes
automatically. Vim and Neovim no longer offer project-root, index-location or
source-selection settings. A project's `cscope.files` provides a shared source
selection for every editor. This supersedes those settings in
[ADR 0014](0014-own-editor-integrations.md).

Editor manual-refresh settings and the redundant `--no-auto-refresh` CLI option
are removed. The cscope-compatible `-d` remains for scripts that need manual
control. `--auto-refresh` remains because Vim itself always supplies `-d`.
Explicit editor build commands remain useful for retries and immediate updates.

Installers use the project's GitHub release address. There is no public mirror
override or forced-reinstall option. Release and editor tests change only the
release address in temporary installer copies to serve unpublished candidate
archives and broken downloads from a loopback server. Shipped installers retain
their fixed address. This refines the test route in
[ADR 0017](0017-install-latest-release.md).

Executable paths, include directories, cache location, editor timeouts and window
controls retain concrete uses. Homebrew, Nix and conda recipes remain for package
channel adoption. The Arch binary-package template remains useful for local
pacman installation without an AUR listing.

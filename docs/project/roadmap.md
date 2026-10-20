# Roadmap

Ripscope provides cscope-style navigation over written C source. The
[quality method](../architecture/quality.md) explains the separate compatibility,
Navigation accuracy and workflow checks.

## Implemented

- **Every readable file indexed.** A new analyzer that reads C the way cscope does, so no
  file is left out, with Corrections where cscope reads C wrongly.
- **Separate compatibility and accuracy checks.** Reference captures pin observed
  readings; independent source labels check selected navigation facts separately.
- **An incremental index.** One SQLite file, updated in place, with metadata-based
  change detection and content checks for recently modified files.
- **Drop-in command line.** The invocations Vim, Neovim, Emacs and cscope scripts
  already use, including `cscope.files` options.
- **Editor integrations.** Vim, and Neovim and Emacs packages over persistent JSON
  sessions, with workflow checks on Linux and Windows.
- **Scoped evidence.** Synthetic reference captures, finite source-labelled inputs
  and index-lifecycle tests remain available with their stated limits.

## Continuing: ordinary unfamiliar-project use

- Keep independent labels and Linux and Windows project workflows current as
  product behavior changes.
- Improve common recognition defects with bounded reproductions and retained
  controls.
- Keep conditional-function context limits visible before considering a change to
  query semantics or stored scope.

Known recognition limits remain explicit; completed index builds and reference
agreement do not establish navigation correctness throughout a project.

## Later, if there is demand

- C++ support.
- macOS and ARM64 builds.
- An interactive terminal interface.

Have a use case that is not covered? Open a
[feature request](https://github.com/pederbe/ripscope/issues/new?template=feature_request.yml).

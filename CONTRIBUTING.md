# Contributing to Ripscope

Thanks for your interest! Bug reports, compatibility reports, documentation fixes and
code are all welcome. Ripscope is young, so it is worth opening an issue to discuss a
larger change before writing it.

## Ways to help

- **Try it on your code** and [report](https://github.com/pederbe/ripscope/issues/new/choose)
  what fails. A wrong or missing result with a small reproduction is one of the most
  useful reports you can file.
- **Report compatibility differences** with cscope using the
  [compatibility template](https://github.com/pederbe/ripscope/issues/new?template=compatibility.yml).
- **Improve the documentation** in [`docs/`](https://github.com/pederbe/ripscope/tree/main/docs).
- **Fix an issue.** Issues labelled `good first issue`, when available, are a good
  start.

## Development setup

You need stable Rust and a C compiler (SQLite is compiled from its bundled sources).
On Windows, install the MSVC Build Tools with the Windows SDK.

```sh
git clone https://github.com/pederbe/ripscope.git
cd ripscope
cargo build --locked
cargo test --locked
```

Before sending a change, run the same checks as CI:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Ripscope forbids `unsafe` code and pins every dependency to an exact version. Adding
a dependency needs a good reason in the pull request.

To preview the documentation site, install
[mdBook 0.5.4](https://rust-lang.github.io/mdBook/), then:

```sh
scripts/build-site.sh --dev
```

## Where things are

| Path | Contents |
| --- | --- |
| `src/invocation.rs` | The command line, read as cscope reads its options |
| `src/cli.rs`, `src/cli/` | Index builds, one-shot queries, sessions, `--verify` and the hidden commands the compass uses |
| `src/analyzer.rs`, `src/scanner/`, `src/unit.rs` | The Analyzer: Ripscope's scanner, matched to cscope, the Corrections it reads with, and the Analysis unit it produces |
| `src/store.rs`, `src/store/` | The index: one SQLite file |
| `src/revision.rs` | The index revision a query reads: files in database order and their Analysis units |
| `src/refresh.rs`, `src/refresh/` | The selection rule, change detection and updates |
| `src/engine.rs`, `src/engine/` | The query engine for the stored fields |
| `src/query.rs`, `src/query/` | Queries: a field's pattern compiled with the product's limits, and Result rows |
| `src/sink.rs` | Result rows in the line and JSON formats |
| `tests/*_cli.rs`, `tests/fixtures/` | Integration tests and reference fixtures |
| `tests/editor/` | Real-editor workflow tests |
| `plugin/`, `lua/` | The Vim and Neovim plugin, where plugin managers find it |
| `contrib/` | The Emacs integration |
| `examples/`, `tools/`, `scripts/` | Reference capture and comparison tools, release builders and their guides, listed in the [tool inventory](https://github.com/pederbe/ripscope/blob/main/scripts/README.md) |
| `docs/` | This documentation site, including [architecture](https://ripscope.com/docs/architecture/overview) and [decisions](https://ripscope.com/docs/adr/) |
| `CONTEXT.md` | The project's [glossary](https://ripscope.com/docs/reference/glossary). Please use its terms |

Code, including tests and tools, is Rust. The language check
(`tests/language_check.rs`) names the only exceptions and the languages each allows.
They are the installers, the editor plugins and the scripts the editor tests run inside
each editor, the C, lex and yacc test fixtures and the demo's C project, the website's
scripts with its landing-page check and demo renderer, short build, packaging and
container smoke glue, and the package-manager recipes.

## Changing behavior

Ripscope's contract is cscope's *observable* behavior: result rows, order,
duplicates, context names, displayed text, diagnostics and exit status. Before
changing any of these:

1. **Check the reference.** Expected output comes from the pinned cscope 15.9
   executable, not from Ripscope's current output or cscope's manual. Include the
   reference output and the command you used in your pull request.
2. **Check the known differences.** A mismatch listed in
   [Differences from cscope](https://ripscope.com/docs/reference/differences-from-cscope)
   is intended.
3. **Proposing a new difference** needs a minimal reproduction, an explanation of
   why cscope's behavior is wrong, and the intended output. It is discussed in an
   issue first.
4. **A reading that differs from cscope's is a Correction.** It stays local to the
   construct, can be switched off, is measured against cscope on real projects, and a
   probe fixture shows both readings ([ADR 0002](https://ripscope.com/docs/adr/0002-measured-compatibility),
   [how Ripscope reads C](https://ripscope.com/docs/architecture/reading-c)).
   Every file is indexed, and none is refused
   ([ADR 0004](https://ripscope.com/docs/adr/0004-own-scanner)).

Add a regression test for every behavior change: a unit test near the code for
internal logic, and a `tests/*_cli.rs` test for anything a user can observe.

Hard-to-reverse design changes (storage, analyzers, interfaces, scope) get a short
[architecture decision record](https://ripscope.com/docs/adr/).

## Pull requests

- Keep each pull request focused on one change, and update the documentation and
  `CHANGELOG.md` when user-visible behavior changes.
- Write commit subjects in the imperative ("Preserve UTF-8 literal bytes"), and
  explain *why* in the body when it is not obvious.
- CI must pass on Linux and Windows.

Contributions are dual-licensed under MIT or Apache-2.0, like the rest of Ripscope,
unless explicitly stated otherwise. The Emacs integration under `contrib/emacs` is
GPL-2.0-or-later.

## Release builds

Ordinary development uses Cargo. The release tool in `tools/release` prepares candidate
archives and packages with the current Analyzer, warning, stored/live query and
Refresh checks. See the [Linux release guide](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md)
and [Windows release guide](https://github.com/pederbe/ripscope/blob/main/docs/windows-release.md) for their
pinned profiles and validation limits, and the
[tool inventory](https://github.com/pederbe/ripscope/blob/main/scripts/README.md)
for the maintained checks. Publishing a release is a separate manual step.

## Code of conduct

This project follows the [code of conduct](https://github.com/pederbe/ripscope/blob/main/CODE_OF_CONDUCT.md).
Report security issues privately as described in
[SECURITY.md](https://github.com/pederbe/ripscope/blob/main/SECURITY.md).

# Contributing to Ripscope

Bug reports, compatibility reports, documentation fixes and
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

A change to the release tool alone runs that tool's checks instead, on Linux and
Windows. A change that touches both runs both sets of checks, and scheduled and
manual CI runs keep their full coverage.

Ripscope forbids `unsafe` code and pins every dependency to an exact version. Adding
a dependency needs a good reason in the pull request.

To preview the documentation site and check its links, install
[mdBook 0.5.4](https://rust-lang.github.io/mdBook/), then:

```sh
scripts/build-site.sh --dev
```

User documentation follows one layout. The README stands on its own, and the book in
`docs/` has the parts Getting started, Editors, Guides, Reference, Explanation, Help
and Project. Run every example you add or change and paste its real output.

## Where things are

[ARCHITECTURE.md](https://github.com/pederbe/ripscope/blob/main/ARCHITECTURE.md) maps
the main parts of the code and the folders around it. The
[glossary](https://ripscope.com/docs/reference/glossary) defines the project's terms.
Please use them. Design decisions are recorded in
[docs/adr](https://github.com/pederbe/ripscope/tree/main/docs/adr).

Code, including tests and tools, is Rust. The language check
(`tests/language_check.rs`) names the only exceptions and the languages each allows.

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
   probe fixture shows both readings ([ADR 0002](https://github.com/pederbe/ripscope/blob/main/docs/adr/0002-measured-compatibility.md),
   [how Ripscope reads C](https://ripscope.com/docs/architecture/reading-c)).
   Every file is indexed, and none is refused
   ([ADR 0004](https://github.com/pederbe/ripscope/blob/main/docs/adr/0004-own-scanner.md)).

Add a regression test for every behavior change: a unit test near the code for
internal logic, and a `tests/*_cli.rs` test for anything a user can observe.

Hard-to-reverse design changes (storage, analyzers, interfaces, scope) get a short
[architecture decision record](https://github.com/pederbe/ripscope/tree/main/docs/adr).

## Pull requests

All changes enter `main` through a pull request, including the maintainer's. The
maintainer creates a `contrib/` branch in this repository, and other contributors
create a branch in their fork. Open the pull request against `main` once the checks
above pass, and link the issue when there is one. A pull request merges after the
required CI check passes on Linux and Windows, with the branch current with `main`.

- Keep each pull request focused on one change, and update the documentation when
  user-visible behavior changes.
- Write commit subjects in the imperative ("Preserve UTF-8 literal bytes"), and
  explain *why* in the body when it is not obvious.

Include a concise entry under `[Unreleased]` in
[CHANGELOG.md](https://github.com/pederbe/ripscope/blob/main/CHANGELOG.md) in
the same pull request as a user-visible change. Cover capabilities, fixes,
installation, editor setup and changes to command-line behavior, result formats
or the index. Explain compatibility consequences and any action users need to
take. Keep performance claims tied to their measured workloads. If no entry is
needed, explain why in the pull request's Changelog section.

Contributions are dual-licensed under MIT or Apache-2.0, like the rest of Ripscope,
unless explicitly stated otherwise.

## Code of conduct

This project follows the [code of conduct](https://github.com/pederbe/ripscope/blob/main/CODE_OF_CONDUCT.md).
Report security issues privately as described in
[SECURITY.md](https://github.com/pederbe/ripscope/blob/main/SECURITY.md).

## Maintainer guides

Release builds, the weekly release and dependency updates are described in the
[tool inventory](https://github.com/pederbe/ripscope/blob/main/scripts/README.md), with
the [Linux](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md) and
[Windows](https://github.com/pederbe/ripscope/blob/main/docs/windows-release.md)
release guides.

# Contributing to Ripscope

Bug reports, compatibility reports, documentation fixes and code are welcome.
Open an issue to discuss a larger change before writing it.

## Report a problem

Use the [issue forms](https://github.com/pederbe/ripscope/issues/new/choose).
Include the smallest source example, commands and output you have. Incomplete
observations are welcome. For a difference from cscope, check the
[known differences](https://ripscope.com/docs/reference/differences-from-cscope)
and use the compatibility form.

Report suspected vulnerabilities privately through
[the security policy](SECURITY.md).

## Build and check a change

You need stable Rust and a C compiler for bundled SQLite. On Windows, install
the MSVC Build Tools and Windows SDK.

```sh
git clone https://github.com/pederbe/ripscope.git
cd ripscope
cargo build --locked
```

Before sending a change, run:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Code, tests and tools use Rust with `unsafe` forbidden. The
[language check](tests/language_check.rs) lists the only language exceptions.
Dependencies use exact versions. Explain why an added dependency is needed.

For specialized changes, follow the [editor checks](tests/editor/README.md),
[website guide](tools/site/README.md), [package guide](packaging/README.md) or
[development tool inventory](scripts/README.md). Check every documentation
example you add or change and use its real output.

## Preserve compatibility

[ARCHITECTURE.md](ARCHITECTURE.md) maps the code. Use the terms in
[GLOSSARY.md](GLOSSARY.md) and read the relevant
[design decisions](docs/adr/README.md).

Ripscope's compatibility contract covers result rows, ordering, duplicates,
context names, displayed text, diagnostics and exit status. Before changing
them, check the pinned cscope reference and the known differences. Include the
reference command and output in your pull request.

Discuss a new difference in an issue with a minimal reproduction, an explanation
of the incorrect cscope behavior and the intended answer. A Correction stays
local to the construct, can be switched off, and keeps a fixture showing both
readings. It is measured on real projects. Every readable file remains indexed.
The [quality method](docs/architecture/quality.md) and
[reference setup](scripts/README.md#reference-runtime) explain the checks.

Add a regression test for behavior changes, with a CLI integration test for
observable behavior. Record hard-to-reverse design changes in
[docs/adr/](docs/adr/README.md).

## Send a pull request

Create a branch in your fork and open a focused pull request against `main`.
Link the issue when there is one and update documentation for user-visible
changes. All changes enter `main` through a pull request with the required CI
check passing and the branch current with `main`.

Write imperative commit subjects and explain why when it is not obvious.
Use a conventional pull request title:

- `feat:` for a capability, `fix:` for a repair and `perf:` for a measured speed
  improvement.
- `docs:` for documentation and `fix(deps):` for dependency updates.
- `ci:`, `build:`, `chore:`, `test:` or `refactor:` for internal changes.

Release notes come from these titles. Write for someone using Ripscope and keep
performance claims tied to their measured workloads. Mark a breaking title with
`!` before the colon and put compatibility effects and upgrade instructions in
Solution. Keep Unreleased empty and correct wording in the original change.

## Public issues and pull requests

Issues use **Problem**, **Desired outcome** and **Evidence**. Pull requests use
**Problem**, **Solution** and **Validation**. Keep small changes brief. Public
descriptions and comments cover technical behavior, rationale and evidence.
Attribute checks accurately and keep private working notes out.

Keep facts and evidence current, and respect the reporter's observations and
wording when editing. Use checklists, linked issues and labels for progress.

Contributions are dual-licensed under MIT or Apache-2.0 unless stated otherwise.
This project follows the [code of conduct](CODE_OF_CONDUCT.md).

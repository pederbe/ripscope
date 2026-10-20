# Ripscope guidance

Ripscope answers cscope's searches (symbols, definitions, callees, callers, text,
egrep, files, includers, assignments), in line-oriented and one-shot mode, for C
source on Linux and Windows, from Vim, Neovim, Emacs and scripts. C only.

Start with `README.md` and `CONTRIBUTING.md`. `CONTEXT.md` defines the project's
terms, and `docs/adr/` records the decisions behind the design.

## Workflow

- Contributors: work on a branch of your fork and open a pull request against `main`.
- The maintainer works on `main` in `pederbe/ripscope` and may follow additional
  private procedures.

## Checks

Run these before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`cargo check --target x86_64-pc-windows-gnu --all-targets` compiles Windows-only
code on Linux. `tests/editor/README.md` describes the Vim, Neovim and Emacs checks,
and `tools/site/README.md` the website's.

All code, including tests and tools, is Rust. `tests/language_check.rs` lists the
only exceptions.

## Compatibility with cscope

Ripscope matches cscope's line protocol exactly, and every difference in what it
finds is measured against a pinned cscope release (ADR 0001, ADR 0002).
`evidence/README.md` and `docs/architecture/quality.md` describe the reference
captures, and how a change that reads C differently is shown and measured. cscope
is the yardstick, not the ceiling: Ripscope reads C correctly where cscope does not,
and shows both readings in a fixture.

## Decisions and terms

Record a new or changed design decision as an ADR in `docs/adr/`, and add new
terms to `CONTEXT.md`.

## Agent skills

### Issue tracker

Issues live in GitHub Issues in `pederbe/ripscope`. Before tracker operations, read
`docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage roles. Before assigning a triage state, read
`docs/agents/triage-labels.md`.

### Domain docs

Use one root `CONTEXT.md` and `docs/adr/`. Before exploring or changing domain
behaviour, read `docs/agents/domain.md`.

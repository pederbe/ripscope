# Architecture

Ripscope reads C source into an index and answers cscope's searches from it. This
page is a short map. The published
[architecture overview](https://ripscope.com/docs/architecture/overview) has the
diagrams, every module and the detailed design pages.

## Main parts

1. **Command line** (`src/invocation.rs`, `src/cli.rs`, `src/cli/`) reads options as
   cscope reads them and runs index builds, one-shot queries, sessions and `--verify`.
2. **Refresh** (`src/refresh.rs`, `src/refresh/`) applies the selection rule, finds the
   files that changed, follows their `#include`s and publishes one new index revision.
3. **Analyzer** (`src/analyzer.rs`, `src/scanner/`, `src/unit.rs`) reads one file's
   bytes into an analysis unit, the lines and navigation facts a query can return. It
   reads C as cscope does, with Corrections where cscope is wrong.
4. **Store** (`src/store.rs`, `src/store/`, `src/revision.rs`) is the index, one SQLite
   file, and the revision a query reads from it.
5. **Queries** (`src/query.rs`, `src/query/`, `src/engine.rs`, `src/engine/`) compile a
   field's pattern and answer the stored fields from analysis units.
6. **Sink** (`src/sink.rs`) writes result rows in cscope's line format or as JSON.
7. **Watch** (`src/watch.rs`, `src/watch/`) tells a session when its sources changed.

## How data flows

An update holds one SQLite transaction. Refresh keeps the analysis of every unchanged
file, has the analyzer read the changed ones, and commits a new revision. A reader
keeps the revision it started with until its next query.

A query for a symbol, definition, caller, callee, file, includer or assignment reads
one revision of the index. A text or regular-expression query reads the current
source files. Both hand their rows to the sink.

## Around the code

| Path | Contents |
| --- | --- |
| `tests/*_cli.rs`, `tests/fixtures/` | Integration tests and reference fixtures |
| `tests/editor/` | Real-editor workflow tests |
| `plugin/`, `lua/` | The Vim and Neovim plugin, where plugin managers find it |
| `contrib/` | The Emacs integration |
| `examples/`, `tools/`, `scripts/` | Reference capture and comparison tools, release builders and their guides, listed in the [tool inventory](https://github.com/pederbe/ripscope/blob/main/scripts/README.md) |
| `evidence/` | Reference captures and measurement records |
| `docs/` | The book, and the decision records in [docs/adr](https://github.com/pederbe/ripscope/tree/main/docs/adr) |
| `GLOSSARY.md` | The project's terms |

# How Ripscope is built

Ripscope is designed and maintained by [Peder Bergan](https://github.com/pederbe).
It combines a scanner for written C, a SQLite index and cscope's line-oriented
query protocol. The source and documentation explain how a result is obtained
and where the reading has limits.

## Written source, without a build

The scanner reads files without invoking a compiler or preprocessor. It records
occurrences, declarations, calls and their contexts, including inactive `#if`
branches. It does not resolve types, expand arbitrary macros or prove which calls
execute. Every readable file contributes the facts the scanner recognises;
uncertainty in one construct does not discard the whole file. See
[how Ripscope reads C](../architecture/reading-c.md) and
[limitations](../reference/limitations.md).

## A complete revision for each query

The index is one SQLite file. Updates commit through a transaction, while queries
read a complete revision. Stored symbol queries use the index; text searches
read the current source files. Ordinary Refresh uses file metadata and identity,
plus content hashes for recent edits. `--verify` reads every source file, including
Windows edits that preserve size and restore modification time. The
[storage and lifecycle guide](../architecture/storage.md) explains these choices.

## Evidence for specific behavior

- **A pinned reference.** Baseline observations come from a checksum-verified
  cscope executable. Source reading explains behavior; the executable supplies
  the captured output.
- **Separate intended readings.** A Correction changes a bounded reading where
  cscope is wrong. Paired fixtures keep both the reference observation and the
  intended result. Ambiguous cases and disabled candidates stay visible.
- **Independent source labels.** Navigation expectations from written C check
  correctness separately from agreement with cscope. Unassessed source remains
  unassessed, and a retained expectation does not mean the product passes it.

The [evidence index](../../evidence/claims.json) links these inputs and their
scopes. The [proof page](../reference/differences-from-cscope.md) gives concrete
examples, and [quality and compatibility](../architecture/quality.md) describes
how to run the checks. Finite fixtures do not establish arbitrary-C accuracy or
current corpus-wide performance.

## Implementation and checks

Ripscope is written in Rust with `unsafe` code forbidden, exact dependency pins
and a locked build. Unit tests exercise the scanner, query engine, storage and
Refresh. Integration tests run the real executable through one-shot queries,
sessions, warnings, JSON output and recovery. Separate editor checks drive Vim,
Neovim and Emacs in isolated projects.

Design choices and their tradeoffs are recorded in the
[architecture decisions](../adr/README.md). Start with the
[architecture overview](../architecture/overview.md) for the module boundaries,
or [Contributing](contributing.md) to build and test a change.

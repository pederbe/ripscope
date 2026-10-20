# Quality and compatibility method

Ripscope keeps cscope's line protocol, measures its baseline reading against the
Reference cscope, and answers queries with its own per-occurrence contexts
([ADR 0010](../adr/0010-own-query-semantics.md), "Ripscope's own query semantics give
each name the context it sits in"). Corrections change selected C readings, and every
query uses one complete index revision. This page explains how compatibility and
Navigation accuracy are checked separately.

## The reference decides baseline compatibility expectations

Expectations are captured from a pinned cscope executable, verified by checksum before
each capture ([ADR 0001](../adr/0001-pinned-reference-cscope.md)). Reading cscope's
source explains *why* it behaves as it does, and the executable decides *what* the
output is.

- **Scanner probes** (`tests/fixtures/scanner/`): small synthetic C files, each indexed
  on its own by the reference. Its database, decoded as cscope displays it, is the
  expected reading. Where an enabled Correction reads a probe differently, its reading is
  kept beside cscope's in `intended/`. A probe in `open/` keeps cscope's records of a
  reading the baseline does not reproduce, where Ripscope defines its own behaviour
  ([ADR 0004](../adr/0004-own-scanner.md)), and a unit test pins Ripscope's reading
  against them.
- **Query probes** (`tests/fixtures/queries/`): small projects queried by the reference,
  every field and pattern kind. The sequential oracle must reproduce the captured
  answers, and separate intended answers check the product's own semantics and
  Corrections through the command line.

## The compass

Compatibility is measured against the reference
([ADR 0002](../adr/0002-measured-compatibility.md)). The compass compares Ripscope
with the reference on declared selections of real C source. Some projects are held out
from designing Corrections and only measured. The comparison views are:

- `units`: each selected file's analysis against cscope's own database with
  Corrections off.
- `engine`, `store` and `names`: whole query streams and sampled exact-name queries.
- `fidelity`: every stored field, per file and in order.

The [evidence index](../../evidence/claims.json) maps each claim to its source
inputs, reference captures and independent labels. Captures keep their original
bytes and reference identities.

Each Correction is measured on the corpus: rows gained and lost, false readings
removed and real definitions lost. A clear fix is enabled once its kept readings are
checked and no real definition is lost. A Correction that trades off a correct
reading, or changes displayed text or the protocol, is a product decision
([ADR 0002](../adr/0002-measured-compatibility.md), "Compatibility with cscope is
measured, and its misreadings are corrected").

## Accuracy independently of the reference

The [source-labelled assessment](https://github.com/pederbe/ripscope/blob/main/scripts/navigation-assessment.md)
checks exact-name definitions and both call directions against expectations labelled
from written source. Its C89, C99 and C11 fixtures keep their expectations even when
the product misses a name or invents a call. Pinned samples of real projects check
source discovery and index completion separately from navigation accuracy, and audit
the answers inside explicitly labelled ranges.

Project workflow assessments also check discovery, all nine searches, source edits,
Refresh and editor navigation. A shared Linux and Windows selection keeps source bytes
and relative paths identical, and records each platform's paths, product and editor
identities separately.

## Tests

| Layer | What it covers |
| --- | --- |
| Unit tests (`src/**`) | The scanner against every probe in both readings, a seeded totality test over arbitrary bytes, the query engine, the store, refresh, the invocation and the sink. |
| CLI integration tests (`tests/*_cli.rs`) | The real executable: every query field through the command line, sessions with standard output and error in one pipe (as Vim reads them), JSON framing, selection rules, updates and recovery. |
| Editor tests (`tests/editor/`) | Drive real Vim, Neovim and Emacs against a built executable in isolated environments. |

`cargo test --locked` runs the unit and integration suites on Linux and Windows. Editor
checks and the compass run separately, because they need the editors, the reference
cscope and the corpus sources.

## Performance

Session startup and query latency come first, measured against the fastest
applicable cscope configuration for each operation. Index build time comes second,
and memory, CPU and index size are supporting measurements. The
[benchmarks](../reference/benchmarks.md) page gives the measured search times and
their conditions.

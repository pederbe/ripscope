# Evidence and its scope

`claims.json` connects documented behavior with the inputs and checks behind it. The
website copies the files `sources.json` selects into its `evidence/captures/` folder
without altering their bytes. Tests read their canonical copies in
`tests/fixtures/`, so each capture has one maintained source.

The reference is Ubuntu `cscope 15.9-2build1`, amd64, with executable SHA-256
`c2b68c8d216835b5b0637fcf41a40c59d6b73a81cc411136ec224cfcff01401c`.
Captures record the commands, source selection, diagnostics and reference identity.
Scanner captures describe synthetic files indexed with `-bk`, and query captures also
keep sequential and inverted-index answers. The `intended/` scanner readings are
Ripscope's expected readings with Corrections, including disabled candidates. They are
not observations of cscope.

These inputs show particular readings and limits. The navigation labels in
`captures/navigation/` are expectations from written C, not output of either program,
and a missed expectation stays a miss. The zlib fixture keeps its complete captured
source text, source hashes, upstream revision, copyright notice and bounded
expectations.

Some fixture metadata names a file in the project's research archive. Those strings
record provenance, and tests do not load the archive. The reference copies document
their absolute-path redactions in `tests/fixtures/reference/README.md`.

To inspect or reproduce a capture, use its public source and query pointers:

| Capture | Public inputs and replay route |
| --- | --- |
| `captures/scanner/expected/*.json` | The capture's `probe` points into `captures/scanner/probes/`, and the canonical sources are in `tests/fixtures/scanner/probes/`. The `scanner-probes` example records the reference reading. |
| `captures/queries/*/expected.json` | Source files and `queries.txt` sit beside the capture, with canonical copies in `tests/fixtures/queries/`. The `query-probes` example records sequential and inverted-index answers. |
| `captures/reference/cscope-baseline.json` | Each case embeds its exact `files`, build arguments and query arguments. Recreate those relative files in a fresh temporary directory and run the identity-checked reference with the recorded arguments and C locale. |
| `captures/reference/selection-oracle.json` | Each case embeds `files` and ordered `runs`. Its `workspace` field is provenance, not a required directory. Replay in a fresh directory without changing the relative file names. |
| Approved fixtures in `tests/fixtures/` | Source strings, paired `*-cscope.json` captures and named fixture fields keep their inputs. The reading rules are in `docs/architecture/reading-c.md` and the paired scanner probes. |

Reference replay setup is in `scripts/README.md`. The two probe programs overwrite
canonical expected files, so replay in a disposable source copy and compare the
results before replacing a maintained capture. Source-labelled navigation uses
the separate seal and run procedure in `scripts/navigation-assessment.md`.

Ripscope's scanner is its own implementation. cscope's behavior is credited under the
notice in `CSCOPE-NOTICE.txt`, and Ripscope is not affiliated with cscope.
Third-party fixture source keeps its own copyright notices and licenses.

## Search benchmarks

`performance-20261019.json` holds the medians, quartiles, sample counts, calibration
floors, query identities and source revisions behind the benchmark tables. It was
measured on 18 and 19 October 2026 on the 0.1.0 release candidate, product revision
`2fdce5902704e6e573b1a6bcd7e57a7477136e71`, and keeps Linux and Windows repetitions
separate.

`performance-answers-20261016.json` holds the differing ordered-answer variants, the
source judgments and the inspected function-context witnesses. It comes from the
16 October 2026 comparison, and the release candidate returns byte-identical answers
to every query, so its judgments apply unchanged. Source excerpts keep their upstream
ownership and the licenses of the pinned Linux, Redis and zlib revisions, which the
benchmark answer page links.

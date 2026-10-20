# Full-tree compass runs

The `compass-scale` example supplements the fixed corpus runner with native Linux runs
over a clean source checkout. It compares a pinned reference cscope with a release
Ripscope binary, following ADR 0001, "A pinned cscope executable defines expected
results", and ADR 0002, "Compatibility with cscope is measured, and its misreadings are
corrected".

```sh
cargo build --release --locked --example compass-scale
target/release/examples/compass-scale \
  --binary /absolute/path/to/ripscope \
  --reference /absolute/path/to/pinned/cscope \
  --source /absolute/path/to/source-checkout \
  --output /absolute/path/to/new-results-directory
```

Use native Linux storage for the source and output. The output directory must be new
and outside the source checkout. GNU `sort`, `/usr/bin/time` and Git are required. No
source configure or build step runs. Allow tens of GB for raw
scanner records, query streams, sorted streams and separate indexes on a full kernel.
The default child-process deadline is 900 seconds, and `--timeout` changes it.
The runner prefers `gnusort` where it is installed and checks the sorter identity.
It also checks that sorting preserved the captured row counts.

The run records the source revision, executable hashes, source and selected-file
manifests, commands, exit statuses, diagnostics, wall times and peak resident memory.
The source cache is warmed by reading tracked files. Each timing is a single sample,
including startup, rather than a performance distribution. Keep the machine otherwise
idle during the index builds and refreshes.

It builds separate reference indexes with `-bkR` and `-bkqR`. Only the default index
defines comparison results. It builds the product with Corrections enabled and a
second Store with Corrections off, compares their file order with the reference,
checks every baseline Analysis unit, and captures fields 0, 1, 2, 3, 7, 8 and 9 with
`.*`. The baseline Store uses the Oracle's sequential semantics. The product uses
its own semantics (ADR 0010). No result is sampled.

`summary.json` distinguishes ordered equality from duplicate-preserving row agreement.
Nonzero exits, diagnostics, malformed rows and incomplete hidden-command frames make
a stream unusable. Partial streams remain available for diagnosis, but their counts
are not successful compatibility results. A completed run need not show parity:
Corrections and own semantics change product answers, and the reference itself has
defects. The runner does not automatically attribute mismatches to those causes.

The decoder and comparisons retain one file's records and bounded sorting buffers.
The hidden Oracle command still reads the whole revision and collects a field's rows
in memory. Its process measurements expose that cost. Do not split it into file
batches, because the sequential semantics carry context between files.

For system-header coverage, provide `--sysroot /absolute/path/to/pinned-tree`. That
tree must contain `include/` and `include/x86_64-linux-gnu/`. This omits `-k`, sets
`INCDIR` to the former and `INCLUDEDIRS` to the latter, and leaves all other ambient
search variables out. Pin the header package versions alongside the run. Every reached
header must resolve inside the snapshot or source checkout, and escaping symlinks stop
the run. Confirm the selected-file manifest reaches both include directories.

After each run, keep the summary, manifests and raw captures outside the repository.
The before/after manifests and final clean-tree check detect source changes during
measurement.

The compass and decoder tests run with ordinary `cargo test`, or on their own with:

```sh
cargo test --locked --example compass-scale
```

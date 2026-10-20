# Conditional-head candidate inventory

The `conditional-heads` Rust example inventories conditional-head suspects over
explicit, original corpus manifests. It preserves scanner and source-discovery
observations and chooses a bounded inspection sample. It does not label correct
functions or decide whether several written heads share a body.

```sh
cargo build --example conditional-heads
target/debug/examples/conditional-heads inventory /absolute/ripscope /absolute/frame.json /absolute/new-output-directory
```

The frame is JSON with `method: "conditional-heads-v2"`, `method_sha256`, the
frozen `binary_sha256`, `enabled_corrections` and at most 40 `projects`. Each
project supplies `name`, absolute `root`, absolute original `manifest` and
`manifest_sha256`. A manifest's `corpus` supplies `revision` and `files`, each
with `path`, `bytes` and `sha256`. Record the exact tool revision separately beside
the frame. The product reading must explicitly retain any disabled Correction.

Before capture, all revisions, manifest hashes, selected bytes and source targets
are checked. Selected path spellings and traversed symbolic links are retained,
and the same identities are checked after capture. No files are acquired or modified.
The baseline and enabled `--dump-units` commands run sequentially with file-backed
captures, a 600-second limit each, an 8 GiB aggregate artifact guard and the shared
capture driver's output limits. An optional frozen `capture_seconds` can shorten
the 600-second limit but cannot extend it. Linux memory scopes default to 8 GiB. Set
`memory_scope: false` only for environments without that support. A failed command,
warning, missing file, malformed stream or identity change yields incomplete
evidence and no valid sample. An existing output directory is never overwritten.
Dump validation checks physical source bounds, line order, Latin-1 string
fragments/names, recognized mark bytes and complete selected-file membership.

## Candidate discovery

The current mark frame contains runs of at least two distinct Function names
without a FunctionEnd between them. The baseline frame retains such runs that
no longer contain two distinct enabled heads. EOF runs are flagged incomplete.
Runs in either frame are only suspects.

The independent source supplement masks comments, quoted tokens and directive
payloads. It keeps sibling conditional arms with head-like suffixes and a shared
opening brace, sibling opening braces with a shared continuation, or sibling
bare names followed by shared parameters and a body. Equal names remain source
suspects, although they do not add a distinct caller. This is lexical discovery,
without expression evaluation, macro expansion or C parsing. It reports malformed
tokens, unsupported token splices, orphan or misordered conditional directives,
unmatched groups, groups over 256 KiB and supported tail-window limits
separately. It can miss conditional declarations.
Continuations between separate tokens retain their physical offsets and line
numbers. Splices joining identifiers or comment delimiters remain explicitly
unassessed.
The 512-byte forward window after `#endif` must cover the shared parameter list
and body cue. Reaching its bound without them is an explicit unassessed scope.

Validate every proposed source name against its complete original physical line
before using it as an inspection anchor or query. A cue can truncate a name, for
example `OPENSSL_INIT_set_config_appname` to `NIT_set_config_appname`. Keep such an
invalid record and its judgment in the sample, excluded from call-loss scoring and
without replacement. A tool repair needs a separate frozen run, rather than rewriting
a completed sample.

Primitive candidates retain intervals, origins, proposed head locations and
identities. Transitively overlapping intervals form components for sampling.
Their primitives are not replaced by one relationship. `candidates.json` and
`report.json` count primitives and components separately. Bounds in the source
supplement can coexist with a complete paired-mark capture. `source_discovery_complete`
records these limitations independently of `paired_mark_capture_complete`.
The report's `complete` means execution, validated captures and identity checks
completed. It does not mean exhaustive source discovery or inspected correctness.
Each project's counts name source completeness, the number of limitations and a
pointer to its details in `candidates.json`. Counts describe discovered suspects.
Declared lexical limits retain remaining conservative source candidates and
complete mark evidence. They do not suppress a whole project or stratum. The
sample repeats global source completeness and its limitation pointer. Failed or
malformed captures, identity changes and deadlines still leave no valid sample.

## Fixed sample and inspection

For each project, components have three strata in priority order: containing a
current primitive, otherwise a baseline primitive, otherwise source primitives.
Take one minimum SHA-256 component tuple per stratum, at most 120 components.
Within each selected component take one minimum-hash primitive of each origin,
then fill remaining slots by hash order, at most four. Unselected components and
primitives remain unassessed. Rejected observations are never replaced.

The tuple starts with `conditional-heads-v2` and NUL-separated project, file,
start and end. Primitive ranking adds origin and ordered JSON head locations.
`sample.json` retains the ranking hashes, chosen records and these fixed windows:

- Head relationship: up to 32 preceding lines / 4 KiB and the anchor plus 127
  following lines / 12 KiB. `head_window` records actual physical offsets.
- Body calls: up to 256 lines / 24 KiB, starting at the earliest independently
  proved shared-body join. One body window per primitive even if it contains
  several relationships. A proof does not automatically establish the body end.
- Neighbor controls: up to 64 lines / 8 KiB before the earliest proved head and
  after an established body end. Unknown body ends leave following controls
  unassessed.

Every byte/line bound stops at whichever is reached first. Additional relationships
never allocate extra windows. Correctness classifications belong to independent
source labels. The inventory CLI tests only preservation, selection and metadata.
No-call judgments require the entire body to have been inspected. A partial body
with no observed calls remains unassessed. Use `assess-navigation` for sealed
exact caller/callee labels and stored-query scoring, retaining unassessed rows and
tool limitations.

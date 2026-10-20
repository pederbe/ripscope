# Source-labelled navigation assessment

The `assess-navigation` Rust example checks all search fields against independently
labelled source locations, with the strongest inventories for exact definitions
and both directions of caller/callee searches.
It uses the stored CLI with Corrections enabled. Expectations come from written C
and the product contract, independently of cscope or product output.

This complements the compatibility compass in
[the quality method](../docs/architecture/quality.md), which measures agreement with
cscope rather than whether each declaration and call was understood.

## Finite fixture inventory

Inputs and independent labels live in `tests/fixtures/navigation/`. The matrix
is deliberately finite, rather than a language-conformance suite.

| Input | Constructs |
|---|---|
| `c89.c` | Ordinary and qualified objects. Pointers and arrays. Function-pointer objects, members and nested declarators. Scalar, array and function typedefs. Struct, union and enum tags. Ordinary, recursive, callback and old-style functions. Direct, named indirect, dereferenced and member calls. Repeated calls on one line. Prototype/parameter and keyword controls. |
| `c99.c` | Boolean and complex types, `restrict`, bit-fields and flexible arrays, inline functions, array parameters and variable-length arrays, designated initializers and compound literals. |
| `c11.c` | Both atomic declaration forms, atomic typedefs, thread storage, alignment, anonymous unions, static assertions, generic selection and `_Noreturn`, each with neighboring controls. |
| `policy.c` | Written macro definitions, calls in replacement text and macro invocations. Both conditional arms. A name produced only by macro expansion. |

The standard fixtures pass strict GCC syntax checks in their respective language
versions: `gcc -std=c89` (or `c99` or `c11`)
`-pedantic-errors -Wall -Wextra -fsyntax-only <input>`. Language rules are described
in [WG14 N1570](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf), especially
6.5.1.1, 6.7.2.4 and 6.7.5 for the C11 forms.

`reductions/` retains isolated atomic, generic-selection, alignment-expression,
dereferenced-call and nested-declarator inputs with positive neighboring controls.
Its manifest is a separate confirmation inventory, so adding a confirmation does
not change the initial matrix's denominators. Use that directory as the source
root and its `matrix.json` as the draft to run those inputs.

The policy fixture follows ADR 0003, "Each file is read as written, without a compiler
or preprocessor": all conditional arms are read, and macro expansion and resolving an
indirect call to its runtime target are outside the assessment. A written indirect
call is labelled by its written name. Enum constants and their enum tag are assessed as definitions.

## Run

Build the ordinary CLI and the example, then seal the labels before inspecting
any output. Paths below are placeholders for fresh files/directories in the OS
temporary directory. On Windows select `target/debug/ripscope.exe`.

```sh
cargo build --bin ripscope --example assess-navigation
cargo run --example assess-navigation -- seal tests/fixtures/navigation tests/fixtures/navigation/matrix.json /path/to/manifest.json
cargo run --example assess-navigation -- run target/debug/ripscope tests/fixtures/navigation /path/to/manifest.json /path/to/new-output-directory
```

`seal` pins the selected source bytes and materializes exact queries, scopes and
expected rows. `run` requires an unused output directory, verifies source hashes
before and after measurement, builds an isolated index from an explicit file list,
captures discovered paths, and runs the declared queries with `-dL --json`. Legacy
manifests declare fields 1, 2 and 3. Include
environment variables are cleared, and `-k` excludes system headers. Literal local
includes can still extend the selection. Report those additional files separately
and pin them before making accuracy claims about their contents.

New seals also pin each discovery path's canonical target and the spelling of
every traversed symbolic link. Equal-byte retargeting, directory-link changes,
missing targets and content changes invalidate those identities before any build
or query. A conservative safety pass follows local literal includes before index
commands, including for legacy manifests. External absolute operands or escaping
aliases produce an explicit `unsupported_inputs` report with
`comparison_eligible: false`. Host bytes are never included by that pass.
Include-looking comments and strings can also make the assessment incomplete. This
is a limitation of the assessment tool.

Index builds have a 1,800-second limit, discovery 900 seconds, and exact queries
120 seconds. Failed or malformed queries remain unmeasured, with raw partial
captures. Changed identities and unpinned discovery inputs stop the assessment.
Commands remain available. Navigation errors are reported in `report.json` and do
not make the example fail: keeping an independently correct expectation for a
known failure is essential to the assessment.

## Label format and counting

A draft JSON manifest provides `files`, `definitions`, `calls`, `controls`,
`whole_files`, `unresolved`, and a written sampling rule. Real-project manifests
also record upstream URL, revision, exclusions and missing generated inputs.

- `definitions`: `[path, line, name, construct]`.
- `calls`: `[path, line, caller, callee, construct]`, where the line is the one
  where the written called name starts, including multiline expressions.
- `controls`: `[field, name, construct]` for a complete expected empty answer
  inside the labelled scope. A control may not duplicate a positive query.
- `whole_files: true`: every selected file is the assessed scope for each query.
  Otherwise the selected definition lines form field 1's scope and the selected
  call lines form fields 2/3's scope. Label **all** matching answers within those
  scopes before querying. Every invocation on a selected call line needs a label.
- `unresolved`: source ambiguities deliberately withheld from the scored facts,
  with locations and reasons. Nonempty inventories require `unscored_queries`
  triples `[field, pattern, reason]`. These queries never become absence controls.
- Optional `index_files` and `include_directories` separate the complete indexed
  selection from assessed `files`. `closure_files` pins extra include inputs.
- Optional `query_inventory` triples `[field, pattern, construct]` add complete
  expected-empty queries, including all written identifiers and empty call contexts.
- Optional `extra_queries` provide explicit `field`, `pattern`, `construct`,
  `expected` rows and, when necessary, `scopes`. `match_text: true` checks the
  fifth expected column against live result text. File-inventory searches assess
  every returned file, including unexpected paths.
- Optional `query_limit` bounds the fixed field/pattern order. Queries past it
  remain unmeasured. By default there is no limit.

Identical name/caller/path/line call locations are one labelled navigation fact,
even if a name occurs twice on the line. The draft retains written occurrences.
The sealed expected rows collapse identical locations. The two directions are
separate checks of the same fact. Different exact definition names on one line
each have their own query and expected row, following ADR 0010, "Ripscope's own
query semantics give each name the context it sits in".

The scorer matches row multisets by path, context and line, with exact matches
first. Remaining answers can establish a wrong context or location only with
unambiguous pairing inside the labelled ranges. Other expected answers are misses
and other assessed rows are spurious. Text is retained in the raw result and is
scored only with `match_text`. In field 2, the context column is the **callee**, rather
than the enclosing function. Field 3 checks that enclosing function directly.

Scope admission uses only path and line, never the returned context. Answers
outside labelled ranges are **unassessed**, including a possibly displaced answer
outside a single-line scope. Counts of unassessed answers are per query, so one
source location may be counted again when another query returns it.

`report.json` retains per-field and per-construct counts, individual outcomes,
unassessed rows, unresolved labels, selected/discovered file counts, binary and
manifest hashes. `build.json`, `discovery.json` and `query-NNN.json` retain exact
arguments, current directory, exit status, stdout and stderr. Keep the sealed
manifest, source revision, tool revision and these captures together. Keep source
samples and working assessments outside the repository. Only reusable synthetic
fixtures belong in the tree.

For a query mixing construct labels, spurious/unassessed counts in `constructs`
use its first label. They describe that query, rather than an inferred construct
of an unlabelled answer. Expected-fact outcomes always use their own source label.

# Differences from cscope

Ripscope keeps cscope's line-oriented query protocol, with its own C scanner,
per-occurrence query contexts and transactional index updates. This page explains
the deliberate differences and points to the inputs that show them.

Compatibility with cscope and correctness against written C are checked separately.
The [evidence index](../../evidence/claims.json) names the inputs and checks behind
each difference, and the [limitations](limitations.md) page lists what Ripscope does
not read.

## Updates and index safety

| Behavior | Contract and check |
| --- | --- |
| Complete revisions | An update commits one SQLite transaction. A query reads one complete revision, and an open session sees a later commit on its next query. [Storage and lifecycle](../architecture/storage.md) explains the contract, and `tests/pipeline_session_cli.rs` and `src/store/tests.rs` exercise it. |
| Changed sources | Ordinary Refresh compares size, file times and identity, and hashes recently modified files. `--verify` or `-bu` reads every file. On Windows, an in-place edit that preserves size and restores modification time can be missed by ordinary Refresh. [ADR 0007](../adr/0007-metadata-based-refresh.md) and `tests/freshness_cli.rs` cover this boundary. |
| Source removal and read failures | Stored symbol queries can answer after sources are deleted. Text searches read live files and skip unreadable ones with a warning. A failed update keeps the preceding revision. [Recovery](../guide/recovery.md), `tests/persistent_cli.rs` and `tests/pipeline_selection_cli.rs` cover these cases. |
| Existing files | An index build accepts an absent path, an empty file or a Ripscope index. It refuses a cscope database or unrelated file. Local-storage restrictions and sidecar-file checks are described in [limitations](limitations.md#platforms-and-storage). |

Run these checks with `cargo test --locked`. An unchanged stored index and a live
text search can show different source states, and
[stored and live queries](../guide/queries.md) explains when to update.

## Corrections: reading C where cscope misreads it

A Correction changes the reading of a specific written construct. Each keeps the
reference observation and the intended reading side by side
([ADR 0002](../adr/0002-measured-compatibility.md)). The scanner
does not expand macros or resolve types, so an ambiguous construct can still lose a
correct reading. The [recognition limits](../architecture/reading-c.md#12-corrections)
list those cases.

Examples with source and paired captures:

| Written construct | Difference and evidence |
| --- | --- |
| `extern "C" { ... }` in a C header | The linkage-block Correction prevents cscope's declaration state from hiding later definitions. [Source](../../evidence/captures/scanner/probes/33-linkage-blocks.c), [reference](../../evidence/captures/scanner/expected/33-linkage-blocks.json), [intended reading](../../evidence/captures/scanner/intended/33-linkage-blocks.json). |
| A continued macro with CR LF endings | The CR LF continuation Correction keeps its body together. [Source](../../evidence/captures/scanner/probes/34-crlf-continuations.c), [reference](../../evidence/captures/scanner/expected/34-crlf-continuations.json), [intended reading](../../evidence/captures/scanner/intended/34-crlf-continuations.json). |
| A pointer-to-function or pointer-to-array declaration | Supported declarator shapes define the member or global name rather than its parameter or type. [Source](../../evidence/captures/scanner/probes/41-pointer-declarators.c), [reference](../../evidence/captures/scanner/expected/41-pointer-declarators.json), [intended reading](../../evidence/captures/scanner/intended/41-pointer-declarators.json). |
| Parameters of a function typedef | Recognised parameter names and types stay out of definition results. [Source](../../evidence/captures/scanner/probes/48-typedef-parameters.c), [reference](../../evidence/captures/scanner/expected/48-typedef-parameters.json), [intended reading](../../evidence/captures/scanner/intended/48-typedef-parameters.json). |

Other Corrections cover atomic types, attributes, directive and macro boundaries,
initializer entries, function heads and typedef decorations. Their rules and the
ambiguous shapes they leave alone are in [how Ripscope reads C](../architecture/reading-c.md).
The [evidence guide](https://github.com/pederbe/ripscope/blob/main/evidence/README.md)
tells reference captures, intended expectations, open readings and disabled
candidates apart.

### Assignment search

Field 9 includes supported array initializers, direct named subscript stores and
direct named prefix and postfix increments and decrements. It reports written sites,
including macro bodies and unevaluated operands, whether or not they run, and does not
resolve pointer aliases. The paired [subscript-store
source](../../evidence/captures/scanner/probes/71-subscript-assignments.c) and
[named-mutation source](../../evidence/captures/scanner/probes/72-named-mutations.c)
have reference and intended captures beside them. [ADR 0012](../adr/0012-written-assignment-search.md)
gives the exact recognition boundary, and `tests/assignment_cli.rs` checks the query
interface.

## Query results

cscope's sequential and inverted-index searches can give different rows. Ripscope
uses one set of [query semantics](../architecture/answering-queries.md): it reads
names in order and gives each occurrence the context it sits in within its file
([ADR 0010](../adr/0010-own-query-semantics.md)).

| Case | Ripscope's reading |
| --- | --- |
| A matching type appears before the function name in a head | Reading continues through every name, so the function still becomes the body's context. |
| Alternative function heads under `#if` share a body | Only the latest head is the call's function context, so a callee query for an earlier alternative head finds no calls. |
| A macro is defined inside a function body | Calls in its body use the macro context. Callee queries match that context rather than the surrounding function. |
| A file ends with an open context | Context does not carry into another file. |
| More than one definition occurs on a line | A definition query gives one row per matching line. |
| An include occurs inside a function | The includer result uses `<global>`. |

The five synthetic query-project groups keep their source files, patterns, cscope
commands, sequential and inverted-index rows, diagnostics and exit statuses
together. The [semantics capture](../../evidence/captures/queries/05-semantics/expected.json)
shows these reference differences. `tests/intended/mod.rs` checks the product's
intended answers, and [quality and compatibility](../architecture/quality.md)
describes the separate baseline and product checks.

### Patterns, bytes and displayed text

- Patterns use the documented [regular-expression dialect](../guide/queries.md#patterns).
  Ripscope anchors a definition-query alternation as a whole and refuses unsupported
  expressions with a diagnostic. A pattern can contain up to 4,096 bytes.
- Names are stored, matched and shown whole, including names beyond cscope's
  250-byte buffer. The [long-name reference observation](../../evidence/captures/scanner/open/expected/long-define-names.json)
  and [ADR 0004](../adr/0004-own-scanner.md) cover this case, where Ripscope defines
  its own behavior.
- Stored rows preserve UTF-8 and display control bytes visibly as `\xHH`.
  Live text-search rows keep tabs and CR LF endings, while escaping other control
  bytes. On a Windows console, bytes that are not UTF-8 are escaped too.
- Editor row parsers written for cscope can split paths at spaces. The
  [JSON interface](../guide/json.md) carries exact path and line bytes, and the
  Neovim plugin uses it.

## Independent checks

Independent [written-source labels](../../evidence/captures/navigation/matrix.json)
check C89, C99, C11 and policy fixtures. They keep their expectations even where the
product misses a name or invents a call. The [zlib header fixture](../../evidence/captures/real-header-approved.json)
records its source and the product revision it was made with. Search times and
answer comparisons on real projects are on the [benchmarks](benchmarks.md) page.

The best check is the navigation you need in your own project. Please
[report an unexpected difference](https://github.com/pederbe/ripscope/issues/new?template=compatibility.yml)
with the source, exact commands and program versions.

## Not supported

- cscope's interactive terminal interface and field 5 text-changing features.
  In line mode, field 5 is a text search, as in cscope.
- Reading or writing cscope databases.
- `-T` and `-s`, which would change the answers. They are refused rather than ignored.

`-c`, `-p N` and `-F FILE` change nothing in line mode, so Ripscope accepts and ignores
them, as it does `-q`, `-e`, `-U` and `-v` (see the [command line](cli.md)).

Ripscope is an independent project and is not affiliated with cscope. cscope's
interface and observable behavior are credited in the
[cscope notice](../../evidence/CSCOPE-NOTICE.txt).

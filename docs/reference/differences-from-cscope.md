# Differences from cscope

Ripscope takes cscope's command line and prints cscope's rows, but it reads C with
its own scanner, keeps its own index and updates that index differently. The
deliberate differences are explained below. To move an existing setup
over, follow [Switch from cscope](../guide/switch-from-cscope.md).

## Updates and index safety

| Behavior | What Ripscope does |
| --- | --- |
| Complete revisions | An update commits one SQLite transaction. A query reads one complete revision, and an open session sees a later commit on its next query. |
| Changed sources | An ordinary update compares each file's size, times and identity, and hashes recently modified files. `--verify` or `-bu` reads every file. On Windows, an in-place edit that preserves size and restores modification time can be missed by an ordinary update. |
| Source removal and read failures | Stored symbol queries can answer after sources are deleted. Text searches read the current files and skip unreadable ones with a warning. A failed update keeps the preceding revision. See [Check and repair an index](../guide/recovery.md). |
| Existing files | An index build accepts an absent path, an empty file or a Ripscope index. It refuses a cscope database or unrelated file. The storage it accepts is described in [limitations](limitations.md#platforms-and-storage). |

An unchanged stored index and a text search of the current files can show different
states of the source. [Stored and live fields](../guide/queries.md#stored-and-live-fields)
explains when to update.

## Corrections: reading C where cscope misreads it

Ripscope reads C the way cscope does, except for specific constructs that cscope
gets wrong. Each such change is called a Correction. For every Correction the project
keeps the source, what cscope reports and what Ripscope reports, so you can check
the claim yourself. Four examples:

| Written construct | What changes |
| --- | --- |
| `extern "C" { ... }` in a C header | The linkage-block Correction prevents cscope's declaration state from hiding later definitions. [Source](../../evidence/captures/scanner/probes/33-linkage-blocks.c), [reference](../../evidence/captures/scanner/expected/33-linkage-blocks.json), [intended reading](../../evidence/captures/scanner/intended/33-linkage-blocks.json). |
| A continued macro with CR LF endings | The CR LF continuation Correction keeps its body together. [Source](../../evidence/captures/scanner/probes/34-crlf-continuations.c), [reference](../../evidence/captures/scanner/expected/34-crlf-continuations.json), [intended reading](../../evidence/captures/scanner/intended/34-crlf-continuations.json). |
| A pointer-to-function or pointer-to-array declaration | Supported declarator shapes define the member or global name rather than its parameter or type. [Source](../../evidence/captures/scanner/probes/41-pointer-declarators.c), [reference](../../evidence/captures/scanner/expected/41-pointer-declarators.json), [intended reading](../../evidence/captures/scanner/intended/41-pointer-declarators.json). |
| Parameters of a function typedef | Recognised parameter names and types stay out of definition results. [Source](../../evidence/captures/scanner/probes/48-typedef-parameters.c), [reference](../../evidence/captures/scanner/expected/48-typedef-parameters.json), [intended reading](../../evidence/captures/scanner/intended/48-typedef-parameters.json). |

Other Corrections cover atomic types, attributes, directive and macro boundaries,
initializer entries, function heads and typedef decorations.

Ripscope does not expand macros or resolve types, so an ambiguous construct can still
be read wrongly. [How Ripscope reads C](../architecture/reading-c.md#12-corrections)
gives every rule and the shapes it leaves alone.

### Assignment search

Field 9 includes supported array initializers, direct named subscript stores and
direct named prefix and postfix increments and decrements. It reports written sites,
including macro bodies and unevaluated operands, whether or not they run, and does not
resolve pointer aliases. The paired [subscript-store
source](../../evidence/captures/scanner/probes/71-subscript-assignments.c) and
[named-mutation source](../../evidence/captures/scanner/probes/72-named-mutations.c)
show both tools' answers. The
[assignment search decision](https://github.com/pederbe/ripscope/blob/main/docs/adr/0012-written-assignment-search.md)
gives the exact boundary of what is recognised.

## Query results

cscope can give different rows for the same search depending on whether its database
was built with `-q`. Ripscope always answers the same way. It reads names in order and
gives each occurrence the context it sits in within its file.

| Case | Ripscope's reading |
| --- | --- |
| A matching type appears before the function name in a head | Reading continues through every name, so the function still becomes the body's context. |
| Alternative function heads under `#if` share a body | Only the latest head is the call's function context, so a callee query for an earlier alternative head finds no calls. |
| A macro is defined inside a function body | Calls in its body use the macro context. Callee queries match that context rather than the surrounding function. |
| A file ends with an open context | Context does not carry into another file. |
| More than one definition occurs on a line | A definition query gives one row per matching line. |
| An include occurs inside a function | The includer result uses `<global>`. |

The [semantics capture](../../evidence/captures/queries/05-semantics/expected.json)
records cscope's rows for these cases, with and without `-q`.
[How Ripscope answers stored queries](../architecture/answering-queries.md) describes
the rules in full.

### Patterns, bytes and displayed text

- Patterns use the documented [regular-expression dialect](../guide/queries.md#patterns).
  Ripscope anchors a definition-query alternation as a whole and refuses unsupported
  expressions with a diagnostic. A pattern can contain up to 4,096 bytes.
- Names are stored, matched and shown whole, including names beyond cscope's
  250-byte buffer. The [long-name capture](../../evidence/captures/scanner/open/expected/long-define-names.json)
  shows what cscope does with them.
- Stored rows preserve UTF-8 and display control bytes visibly as `\xHH`.
  Live text-search rows keep tabs and CR LF endings, while escaping other control
  bytes. On a Windows console, bytes that are not UTF-8 are escaped too.
- Editor row parsers written for cscope can split paths at spaces. The
  [JSON interface](../guide/json.md) carries exact path and line bytes, and the
  Neovim plugin uses it.

## Check it yourself

The [benchmarks](benchmarks.md) compare both tools' answers and search times on the
Linux kernel, Redis and zlib, and explain every row where they differ. The
[evidence index](../../evidence/claims.json) lists the inputs behind each difference
on this page, and [quality and compatibility](../architecture/quality.md) describes
how the project measures them.

The best check is the navigation you need in your own project. Please
[report an unexpected difference](https://github.com/pederbe/ripscope/issues/new?template=compatibility.yml)
with the source, exact commands and program versions.

## Not supported

- cscope's full-screen terminal interface and its text-changing feature.
  Field 5 is a text search, as in cscope's line mode.
- Reading or writing cscope databases.
- `-T` and `-s`, which would change the answers. They are refused rather than ignored.

`-c`, `-p N` and `-F FILE` change nothing in line mode, so Ripscope accepts and ignores
them, as it does `-q`, `-e`, `-U` and `-v` (see the [command line](cli.md)).

Ripscope is an independent project and is not affiliated with cscope. cscope's
interface and observable behavior are credited in the
[cscope notice](../../evidence/CSCOPE-NOTICE.txt).

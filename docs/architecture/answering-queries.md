# How Ripscope answers stored queries

This page specifies how the query engine (`src/engine.rs`) answers the stored query fields
over Analysis units. The product answers with Ripscope's own query semantics
([ADR 0010](../adr/0010-own-query-semantics.md), "Ripscope's own query semantics give
each name the context it sits in"). The engine also keeps, exactly, the pinned
reference cscope's sequential search, the search it runs on a database built without
`-q`, as the **oracle** (section 7). The evidence is:
- the query probes in `tests/fixtures/queries/`: small synthetic projects, indexed and
  queried by the reference binary with and without `-q` (`examples/query-probes.rs`).
  Where the product's answer differs from the reference's, its intended answer is in
  `tests/fixtures/intended-answers.json`, with the rule that explains each changed row.
- `examples/support/query_semantics.rs`, a second implementation of both semantics
  written from the rules, with one switch per rule of the sequential search.
- the corpus runner's `engine` mode, which compares the oracle's field streams with the
  reference's over real projects.

Where this page and the binary disagree for the oracle, the binary wins, and this page is
corrected. Probe names are cited in brackets, for example [01-contexts].

## 1. What a query reads

- **Files in database order.** A query reads the indexed files in the order the
  reference's database holds them: the listed files sorted by byte value, then each level
  of the include closure, each level sorted the same way [04-names].
- **Lines and names.** Within a file it reads the stored lines in order, and within a
  line its names in order. It never reads the displayed text between names: the
  reference's regular-expression search can also match that text in rare cases (a
  continued string literal), and Ripscope does not reproduce that.
- **Which files.** A file gives rows only through the names it holds, since nothing
  carries from one file to the next (section 3), so each query says which files it needs
  (`Reads` in `src/engine.rs`): those that hold a name its pattern matches, in the roles
  its field reads:
  - field 0: every name but include operands and function and macro ends, a plain name
    tried against a regular expression only when it starts with a letter or underscore
    (section 2).
  - field 9: field 0's names marked assigned.
  - field 1: definitions.
  - field 2: the function and macro heads that calls sit in, not the calls.
  - field 3: calls.
  - field 8: include operands, without their opening delimiter.

  Field 7 reads the indexed paths alone, and no unit. The oracle's fields 0, 2, 3 and 9
  carry context from file to file (section 7), so they read every file. The index finds
  the files in its name postings: for an exact name its list, and for a regular
  expression or `-C` name pattern the lists of the names it matches among every distinct
  name the index holds. For field 8, it finds them in its include operands. A test
  (`a_query_needs_only_the_files_that_hold_what_it_reads`) checks, over the query probes,
  the captures and each name their files hold, that a query answers over only the files
  that hold what it reads as over every file, that in fields 0, 1, 8 and 9 each of those
  files gives a row, and that the postings select exactly those files, for any pattern.
- **Which blocks.** The index keeps each file's lines in blocks of about 16 KiB, each with
  the scope its first line starts in: the macro being defined and the function, all the
  context a walk carries from line to line (section 3 and
  [ADR 0011](../adr/0011-blocks-of-lines-with-their-scope.md)). A query reads its files'
  blocks in order and walks each from its start scope, so each name sits in the context a
  walk of the whole file gives it. For an exact name it reads only the blocks the name's
  postings list in each file: those the name occurs in, in any role, and those that start
  inside the body of a macro or function of that name, where field 2's calls can sit. Any
  other pattern reads every block of its files, and the oracle every block of every
  file. Field 8 reads no blocks: the index records each include's row, the text it shows
  and where reading resumes on its line, so the query reads only the includes of the
  operands it matches. In fields 0, 1, 2, 3 and 9 an exact name's postings also record the
  rows it answers in each file, with each line's place in its block and its context, so
  the query formats those lines and walks nothing. A test
  (`recorded_rows_answer_as_the_walk_does`) answers every name of the C fixtures in each
  of those fields from the rows and by walking, in blocks of two sizes, and field 8 for
  each operand and its last part, and requires the same answer. `a_query_needs_only_the_files_that_hold_what_it_reads` also answers each
  query over the units split into blocks of a
  line, of a few lines and of many, reading the blocks the postings list, and checks that
  it answers as over whole units.
- **In parts.** For the same reason a query reads its files in parts, each ending between
  files once its blocks hold 1 MiB of images, and lets each part go once it is answered,
  so it holds a few parts' blocks at a time. The thread that reads the parts walks a query's only part
  itself, its rows going straight to the answer's spools. Once there is a second part, up
  to three other threads (the cores but one) walk the parts while it reads the next, and
  hand each part back to be let go. A slow part holds back the answers after it, so the
  parts out at once, handed to those threads and not yet written, are one for each thread
  whatever its size, and more only while they hold at most two parts' worth of images for
  each thread. A walking thread keeps up to 1 MiB of each of a part's two spools in
  memory, and the rest in a temporary file, until the part's turn comes. Each part's rows
  at the file scope are spooled, in the parts' order, after those of the parts before, and
  its other rows in a second spool, which follows the first once the last part is answered
  (section 5). Nothing reaches the reader before then, since the count comes first. The
  oracle's fields that carry context read every file in one part. The same test checks
  that a query answered a file at a time gives the rows it gives over every file.

## 2. Patterns

Patterns are compiled as the query module compiles them (`src/query/pattern.rs`):
- **Symbol fields (0-3 and 9).** Trailing whitespace is ignored. A pattern containing any
  of `^ . [ { * + $ | (` is an extended regular expression that must match a whole name,
  after one leading `^` and one trailing `$` are dropped. Any other pattern must be a C
  identifier and matches names exactly. Anything else matches nothing.
- **`-C`** folds ASCII case on both sides, and makes every pattern a regular expression.
- **Files (7) and includes (8).** An extended regular expression that may match anywhere
  in the path or operand.

A regular expression never matches an empty name. A name without a mark (a plain
occurrence) is tried against a regular expression only when it starts with a letter or
underscore [04-names: `0[$].*`].

## 3. Context

Each name sits in a context, worked out within its file:
- the macro being defined, if any: from its definition to its end, or to the end of its
  line when it has no continuation.
- otherwise the function being defined, if any: from its head to the next function end,
  where a later head replaces an earlier one.
- otherwise the file scope, shown as `<global>`.

A head sits in the context it opens, so a function definition's own name takes its own
name. An end sits outside the context it closes. Every name updates the context, whether
or not a row matched earlier on its line [05-semantics: `0page`]. Nothing carries from
one file to the next: a file that ends inside a function or a macro leaves the next file
at file scope [05-semantics: `3first`].

## 4. The fields

- **0, every occurrence.** Every name except include operands, macro ends and function
  ends is matched. A line gives at most one row, for its first matching name, with that
  name's context. A macro definition's own row takes the function it sits in, with or
  without `-C` [05-semantics: `-C 0INNER`].
- **9, assignments.** As field 0, for names the analyzer marked as assigned (the name is
  followed by `=` but not `==`, by a compound assignment, or by `<<=` or `>>=`): a line
  gives a row for its first matching assigned name [02-lines].
- **1, definitions.** Names of macros, functions, globals, members, typedefs and struct,
  union, enum and class tags. The context column shows the pattern as the user gave it,
  after the leading `^` and trailing `$` are dropped (without `-C`) [02-lines: `1^[ab]$`].
  A row ends the reading of its line.
- **8, includers.** Include operands without their opening delimiter. The context is the
  file scope, also for an include inside a function (cscope's `-q` database shows the
  function). A row ends the reading of its line.
- **3, callers.** Each matching call, attributed to the macro it sits in, else to the one
  function it sits in. A call outside both gives no row. Where alternative heads under
  `#if` share one body, only the last is its caller [05-semantics: `3after`].
- **2, callees.** Each call whose macro, else function, matches, with the called name as
  its context, in reading order. A call in a macro defined inside a body belongs to the
  macro [05-semantics: `2.*`].
- **7, files.** Each indexed path that matches, as `path <unknown> 1 <unknown>`.

So `2.*` and `3.*` give the same calls, and a query over several files gives what it
gives over each file alone.

## 5. Row order and text

- **Order.** Rows at the file scope come first, then the rest, each group in reading
  order, whether the files are read whole or in parts. Only fields 0, 8 and 9 produce
  file-scope rows.
- **Text.** A row shows its line's displayed text. Where two names on a line touch, with
  no text between them, the reference's display of the line stops after the first: rows
  for names up to there show the line up to there.

## 6. Where the reference fails

- **A match after two touching names** makes the reference stop with "cannot get source
  line from database". Ripscope answers, showing the whole line.
- **The reference's 8 KiB read blocks** break its reading at block ends, depending only
  on byte offsets in its database file:
  - in fields 2 and 3, a line across a block end makes it resume at the wrong place,
    skipping or repeating calls and leaking callers.
  - in field 9, the assignment test reads past a block end: it can take `==` for an
    assignment or miss `&=`, and it moves the reading to the next block, skipping a macro
    end there, so that macro's context leaks into later rows and files.

  Ripscope reads every name once. On all 40 corpus projects, the oracle's field 2 and 3
  rows equal those of the reference's own algorithm walked over the reference's own
  database without the block reading, so every remaining difference in those fields is
  this defect. The corpus runner checks this on each run and classifies field 9's
  differences by block position.

## 7. The oracle: the reference's sequential search

The hidden developer commands `--units-query` and `--store-query` answer with the oracle
unless `--own-semantics` is given, so the compass compares like with like. It differs
from the own semantics by five rules, the switches of `examples/support/query_semantics.rs`:

- **Consumption (fields 0 and 9).** A row ends the reading of its line up to where the
  display stops: later names there are neither matched nor allowed to change the context.
  So a match on a type in a function head skips the head, and a function that opens and
  closes on one line leaves its context in place [01-contexts: `0.*` gives
  `b.c f 4 int later = 2;`, 02-lines]. After the display's stop, reading resumes, and the
  line can give another row. In field 9 a name that is not assigned is passed over
  without a row, so it consumes nothing.
- **Carry (fields 0, 2, 3 and 9).** The macro context carries over into the next file
  when a file ends inside a macro definition [01-contexts: `c.c LAST 1`]. The function
  context starts afresh in each file. In field 3 the macro and the list of open functions
  carry over too [03-calls: `3target`, `3second_call`]. In field 2, a list that runs to
  the end of a file hides that file's start from the search, so later rows keep the
  earlier path [03-calls: `2.*`].
- **Heads (field 3).** A call is attributed to each distinct function opened since the
  last function end, at most nine, in the order opened.
- **Own row under `-C` (fields 0 and 9).** A macro definition's own name takes its own
  name as context under `-C` [01-contexts: `-C 0BUMP`].
- **Listing (field 2).** For each matching function or macro definition, every call that
  follows, up to the function's end or the file's end. Once any macro definition has been
  read before a match, a macro end also stops the list. A macro defined inside the listed
  body is passed over, calls and all. Function heads inside the body do not stop the list.

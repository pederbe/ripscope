# The index keeps lines in blocks with their scope, and records answer rows

A rare name in a multi-megabyte generated header would make a query read and walk that
whole file to answer one row. Sessions repeat exact-name queries more than any other,
so their cost decides how fast navigation feels.

## Decision

Each file's analysis unit is stored as blocks of its lines, about 16 KiB each. Each
block records the scope its first line starts in under
[Ripscope's own query semantics](0010-own-query-semantics.md): the macro being defined
and the function, if any.

- Each name's postings list, for each unit that holds the name, the blocks it occurs
  in. For field 2, the blocks after a function's or macro's head that start inside its
  body are listed under the head's name too.
- The postings also record the rows an exact-name query in fields 0, 1, 2, 3 and 9
  answers: each row's line, its block, its place in the block and its context. Such a
  query reads only those blocks and formats those lines, without walking them.
- Each stored include records its row, so field 8 reads no blocks. Each distinct
  include operand is stored once.
- Regular expressions, `-C`, the oracle, `--verify` and an update that drops units read
  every block of a unit, in order.
- The block size is a parameter of the format, so other block sizes need no new format.

Name postings use an ordinary `STRICT` table with a separate primary-key index. In a
`WITHOUT ROWID` table, large posting lists can sit in the interior nodes of the name
index, and SQLite reads an overflowing record before comparing its key, which slows
unrelated lookups and updates.

## Consequences

- The index is larger, and `--verify`, which derives the rows again, is slower.
  Ripscope ranks query latency above index size and index build time.
- The recorded rows repeat what walking the blocks gives, so a change to the walk must
  change the rows with it. A test answers every name of the C fixtures in each recorded
  field both ways and requires the same answer. A query whose rows cannot be read
  walks the blocks instead.
- A block is checked whole when it is read, and `--verify` also checks the block lists
  and recorded scopes against the blocks.
- Changing the query semantics, or adding to the scope a block records, needs a new
  index format.

## Considered options

- **Smaller blocks of about 3 KiB.** They cut query work further but add SQLite page
  overhead.
- **SQLite's `quick_check` in place of the full integrity check.** It skips the
  uniqueness and index-to-table checks that `--verify` relies on.

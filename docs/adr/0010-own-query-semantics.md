# Ripscope's own query semantics give each name the context it sits in

cscope answers fields 0 to 3 and 8 in one of two ways, depending on whether its
database was built with `-q`, and each is wrong in its own way. Without `-q`, a search
reads the database in order and a matching row consumes the rest of its line. A match
on a type in a function head skips the head, so the body's rows show the previous
context, and a head still open at the end of one file passes its context to the next.
With `-q`, an inverted index records each name's context, but its prefix seek misses
rows. Vim's `:cs find i ^<cfile>$`, for example, finds no includers at all.

## Decision

Ripscope gives one answer for every index, by its own query semantics.

- **Sequential matching.** Files are read in database order, lines in order and names
  in order, so nothing is missed. File-scope rows come out first, then the rest, as in
  both cscope searches.
- **Per-occurrence context.** Each name's context is the macro it sits in, else the
  function it sits in (the latest head since the last function end), else the file
  scope. It is worked out within its own file, whether or not an earlier name on its
  line matched. Nothing carries from one file to the next.

Per field:

- **0 and 9.** At most one row per line, for its first matching name, or for field 9
  its first matching assigned name, with that name's context. A macro definition's own
  row takes the function it sits in, with or without `-C`.
- **3.** Each matching call, attributed to the macro it sits in, else to the function
  it sits in. A call outside both gives no row.
- **2.** Each call whose macro, else function, matches the pattern, with the called name
  as context, in reading order.
- **1, 7 and 8.** As cscope's sequential search. Field 8 keeps `<global>` as its
  context, and field 1 gives one row per line.

The engine keeps an exact implementation of cscope's sequential search, the oracle.
The query probes test it against captured cscope answers, so the compass can attribute
every difference to one of the rules above, a Correction or a known cscope defect. The
hidden `--units-query` and `--store-query` commands answer with the oracle unless
`--own-semantics` is given.

## Consequences

- Answers differ from cscope's by design. The line protocol stays the same, and only
  which rows come out and their contexts change.
- Where alternative function heads under `#if` share one body, such as `umain` and
  `wmain`, a call has only the last head as its caller, and field 2 for an earlier head
  answers nothing. Crediting the call to every open head, as cscope does, would also
  credit it to wrapper macros misread as heads.
- The index stores each block's starting scope under these semantics
  ([ADR 0011](0011-blocks-of-lines-with-their-scope.md)), so changing them needs a new
  index format.

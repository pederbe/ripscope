# Assignment search includes subscript stores, increments and decrements

cscope's assignment search (field 9) finds a name directly followed by an assignment
operator. It misses array initialisers and stores through a subscript, where the
subscript sits between the name and the operator, and counters changed with `++` or
`--`.

## Decision

Field 9 also returns:

- Written array initialisers and direct `name[index]` stores, under the written base
  name, including a pointer base.
- Direct named prefix and postfix `++` and `--`, allowing bounded redundant
  parentheses. `pointer++` and `*pointer++` change `pointer`, and in `slots[counter++]`
  the assigned name is `counter`.

Member, indirect, call-result and arbitrarily parenthesised subscript targets are not
recognised, and neither are `++*pointer`, `(*pointer)++` or member and subscript
increments.

These are written sites ([ADR 0003](0003-read-source-as-written.md)), so a macro
operand or `sizeof(counter++)` counts although it may never run. Each row follows
[Ripscope's own query semantics](0010-own-query-semantics.md). The first matching
assigned name on a line supplies the row's context, and an earlier read on that line
neither hides a later write nor lends it a context. Each part is a switchable
Correction ([ADR 0002](0002-measured-compatibility.md)).

## Consequences

- `pointer[index] = value` is reported as an assignment to `pointer`, though it changes
  the storage `pointer` reaches. Telling arrays from pointers would need symbol
  resolution.
- Displayed text and the line protocol are unchanged.

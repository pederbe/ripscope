# Ripscope's own scanner reads every file as cscope does

To give cscope's answers, Ripscope reads C the way cscope's scanner does. That scanner
never refuses a file. Code it cannot follow costs accuracy where that code is, and the
rest of the file is still indexed.

## Decision

Ripscope has one scanner, and it is Ripscope's own code. With every Correction off, it
reads C as the [reference cscope](0001-pinned-reference-cscope.md) does, measured file
by file against cscope's own database. Corrections plug in at named decision points
([ADR 0002](0002-measured-compatibility.md)).

The scanner is written from observed behaviour, not translated from cscope's source.
Its behaviour description is in Ripscope's own words and backed by probe fixtures of
the reference's output. Its rules, names, comments and module structure are its own.
Keyword and delimiter tables and mark bytes are compatibility facts, recorded as
observed. Where cscope's result depends on which of several look-ahead patterns
matches longest, a small layer evaluates each group by longest match with rule
priority.

The scanner never refuses a file. Linear-time caps bound its work without dropping a
file. Where cscope hangs or its behaviour is undefined, as with NUL bytes, and where it
truncates long names, Ripscope defines its own behaviour. These cases are listed in
[differences from cscope](../reference/differences-from-cscope.md).

## Consequences

- With Corrections off, a difference from cscope is a scanner bug.
- Every readable file contributes navigation facts, so Ripscope sometimes gives a
  wrong answer where a stricter reader would give none. Corrections improve those
  readings where the compass measures them.
- The scanner is covered by Ripscope's licence alone.

## Considered options

- **Port cscope's flex scanner.** The port would be a derivative of cscope's code under
  its licence, and would need an emulator for flex's start conditions, `yyless`,
  `unput` and jumps between rules.
- **A grammar-based parser such as tree-sitter-c.** Many real files can be parsed only
  by evaluating combinations of preprocessor branches, which grows combinatorially.
- **Refuse a file the scanner cannot read exactly.** The file would lose every
  definition, call and include, and the line protocol cannot show that a result is
  missing.

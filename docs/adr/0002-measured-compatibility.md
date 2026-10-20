# Compatibility with cscope is measured, and its misreadings are corrected

Editors parse cscope's output, so its format has to stay exact. cscope also misreads
some C, and reproducing every misreading would keep its false calls and lost
definitions.

## Decision

Ripscope keeps cscope's line protocol exactly: row format, row order, prompts and
messages. Its answers are compared with the
[reference cscope](0001-pinned-reference-cscope.md) over real C projects, for every
stored query field, file by file and in order. This comparison is called the compass.

Where cscope reads a construct wrongly, Ripscope reads it differently through a
Correction. Each Correction looks at most a bounded distance ahead, so reading stays
linear in the size of a file. Each has a probe fixture that shows both readings, and
each can be switched off, so the compass can measure what it gains and loses.

A Correction is judged by its measured effect across the projects and by sampled
examples, not difference by difference. A clear fix is enabled once the measurement
shows it loses no genuine definition. A Correction that trades one correct reading for
another, or changes displayed text or the protocol, is a product decision rather than
a measured fix.

## Consequences

- Every difference from cscope has a cause the compass can name: a Correction,
  [Ripscope's own query semantics](0010-own-query-semantics.md) or a known cscope
  defect.
- Without macro expansion some written shapes stay ambiguous. Where a Correction would
  remove a genuine definition along with false ones, it stays off.
  [How Ripscope reads C](../architecture/reading-c.md#12-corrections) lists these
  shapes and the examples that keep them.
- Some projects are held out from designing Corrections and only measured, so a
  Correction is not tuned to the projects that judge it.
- The [quality method](../architecture/quality.md) describes the comparison views and
  the independent accuracy checks.

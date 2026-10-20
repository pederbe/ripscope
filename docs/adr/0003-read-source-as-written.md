# Each file is read as written, without a compiler or preprocessor

cscope indexes the source text that exists, every conditional branch included. A
compiler front end would need flags, generated headers and a working build for each
file, and would see only the branches one configuration selects.

## Decision

Ripscope reads each file's own bytes as written. It does not expand macros, evaluate
`#if` or read the headers a file includes. A file's analysis unit depends only on its
bytes, its name's suffix and the analyzer, so files that match in all three share one
unit.

Stored facts are occurrences of written names, kept as their original bytes and keyed
by where they occur. Ripscope does not resolve a name to its declaration or a call to
its runtime target.

## Consequences

- Users need no compiler, flags, generated files or successful build, and inactive
  branches stay navigable.
- A name that exists only after macro expansion is not a definition. Corrections
  recognise a few written macro shapes directly, such as a function head whose name is
  wrapped in a one-argument macro
  ([reading rules](../architecture/reading-c.md#12-corrections)).
- An edit, a new `#include` line or a new file means analysing only the files it
  changed or added. Include resolution only asks the file system which candidate
  headers exist, as cscope does.

## Considered options

- **libclang or another compiler front end.** It needs a complete configuration for
  every translation unit and hides inactive branches.
- **Reading included headers to classify macros,** for example to tell a return-type
  wrapper such as `API(int) wrapped(void) {` from a function head. Every unit would
  depend on the headers it might include, so one new `#include` line or file would
  mean analysing the whole selection again, for the sake of a few macro shapes.

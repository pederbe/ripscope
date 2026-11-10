# How Ripscope works

Ripscope answers quickly and stays current because of three choices: it reads C as
it is written, it keeps what it found in one indexed file, and it replaces that
file's contents in complete steps. Each choice shapes what your searches find and
how fresh the answers are.

## It reads source as written

Ripscope's scanner reads each C file on its own, without a compiler or
preprocessor. It records where names occur, where they are declared and defined,
which functions call which, and where variables are assigned.

That is why Ripscope needs no build setup and why it works on code that does not
compile. It also reads every `#if` branch, so code for other platforms and
configurations is searchable.

The tradeoff is that Ripscope recognises names by their shape. It does not resolve
types, expand arbitrary macros or work out which calls actually run. Where one
construct is unclear, only that spot loses accuracy, and the rest of the file is
still indexed. [Limitations](../reference/limitations.md) lists what this means in
practice, and [How Ripscope reads C](../architecture/reading-c.md) gives every rule
the scanner follows.

## It answers from an index

The index is one SQLite file. Symbol, definition, caller, callee, file, include and
assignment searches read it directly, which is what makes them fast. On Linux, with 3,579 files of the Linux kernel, symbol, definition, callee and
caller lookups take about a tenth of a millisecond, and repeated file, include and
assignment searches between half a millisecond and about two. The [benchmarks](../reference/benchmarks.md) have every
number. These searches keep working even when the source files have moved or been
deleted.

Text and regular-expression searches read the current source files instead, as
cscope does, so they always show what is on disk. [Queries](../guide/queries.md#stored-and-live-fields)
says which search reads what.

## It updates in complete steps

An update looks at each file's size, times and identity, and hashes files that
were modified very recently. Only files whose bytes changed are read again, so an
update after a small edit is quick even in a large project.

Each update is one transaction. A search reads one complete revision of the index,
never a half-written one, and a search that runs during an update answers from the
revision before it. If an update is interrupted, the previous revision stays.

In an editor, Ripscope watches the files the index was built from. The first
search after a save updates the index, and searches between saves answer at once.
[Sessions](../guide/sessions.md#seeing-updates) describes the watch.

`--verify` reads every source file again and checks the whole index. See
[Check and repair an index](../guide/recovery.md).
[Index storage and lifecycle](../architecture/storage.md) describes the index file
and its updates in full.

## It is measured against cscope

Ripscope's starting point is what cscope reports. The project captures the output
of one fixed cscope release on a set of test sources and compares Ripscope with it.

Ripscope reads C the way cscope does, except for specific constructs that cscope
gets wrong. Each such change is called a Correction. It changes the reading of
that one construct, and the project keeps the source, cscope's answer and
Ripscope's answer for it side by side. Other misreadings of cscope's remain.
[Differences from cscope](../reference/differences-from-cscope.md) shows examples.

Separate expectations, written from the C source alone, check that the answers are
right and not only that they agree with cscope. The [benchmarks](../reference/benchmarks.md)
apply the same idea to real projects. Every row where the two tools differ there
was checked against the source.

## It is safe Rust

Ripscope is written in Rust with `unsafe` code forbidden, exact dependency pins and
a locked build. The `ripscope` executable never uses the network.

## More detail

Ripscope is designed and maintained by [Peder Bergan](https://github.com/pederbe).
The [architecture overview](../architecture/overview.md) maps the code, and the
[decision records](https://github.com/pederbe/ripscope/tree/main/docs/adr) give the
reasoning behind each design choice. [Contributing](contributing.md) explains how
to build and test a change.

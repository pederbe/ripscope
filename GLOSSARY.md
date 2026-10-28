# Ripscope

Ripscope answers cscope-style navigation queries over C source code from a persistent index. This glossary fixes the words used for its sources, analysis, index lifecycle, results and compatibility work.

## Sources and selection

**Selection rule**:
The persisted description of which files an index covers: explicit source files, a file list, or automatic discovery (optionally recursive), plus include directories.
_Avoid_: recipe, selection descriptor, selection (on its own)

**Indexed file**:
One entry of an index revision's files: a path and the analysis unit of its bytes. A path a file list names twice is two indexed files, as in cscope. Several indexed files can share one analysis unit when their bytes and analysis match.
_Avoid_: member, seed (for all indexed files)

**Include closure**:
The indexed files reached by following literal `#include` directives from the selected files through the include directories.

**Captured source**:
The bytes read for one selected source file during an analysis attempt. Its content fingerprint and navigation facts refer to those same bytes, which may differ from the file's later contents.

## Analysis

**Analyzer**:
The component that turns a captured source into navigation facts: the scanner, with the Corrections it reads with. It reads every file it is given: code it cannot read well costs accuracy where that code is, never the whole file. Its identity (the scanner's version, its Corrections and the unit image's version) is stored with the index, so a change to it analyses every file again.
_Avoid_: producer, parser, backend

**Analysis unit**:
The navigation facts and displayed text one analyzer produced for one captured source, from its bytes alone: the headers it includes do not change its reading (ADR 0003).
_Avoid_: unit (on its own)

**Navigation fact**:
A statement derived from source text that a query can return: a definition, a call, an assignment, an include or a symbol occurrence.
_Avoid_: evidence (for published facts)

**Symbol occurrence**:
A matching use of a name in source text. An occurrence does not by itself establish that the name refers to a particular declaration or definition.

**Assigned occurrence**:
A symbol occurrence at a written initializer, assignment, direct subscript store or direct named increment or decrement. A subscript store is attributed to its written base name even when that base is a pointer. An assigned occurrence is a written site, whether or not it runs.

## Index lifecycle

**Index**:
One SQLite file named with `-f` (`ripscope.db` when none is named), with SQLite's `-wal` and `-shm` files beside it when present, including between queries. They are moved, copied or deleted together (ADR 0005).
_Avoid_: database, cscope.out

**Index revision**:
A committed set of indexed file membership and navigation facts. One query uses one revision even if a newer revision becomes available while it runs.

**Publication**:
Making a new index revision the one that queries read. It either completes, or the previous revision stays published. An index that has published nothing yet, while it is being built afresh, is no index to queries.

**Index identity**:
Random bytes an index is given when it is created, and again by each update that analyses every file again (`-u`, or a new analyzer). A session keeps the blocks it decodes by the identity and each unit's key, so it never takes blocks of an index made afresh at the same path, or split by another size, for those it reads.

**Block**:
A run of consecutive lines of an analysis unit, about 16 KiB of it, stored as an analysis unit of its own with the scope its first line starts in: the macro being defined and the function (ADR 0011). An index keeps each unit as its blocks, and a unit without lines as one empty block.

**Name postings**:
For each distinct name in an index (include operands are kept apart), the units that hold it, the roles it has in each (the marks of its occurrences, and whether one is assigned) and the blocks a query for it reads there: those it occurs in, and those that start inside its macro's or function's body. They are kept with the units. A query for an exact name reads only the blocks its postings list, and a regular expression every block of the files listed by the names it matches.

**Refresh**:
Re-applying the selection rule and re-analyzing the files whose bytes or analyzer changed, then publishing the result. An updating Session refreshes before a query once its watch reports a change, and the `r` command requests a refresh explicitly.
_Avoid_: reindex, rebuild (for a refresh)

**Watch**:
What an updating Session keeps on the files, directories and include candidates its index was made from, to tell before each query whether a Refresh could find anything changed (ADR 0008). It covers what a Refresh reads before the Refresh reads it, and leaves the next query to refresh whenever it cannot be sure.
_Avoid_: watcher (for the whole mechanism), polling

**Index build**:
Creating an index, or changing its selection rule, with `-b`.
_Avoid_: build (on its own), which in software means building Ripscope itself

**Index user**:
Any process that currently has an index open, including one-shot queries, sessions and editor backends.
_Avoid_: owner

## Queries and results

**Query field**:
One of cscope's numbered search kinds, 0 to 9. Field 5 changes text in cscope's full-screen interface. In the line protocol it is a text search, as in cscope.

**Stored query**:
A query answered from the index revision without reading source files: fields 0, 1, 2, 3, 7, 8 and 9. Each says what it reads: field 7 the indexed paths alone, the others the files that hold a name it asks for, or, under the sequential semantics, every file for fields 0, 2, 3 and 9, whose context carries from file to file.

**Live text query**:
A text or regular-expression search (fields 4, 5 and 6) that reads the current contents of the indexed files.

**Session**:
A long-running line-oriented connection (`-l`) that answers many queries, the form editors use.
_Avoid_: persistent mode, line mode

**One-shot query**:
A single query run as its own process with `-L`.

**Result row**:
One answer: file path, context, line number and displayed text.

**Result sink**:
The component that collects result rows in their output order, counts those the reader can receive, and places warnings as that reader expects.

**Context**:
The enclosing function or macro definition, or the file scope, that a result row belongs to.
_Avoid_: owner, scope (in result output)

**Query semantics**:
How the stored query fields 0, 2, 3 and 9 work out each name's context. Ripscope's **own semantics** read every name in order and give each the context it sits in within its file. cscope's **sequential semantics** let a row consume the rest of its line (**consumption**), carry context from one file to the next (**carry**), give a call every function head opened since the last function end as its caller, list a matching function's calls up to its end in field 2, and under `-C` give a macro definition's own row the macro's name.
_Avoid_: mode (for this)

**Oracle**:
The engine's exact implementation of cscope's sequential semantics, kept to compare with the reference cscope and to attribute each difference from it.

**Displayed text**:
The compacted source line shown in a result row, prepared when its file is analysed, so stored queries need no source files. A text search's row shows the found line itself, not compacted.
_Avoid_: record, preparation (in user-facing text)

**Line protocol**:
cscope's space-separated output and prompt format, kept unchanged for existing editor integrations.
_Avoid_: legacy transport, cscope format

**JSON interface**:
The opt-in `--json` output and session framing that carries any path, including spaces and Unicode.
_Avoid_: native interface

## Editors

**Editor integration**:
The plugin or package an editor searches through, which Ripscope ships with each release: the Vim plugin over Vim's built-in cscope support, the Neovim plugin, and the Emacs package, which is an xref backend. The Neovim and Emacs ones keep one session per index over the JSON interface (ADR 0014, ADR 0015).
_Avoid_: adapter (for the Emacs package)

**Project**:
The directory an editor search covers when no setting names one: the nearest directory upward from the file holding `cscope.files`, else the nearest Git repository, else the editor's current directory when the file is inside it and it is neither the home directory nor a file system root, else the nearest existing directory holding the file. A Git repository at the home directory or a file system root is not taken as one. Its index lives in the user's cache directory, named after the project's path with links resolved, and every editor uses the same one (ADR 0014).
_Avoid_: workspace, root (on its own)

## Compatibility

**Navigation accuracy**:
Whether expected navigation facts are found and returned facts are correct against
independently labelled written source within a declared Assessed scope. Agreement
with the Reference cscope or successful indexing does not establish Navigation accuracy.

**Assessed scope**:
The source locations and query kinds whose expected navigation facts are
independently labelled. Indexed files and returned rows outside it remain unassessed.

**Reference cscope**:
The pinned, identity-checked cscope executable whose results Ripscope's are compared against.

**Correction**:
A rule by which Ripscope deliberately reads a C construct differently from the reference cscope because cscope's reading is wrong. Each has a demonstrating example and is judged by its measured effect. A clear fix is enabled once it is measured on the corpus, its kept readings are checked and no real definition is lost (ADR 0002).
_Avoid_: approved difference, improvement, deviation

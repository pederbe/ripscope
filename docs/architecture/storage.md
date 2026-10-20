# Index storage

An index is one SQLite file ([ADR 0005](../adr/0005-one-sqlite-file-per-index.md)), at
the path `-f` names or `ripscope.db`. SQLite keeps its `-wal` and `-shm` files beside it.
They can remain between queries. Move, copy or delete the complete Index together.

## What it holds

| Table | Content |
| --- | --- |
| `meta` | The revision number, the revision's indexed paths in database order as one value (each path's length, then its bytes), which field 7, text searches and a Session's start read at once, the index's identity (16 random bytes written when it was created, and again by each update that analyses every file again), the block size, the selection rule (the directory the index is built in, sources, include directories, `-k`, and `INCLUDEDIRS`/`INCDIR` as they were at the index build), and the analyzer's identity (scanner version, enabled Corrections, image version). After a clean Refresh, a record of what it observed (see below). |
| `files` | The indexed files in cscope's database order: the path as displayed, its Analysis unit, size, modification and status-change times, file identity, SHA-256 of the bytes analysed, and when they were last read. |
| `units` | Analysis units, keyed by the bytes' hash, suffix mode and analyzer identity, so identical files share one unit: a file is read without the headers it includes (ADR 0003). Each unit records how many blocks hold its lines. |
| `blocks` | Each unit's lines in blocks ([ADR 0011](../adr/0011-blocks-of-lines-with-their-scope.md)): runs of consecutive lines, each an Analysis unit image of its own with the macro and the function its first line starts inside. Block `n` of unit `u` has the id `u << 20 \| n`, so a unit's blocks are one range of ids, in order, deleted with the unit. |
| `operands` | Each distinct include operand the units hold, without its delimiters, removed once no include uses it. |
| `includes` | Each Analysis unit's include operands, as `operands` ids, with where each sits (block, line, the line's record in the block and the name's index) and its includer row (the text it shows and the name its line's reading resumes at), so an update follows `#include`s without decoding the units, and field 8 matches each distinct operand once and answers from the includes of those it matches, read from one index, without reading blocks. |
| `names` | The name postings: for each distinct name, the units that hold it, in unit order, with the roles it has in each (the marks of its occurrences, and whether one is assigned) and the blocks a query for it reads there (those it occurs in, and those that start inside its macro's or function's body), so a query finds the files that hold a name it asks for, or one its pattern matches, without reading every unit, and for a name it asks for, the blocks. For each unit they also record the rows an exact-name query in fields 0, 1, 2, 3 and 9 answers: each row's line, block, record and name, and its context as a reference into the block ([ADR 0011](../adr/0011-blocks-of-lines-with-their-scope.md)). |

The application id (`RIPS`) and schema version (`PRAGMA user_version`) identify the
file. No source copies are kept: stored queries answer from the displayed lines in the
units, and text searches read the live files.

The `names` table has a separate primary-key index. Its posting lists stay in table
leaves: putting large lists in the intermediate nodes of a `WITHOUT ROWID` table makes
unrelated name lookups read those lists too. The separate index repeats the names and
takes more space, in exchange for keeping that cost out of lookups and updates. It also
adds index-to-table consistency checks to SQLite's full integrity check.

## Opening

An index is created only where no file exists, in an empty file, or (by an index build)
over a Ripscope index of another schema version, first written beside the path and then
moved into place. Ripscope creates that file itself, before SQLite writes it, so a new
index gets the permissions the umask gives any new file (SQLite would make it 0644, less
the umask). On Unix an index that replaces another keeps that one's permission bits, and
its group and owner where the process may give them (a member of the group, or root). Only a
process that may write an index replaces it, and only once the old index's `-wal`, `-shm`
and any `-journal` are removed, since SQLite would replay them into the new one. On
Windows, where a file another process has open cannot be removed or replaced, the
replacement waits up to ten seconds for them. An index build or update that would make a
new index first checks that its selection rule finds a file, so one that finds nothing
leaves no file behind. Anything else (a cscope database, any other file) is refused
untouched, judged from the file's first 100 bytes. Readers open it read-only in effect
(`query_only`), so they can still recover the write-ahead log after a writer crashed. An
index is not created or updated on storage where the write-ahead log is unsafe (network,
cluster and shared-folder file systems, WSL's view of Windows drives, FUSE mounts no local
disk backs, Windows shares). An index build that makes the index in a folder OneDrive
synchronises warns that OneDrive may lock or copy it.

A query closes its connection after reading one Index revision, leaving SQLite's
WAL companions for the next query instead of checkpointing and deleting them on every
close. On Windows an idle Session holds no SQLite connection or file handle, so its Index
can be moved or replaced. Writers retain SQLite's normal checkpoint and cleanup. If a
reader outlives a writer, the WAL can hold the newest committed revision even after
both connections close. Keep it and the shared-memory file with the main file. Removing
companions alone can discard committed changes.

## Updating

An update takes SQLite's write lock (`BEGIN IMMEDIATE`) for its whole run, waiting up to
60 seconds for another update (a query's update does not wait: it answers from the last
revision). It then:

1. chooses the selection rule after acquiring the lock: the saved rule for Refresh,
   or the invocation's for an index build. It applies the explicit files, the list
   (read again), or automatic discovery (`cscope.files` if it can be read, else the
   directory's sources). If the selected names and options hash as the last clean
   Refresh recorded, every file of the current revision is unchanged (as in step 2),
   and the file system gives every answer that Refresh's include resolution relied on
   (which candidate names exist as regular files spelled exactly), it would publish the
   same revision, and it stops here. The record names the revision and its file rows,
   and a Refresh that reports missing or unreadable files removes it.
2. for each file, level by level of the include closure: keeps its Analysis unit when size,
   times and identity match and the file was last read more than two seconds after its
   last modification. Otherwise it reads and hashes the file, keeps the Analysis unit if
   the bytes match, and scans it if not, storing its unit once. A file's unit depends on
   its bytes alone, so no other file is analysed again for it.
3. follows the discovered Include operands through the directory the index is
   built in (for quoted operands), the `-I` directories, the list's, `INCLUDEDIRS` and
   the system directory, in that order, as cscope does.
4. replaces the file list, deletes units no file uses with their blocks and name postings
   (found from each unit's blocks), bumps the revision and commits, or, if nothing
   differs, commits nothing.

It stores each new unit as blocks: lines go into a block until their records take the
index's block size, so a block ends with the line that reaches it, however long, and the
next line starts another, with the scope the lines before leave (a unit needing more
than 2^20 blocks is refused as too large). The block size is recorded when the index is
made: 16 KiB, or one the environment variable `RIPSCOPE_BLOCK_SIZE` gives (tests store a
block for each line). Every update of the index uses it, so that every unit is split by
one size. When it cannot be read, an update that must store a unit fails, until one that
analyses every file again (`-u`, or the first after the analyzer changes) records the
size a new index gets. Each update that analyses every file again also gives the index a
new identity, so that sessions stop using the blocks they keep, which another size may
have split.

It collects the name postings of the units it adds and drops, and writes them in batches
of up to 8 million: in each, each name's list is read, changed and written back once, in
name order, and an index build afresh writes its first batch without reading. When the
analyzer changes, every unit goes first, with the postings, as with `-u`. A posting list
that cannot be read makes a query for its name, or a pattern that matches it, read every
file, with a warning, and an update that changes it fail until the index is rebuilt with
`-bu`. A `names` table SQLite cannot read does the same for any pattern, and for an exact
name whose lookup meets the damage. [ADR 0011](../adr/0011-blocks-of-lines-with-their-scope.md)
records the tradeoffs behind the block size and verification.

A file that cannot be read keeps its last Analysis unit and is tried again next time. An
index build leaves it out. When none of the selected files exists, the update fails and
the index keeps what it had. When the analyzer's identity differs from the one stored,
every file is analysed again. `-u` drops every Analysis unit first, so each file is
analysed again.

## Reading

A query reads what it needs in one read transaction (`src/revision.rs`): field 7 and
text searches the paths alone, a query for an exact name the blocks its postings list,
formatting in fields 0 to 3 and 9 only the lines they record,
a regular expression or `-C` name pattern every block of the files the postings of the
names it matches list (one pass over the names, or for a case-sensitive regular
expression with literal prefixes over those ranges of names alone, decoding only the
lists of those it matches), and field 8 the recorded rows of the matching includes and
no blocks. The blocks of a unit several files share are read once. It never reads
the whole file list with its eight columns. It decodes the blocks a part at a time, a part
ending between files once it holds about 1 MiB of images, which up to three other threads
walk while it reads the next, and lets each part go once it is answered, so a query holds
a few parts' blocks whatever it reads: the one it reads, one for each walking thread, and
more only while they hold at most 2 MiB of images for each.
Its read transaction, and so the open index, lasts as long as the query takes, seconds for
a common name or a broad pattern on the kernel. On Windows an index build that replaces
the index waits up to ten seconds for it. An index that holds no published revision yet
(one another process is making afresh, or whose first index build failed) is treated as a
missing one: a query fails. A session holds the index open between queries, so that a
query does not open it, read its schema and prepare its statements again. Each query
still reads the current revision in a transaction of its own, and fails, saying why, when
it cannot. On Unix an index another file replaced is noticed before each query by the path
naming another file, and opened afresh. On Windows a held file cannot be deleted, moved or
replaced, so there a session lets go of the index after each query and each update.
It keeps the blocks it decoded,
up to 256 MiB of images, by the index's identity (a random value written when the index is
created, and again by each update that analyses every file again), each unit's key and the
block's number, for the queries after. The first query of a session that reads a unit
whole checks its whole range of blocks, as a query does without a session, and a unit's
blocks go together, the least recently used unit's first.

A unit that cannot be read leaves its file out of the fields that read units, with a
warning: all of them when the stored analyzer names another unit image version (an
index another version made), else the damaged ones, found where a query reads them: a
damaged block leaves its file out of the queries that read that block, and only those.

## Checking

`--verify` runs `PRAGMA integrity_check` and `foreign_key_check` and checks that the block
size can be read, brings the index up to date reading every file, and reads every Analysis
unit once: its blocks must all be there and decode, and be the blocks that splitting their
lines again with the index's block size gives, start scopes included. It also checks that
no block belongs to no unit, that the path list holds the indexed files' paths in order,
that the include records are exactly the units' includes,
and that the name postings, block lists and answer rows included, are exactly
those of the units. Rebuilding the index with `-bu` is the repair.

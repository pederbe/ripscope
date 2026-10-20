# Checking and repairing an index

An index is one SQLite file (with SQLite's `-wal` and `-shm` files beside it while it is
in use). Every update is one SQLite transaction: it either completes, or the previous
revision stays as it was, even if the process is killed or the machine loses power.
There is no separate journal to recover.

## Checking

```text
ripscope --verify [-d] [-f INDEX]
```

`--verify` runs SQLite's full integrity and foreign-key checks, then (unless `-d`)
brings the index up to date reading and hashing every file, and finally decodes every
stored analysis and checks the name postings against them. It prints `index verified:
revision N, M files`, or lists what is wrong and exits with status 1. A query that meets
a damaged index says so in a warning starting `damaged index:`, and still answers from
what it can read. An update that needs a name's postings it cannot read, or the block size
the index splits a new analysis by, fails with such a message and keeps what the index
had, and so does a query without `-d`, which updates first.

## Repairing

```text
ripscope -bu [the options you built the index with]
```

`-u` builds the index *unconditionally*: every file is read and analysed again and nothing
the index held is kept, so a damaged analysis is replaced. When SQLite can no longer read
the index file at all, `-u` makes it afresh. Give the options you built the index with
(the sources, `-i`, `-R`, `-I`, `-k`), since a damaged index may not be able to tell which
they were.

## Older indexes

An index built by an earlier version of Ripscope, whose analyzer read C differently,
analyses every file again at its next update. Until then, `-d` answers from it as it was
built. Where this version cannot read a file's stored analysis at all, fields 0 to 3, 8 and
9 leave the file out and say how many they left out. Field 7 still lists it, and text
searches read it. An index with another schema version is read by neither queries nor
updates, which ask for an index build: `-b` makes it afresh.

# An index is one SQLite file in Ripscope's own format

Editors hand the backend one index path and never read the database themselves. The
index has to let one process update it while others query it, survive a crash, and
need nothing installed beyond the executable.

## Decision

The path given with `-f` (`ripscope.db` by default) is one SQLite file with Ripscope's
own schema. SQLite is compiled into the executable and runs in write-ahead-log mode
with `synchronous=FULL`. The file holds the selection rule, the indexed files, each
file's analysis unit and the name postings. It keeps no copies of the sources.

- Integrity rests on SQLite. `--verify` runs SQLite's integrity checks and decodes
  every analysis this version stored. `-bu` rebuilds everything and recovers a damaged index.
- An index is created only at an absent path, in an empty file or over a Ripscope
  index. A cscope database or any other file is refused and left untouched.
- The write-ahead log needs local file locks and shared memory. An index is not
  created or updated on network, cluster or shared-folder file systems, on WSL's view
  of Windows drives, on FUSE mounts other than a block device's, or on Windows shares.
  In a folder OneDrive synchronises, an index build that makes the index warns that
  OneDrive may lock or copy it.
- An index with another schema version is refused by queries and updates, and the
  next index build makes it afresh. Indexes are not migrated.

## Consequences

- Moving or copying an index means one file, with its `-wal` and `-shm` files when
  they exist.
- Readers write SQLite's `-shm` file beside the index, so an index in a directory the
  user cannot write cannot be queried.
- A schema change raises the schema version, and users rebuild their indexes.

## Considered options

- **cscope's `cscope.out` format.** No editor decodes the database, so byte
  compatibility would buy nothing while constraining storage, concurrency and
  recovery.
- **Publishing each update as a new database written beside the index and renamed
  over it.** On Windows a rename over an open SQLite file fails, and SQLite's
  transactions already publish a revision atomically. Only an index build that
  replaces a whole index works this way.
- **LMDB.** Its Rust bindings need `unsafe` code, which Ripscope forbids.
- **redb.** Only one process can open a redb database at a time, so sessions and index
  builds would need a broker process.

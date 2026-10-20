# A refresh re-applies the selection rule and trusts unchanged metadata

A refresh has to find added, removed, renamed and edited files without reading the
whole project each time. The reference cscope misses an edit that keeps a file's size
and modification time.

## Decision

The index stores how its files were selected, separately from the resulting list: a
file list, automatic discovery with or without recursion, or files named on the
command line, together with the include directories. Each refresh applies the rule
again, so a list or discovery picks up added, removed and renamed files. Files named
on the command line stay a fixed list.

A refresh compares each file's size, modification time, status-change time and file
identity with what it recorded. On Windows the creation time stands in for the
status-change time, and the identity is the volume serial number and file index
instead of the inode. A file whose metadata changed is read and hashed, and is
analysed again only when its bytes or the analyzer changed. A file modified within two
seconds of its last read is read and hashed even when its metadata matches, since its
timestamps cannot be trusted that close to a change. `--verify` reads and hashes every
file, and `-u` analyses every file again.

A file that changes while it is read is indexed as it was read. Its analysis and hash
describe the same bytes, so the next refresh sees any later change.

## Consequences

- Refreshing an unchanged project costs one metadata call per file.
- On Windows, an edit made in place that keeps the size and restores the modification
  time is noticed only by `--verify` or `-u`.
- Without `-d`, one-shot queries and sessions refresh when they start.
  `--auto-refresh` and `--no-auto-refresh` override `-d` either way. A session
  refreshes again when its watch reports a change ([ADR 0008](0008-sessions-refresh-on-watched-changes.md)).

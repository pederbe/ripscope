# Limitations

This reference describes Ripscope’s supported platforms and C reading. See the
[roadmap](../project/roadmap.md) for what comes next.

## Language and analysis

- **C only.** `extern "C"` blocks and `#ifdef __cplusplus` wrappers in C headers are
  read as C.
- **cscope's reading, with Corrections.** The analyzer reads C as cscope does, without
  preprocessing. Every `#if` branch is indexed as written, and names are recognised by
  their shape, not resolved by a compiler. Where cscope misreads common constructs,
  [Corrections](differences-from-cscope.md#corrections-reading-c-where-cscope-misreads-it)
  read them correctly. Other misreadings of cscope's remain.
- **Every readable file is indexed.** Code the analyzer cannot read well costs accuracy
  only where it is.
- **Includes.** Only literal `#include "..."` and `#include <...>` directives are
  followed. A computed include such as `#include MACRO` is not followed, as in cscope.

## Queries and output

- Contexts are worked out within each file from the names as written. Where
  alternative function heads under `#if` share one body, a call has only the last head
  as its caller. See [Differences from cscope](differences-from-cscope.md#query-results).
- Case-insensitive matching (`-C`) folds ASCII letters only.
- A query with `-d` on an index that another version of Ripscope made leaves out the files
  whose stored analysis it cannot read, and says how many, until the index is rebuilt with
  `-bu`. Only fields 0 to 3, 8 and 9 read stored analyses, so field 7 still lists those
  files and text searches read them, without a warning. Without `-d` a query analyses
  them again.
- Vim and xcscope split rows at spaces, so paths with spaces break their parsing, as
  with cscope. The [JSON interface](../guide/json.md) keeps such paths whole, and the
  Neovim plugin uses it.
- A query for an exact name reads the blocks of lines that hold it and, for a
  matching function or macro, the rest of its body. A common name can occur in
  many blocks. A regular expression or a `-C` name pattern in fields 0 to 3 and 9
  first checks every distinct indexed name, then reads the blocks containing
  matches, so a broad pattern can cost much more work than an uncommon exact name.

## Platforms and storage

- x86-64 Linux and x86-64 Windows. No macOS or ARM64 builds.
- The index must be on local storage. Network, cluster and shared-folder file systems,
  WSL's view of Windows drives, FUSE mounts other than a block device's (`fuseblk`, such
  as ntfs-3g) and Windows shares are refused. gocryptfs or mergerfs over a local disk
  are refused too. In the folders the OneDrive environment variables name, an index
  build that makes the index warns that OneDrive may lock or copy it. SharePoint and
  Teams libraries OneDrive synchronises outside those folders, other sync clients'
  folders, and on Linux those of sync clients that are not FUSE mounts look local and
  get no warning, though their client may lock or copy an index there too.
- On Windows an index's path cannot hold `:` after its drive (`-f file:i.db`), which
  would name a stream of another file. Such a path is refused.
- An index path that is a symbolic link stands for the file the link names, as SQLite
  reads it. Its log files, the check of where it lives and a rebuild of the index are
  that file's, and the link stays. A link to nothing is refused.
- Queries need write access to the index's directory, because a reader creates SQLite's
  shared-memory file (`INDEX-shm`) beside the index. An index on read-only storage, or
  in a directory you cannot write, cannot be queried, even with `-d`, unless SQLite's
  files are already there, and the message says so. Copy such an index somewhere
  writable and give it with `-f`.
- An index file you cannot write, in a directory you can, is answered from as it is. A
  query or session says so and skips its update, and an index build fails.
- On Unix, SQLite gives the files it makes the index file's permissions, so a reader
  who cannot write the index leaves them read-only. An update by a user who may write
  the index and owns them makes them writable again. If another user left them, queries
  answer from the index as it is, with a note naming the file, and an index build
  fails, until that user makes them writable. A group shares an index through a setgid
  directory or a default ACL (see [Sharing an
  index](../guide/building.md#sharing-an-index-between-users-unix)).
- SQLite's log files beside an index (`INDEX-wal`, `INDEX-shm`) must be plain files. A
  link, a file with another name, a directory or a FIFO there refuses the index, for
  queries as for index builds, since SQLite would write through it. Remove it to use the
  index. A rollback journal (`INDEX-journal`), which an index never has, is removed
  before the index is opened.
- Use an index from one machine only. SQLite's write-ahead log is unsafe across
  machines, and a query from another machine over NFS or a share is not refused.
- An index holds the text of the lines that hold symbols in the files it covers, so
  anyone who can read the index can read those lines.
- On Windows an update compares a file's size, modification and creation times and
  identity. An edit made in place that keeps the size and restores the modification time
  is missed. `--verify` or an index build with `-bu` reads every file and catches it.
  Editors that save by replacing the file change its identity and are noticed.

## Tools and integrations

- No interactive terminal interface. Use an editor integration or the command line.
- On Windows, Emacs command arguments must fit the active Windows ANSI code page, and
  unsupported characters are refused. Its native index build uses Ripscope's file
  discovery rules. xcscope's suffix and directory-exclusion settings apply only on
  Linux.
- Editor integrations are tested on specific versions, listed on each editor's page.

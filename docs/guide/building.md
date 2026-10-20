# Building an index

```text
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] [--] [SOURCE...]
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] -i LIST
```

`-b` builds the index, or updates one that exists, then stops. The index is one file,
`ripscope.db` unless `-f` names another. It is created only where no file exists, in an
empty file, or over an index Ripscope made: a cscope database or any other file at that
path is left untouched and the index build fails.

Every readable file contributes navigation facts. The analyzer reads C as the reference
cscope does, with [Corrections](../reference/differences-from-cscope.md) where cscope
misreads it; code it cannot read well costs accuracy only where it is.

## Choosing the files

Ripscope saves *how* you selected files (the **selection rule**), and every later update
applies it again, as cscope does at each index build. The first matching rule wins:

1. **A file list**, `-i LIST`. File names given beside it are not read, as in cscope.
2. **Source files** given as arguments. A name given twice is read once.
3. **Automatic discovery**, when neither is given:
   - `cscope.files` in the current directory, if it can be read;
   - otherwise the source files directly in the current directory, or in the whole
     tree with `-R`.

   Because it is applied again at every update, creating or deleting `cscope.files`
   changes what the next update indexes.

A named file (an argument or a list entry) that does not exist or is not a regular file is
reported, as cscope reports it (`ripscope: cannot find file X`), and the rest is indexed;
one that cannot be read is reported as `cannot read X`. When none of the selected files
can be indexed, the index build fails with `no source files found`: a new index is not
made, and an index that already existed keeps what it had. Unlike cscope, a symbolic link
you name is followed.

In every case, literal `#include` directives are followed (see
[Include directories](#include-directories)), so headers reached from the selected files
are indexed too.

### Directory discovery

Source files are regular files whose last suffix is one of cscope's: `.c .h .l .y .C
.G .H .L .bp .qc .qh .sd .cc .hh .tcc .cpp .cxx .hpp .hxx` (case-sensitive).
SCCS-style names such as `s.file.c` are skipped, as in cscope. Symbolic links found in
a directory are not followed, as in cscope. With `-R`, discovery also descends into subdirectories. A directory that
cannot be read is reported; its files keep what the index knew about them.

### File lists

A file list names one source per entry, relative to the directory you build the index in:

```text
src/main.c
src/util.c
"src/file with spaces.c"
-I include
-k
```

- Entries are separated by spaces, tabs or newlines. Double quotes allow spaces;
  inside quotes, `\"` and `\\` are the only escapes.
- A list may hold the options cscope reads there: `-I DIR` and `-k`, and `-q`, `-c`
  and `-p N`, which change nothing in Ripscope's answers. `-T` would cut symbols to
  eight characters, so a list naming it is refused.
- A name the list gives twice is indexed twice, as cscope does.
- The list is read again at every update, so editing it changes what is indexed.
  Reading a list from standard input (`-i -`) is not supported.

## Include directories

`#include "file.h"` is looked up first relative to the directory the index is built in;
then, for both forms, in each `-I` directory in the order given, the list's `-I`
directories, the directories in the `INCLUDEDIRS` environment variable, and last the
system directory (`/usr/include`, or `INCDIR`) unless `-k` is given, as cscope does.
Headers that cannot be found are skipped. A computed include such as `#include MACRO` is
not followed.

## Updating

Run the same `-b` command again, or query with `-L` or use `r` in a
[session](sessions.md): each brings the index up to date. An update:

- applies the saved selection rule again;
- re-reads a file when its size, modification time or identity changed, or when it was
  modified too close to its last read to trust the timestamp, and analyses it again
  only if its bytes changed;
- publishes a new revision in one transaction when anything changed (re-reading a file
  whose bytes did not change publishes nothing), so a query or session always reads a
  complete revision, and sessions see the new one at their next query.

`-u` updates *unconditionally*, as cscope says: every file is read and analysed again,
and nothing the index held is kept. It is also how a damaged index is repaired (see
[Checking and repairing](recovery.md)).

A `-b` with different sources, `-i`, `-R` or `-I` replaces the saved selection rule. A
file that cannot be read keeps its last facts in an update and is tried again next
time; an index build (`-b`) leaves it out.

## Where the index may live

SQLite's write-ahead log, which lets queries read while an update runs, is unsafe on
network file systems. Ripscope therefore does not create or update an index on NFS, SMB
and other network or cluster file systems, on WSL's view of Windows drives, on FUSE
mounts other than a block device's (sshfs, cloud-sync clients, and also gocryptfs or
mergerfs over a local disk) or on Windows shares. Keep the index in a local folder; the
sources may be anywhere.

An index may live in a folder OneDrive synchronises, such as a Documents or Desktop
folder OneDrive backs. OneDrive may lock the index or copy it while it is written, so
the index build that makes the index there warns once, and `-f` can keep it in a folder
OneDrive leaves alone. Ripscope knows OneDrive's folders by the `OneDrive`,
`OneDriveCommercial` and `OneDriveConsumer` environment variables. If the index is ever
damaged there, `-bu` builds it again.

Use an index from one machine only. SQLite's write-ahead log needs every process that
uses the index to run on the same machine, so a query from another machine, over NFS or
a share, is unsafe, although it is not refused.

## Sharing an index between users (Unix)

A new index gets the permissions the umask gives any new file, as cscope's database does.
An index rebuilt over another keeps that index's permission bits, and its group when the
user rebuilding it belongs to that group (root keeps its owner too). Only a user who may
write an index can rebuild it.

Every user of an index must be able to write it, and the files SQLite keeps beside it,
which it gives the index's permissions. An index also holds the text of the lines that
hold symbols in the files it covers, so anyone who can read it can read those lines,
even of files they cannot read. For a group to share an index privately, keep it in a
directory of the group with the setgid bit, so that the index and SQLite's files belong
to the group, closed to other users, and build it with umask 007, so that the group may
write them:

```sh
chgrp team DIR && chmod 2770 DIR
(umask 007 && ripscope -b -f DIR/ripscope.db)
```

Or give the directory an ACL for the group and a default one, which lets the group write
every new file in it, whatever the builder's umask; the entries for other users decide
who else can read the index:

```sh
setfacl -m g:team:rwx,o::---,d:g:team:rwx,d:o::--- DIR
```

Do not set the sticky bit on such a directory: members could no longer remove each
other's SQLite files. Users outside the group cannot open the directory, so they cannot
use the index.

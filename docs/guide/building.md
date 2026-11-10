# Build an index

Ripscope builds its index the first time you search. Build one yourself when you
want to choose which files it covers, tell it where your headers are, or keep the
index somewhere else.

## Index a project

Index the C files in the current directory and below, without the system headers:

```console
$ ripscope -Rbk
```

It prints nothing when it succeeds and leaves one file, `ripscope.db`. Add the
directories your headers are in with `-I`, and name another index file with `-f`:

```console
$ ripscope -Rbk -I include -f ~/indexes/project.db
```

Run the same command again to update the index. You rarely need to, because a
search with `-L` and an editor search update it first.

The two forms of the command are:

```text
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] [--] [SOURCE...]
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] -i LIST
```

Ripscope creates the index only where no file exists, in an empty file, or over
an index it made. A cscope database or any other file at that path is left
untouched, and the index build fails.

Every readable file contributes to the index. Code Ripscope cannot read well
costs accuracy only where that code is.

## Choosing the files

Ripscope saves how you selected the files. This is the **selection rule**, and every
later update applies it again, as cscope does at each build. The first of these that applies wins:

1. **A file list**, `-i LIST`. File names given beside it are not read, as in cscope.
2. **Source files** given as arguments. A name given twice is read once.
3. **Automatic discovery**, when neither is given:
   - `cscope.files` in the current directory, if it can be read.
   - Otherwise the source files directly in the current directory, or in the whole
     tree with `-R`.

   Because discovery runs again at every update, creating or deleting
   `cscope.files` changes what the next update indexes.

Headers that the selected files include are indexed too. See
[Include directories](#include-directories).

A build with different sources, `-i`, `-R` or `-I` replaces the saved selection rule.

### When a file is missing

A named file that does not exist, or is not a regular file, is reported as cscope
reports it, and the rest is indexed:

```console
$ ripscope -b main.c old.c
ripscope: cannot find file old.c
```

A file that cannot be read is reported as `cannot read`. When none of the selected
files can be indexed, the build fails with `no source files found`. A new index is
not made, and an index that already existed keeps what it had. Unlike cscope,
Ripscope follows a symbolic link you name.

### Directory discovery

Source files are regular files whose last suffix is one of cscope's: `.c .h .l .y .C
.G .H .L .bp .qc .qh .sd .cc .hh .tcc .cpp .cxx .hpp .hxx` (case-sensitive).
SCCS-style names such as `s.file.c` are skipped, as in cscope. Symbolic links found in
a directory are not followed, as in cscope. With `-R`, discovery also descends into
subdirectories. A directory that cannot be read is reported, and its files keep what
the index knew about them.

### File lists

A file list names one source per entry, relative to the directory you build the index in:

```text
src/main.c
src/util.c
"src/file with spaces.c"
-I include
-k
```

- Entries are separated by spaces, tabs or newlines. Double quotes allow spaces.
  Inside quotes, `\"` and `\\` are the only escapes.
- A list may hold the options cscope reads there: `-I DIR` and `-k`, and `-q`, `-c`
  and `-p N`, which change nothing in Ripscope's answers. `-T` would cut symbols to
  eight characters, so a list naming it is refused.
- A name the list gives twice is indexed twice, as cscope does.
- The list is read again at every update, so editing it changes what is indexed.
  Reading a list from standard input (`-i -`) is not supported.
- File lists must be regular files or symbolic links to them. Pipes and devices,
  including process-substitution paths, are refused.

## Include directories

Ripscope follows literal `#include` directives and indexes the headers it finds.
It looks for `#include "file.h"` first relative to the directory the index is built
in. Then, for both `"file.h"` and `<file.h>`, it looks in this order, as cscope does:

1. Each `-I` directory, in the order given.
2. The `-I` directories in the file list.
3. The directories in the `INCLUDEDIRS` environment variable.
4. The system directory, `/usr/include` or the one `INCDIR` names, unless `-k` is given.

Headers that cannot be found are skipped. A computed include such as
`#include MACRO` is not followed.

## Updating

Run the same `-b` command again, search with `-L`, or use `r` in a
[session](sessions.md). Each brings the index up to date. An update:

- applies the saved selection rule again.
- reads a file again when its size, modification time or identity changed, or when it
  was modified too close to its last read to trust the timestamp. It analyses the file
  again only if its bytes changed.
- publishes a new revision in one transaction when anything changed. A search or
  session always reads a complete revision, and sessions see the new one at their next
  search. Reading a file whose bytes did not change publishes nothing.

A file that cannot be read keeps its last results in an update and is tried again
next time. An index build with `-b` leaves it out.

`-u` updates unconditionally, as cscope says. Every file is read and analysed again,
and nothing the index held is kept. It is also how you
[repair a damaged index](recovery.md).

## Where the index may live

Keep the index in a local folder. The sources may be anywhere.

Ripscope does not create or update an index in these places, because the SQLite
write-ahead log that lets searches read during an update is unsafe there:

- NFS, SMB and other network or cluster file systems, and Windows shares.
- WSL's view of Windows drives.
- FUSE mounts other than a block device's. That covers sshfs and cloud-sync
  clients, and also gocryptfs or mergerfs over a local disk.

An index may live in a folder OneDrive synchronises, such as a Documents or Desktop
folder OneDrive backs. OneDrive may lock the index or copy it while it is written, so
the build that makes the index there warns once. Use `-f` to keep it in a folder
OneDrive leaves alone. Ripscope knows OneDrive's folders by the `OneDrive`,
`OneDriveCommercial` and `OneDriveConsumer` environment variables. If the index is ever
damaged there, `-bu` builds it again.

Use an index from one machine only. Every process that uses the index must run on
the same machine, so a search from another machine, over NFS or a share, is unsafe,
although it is not refused.

## Sharing an index between users (Unix)

Every user of an index must be able to write it, and the files SQLite keeps beside
it. For a group to share an index privately, keep it in a directory of the group with
the setgid bit, and build it with umask 007:

```sh
chgrp team DIR && chmod 2770 DIR
(umask 007 && ripscope -b -f DIR/ripscope.db)
```

The index and SQLite's files then belong to the group, closed to other users, and
the group may write them.

Or give the directory an ACL for the group and a default one, which lets the group
write every new file in it, whatever the builder's umask. The entries for other users
decide who else can read the index:

```sh
setfacl -m g:team:rwx,o::---,d:g:team:rwx,d:o::--- DIR
```

Do not set the sticky bit on such a directory, because members could no longer
remove each other's SQLite files. Users outside the group cannot open the directory,
so they cannot use the index.

Three more facts decide who can use an index:

- A new index gets the permissions the umask gives any new file, as cscope's database
  does. SQLite gives its own files the index's permissions.
- An index rebuilt over another keeps that index's permission bits, and its group when
  the user rebuilding it belongs to that group. Root keeps its owner too. Only a user
  who may write an index can rebuild it.
- An index holds the text of the lines that hold symbols in the files it covers. Anyone
  who can read it can read those lines, even of files they cannot read.

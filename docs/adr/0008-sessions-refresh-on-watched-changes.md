# Sessions refresh when a watch reports a change

A session answers many queries. A refresh before each one would check every file's
metadata each time, which on a large project costs far more than the query itself, and
on Windows means opening every file.

## Decision

An updating session (`-l` without `-d`, or with `--auto-refresh`) keeps a watch on
what its index was made from: inotify on Linux and `ReadDirectoryChangesW` on Windows.
Before each query it asks the watch whether anything a refresh reads has changed, and
refreshes only then. The refresh itself is the ordinary one of
[ADR 0007](0007-metadata-based-refresh.md).

- **Covered before it is read.** Once a refresh holds the write lock, and before it
  looks at any file, it gives the watch the selection rule and the revision it starts
  from. The watch then covers the directories that hold the indexed files, every
  include candidate it looked up, found or missing, the file list, the directories
  discovery walks and the targets of links. Anything the new revision needs beyond
  that is covered afterwards, and the next query refreshes again.
- **Every change made before the query.** On Linux inotify queues an event before the
  system call that made the change returns, so the watch empties its queue. On Windows
  events reach a watcher thread later, so the watch creates a marker file in the root
  and waits for that file's event.
- **Only changes a refresh would see.** Events for the index and its SQLite files,
  editor swap and backup files, build output and any other name a refresh would not
  read are ignored.
- **When unsure, refresh.** Lost events, a watched directory that moved or was removed,
  or a revision another process published make the next query refresh. Where no watch
  can be made at all, every query refreshes, with a note.
- **Checks where no watch reaches.** Some directories have no watch: on Windows an
  include directory outside the root or one reached through a junction, and on Linux
  one outside the root on a network file system. Before each query the names a refresh
  read or asked about there are checked by their metadata. A directory where many of
  them are missing is checked by its times instead, which move when a name in it is
  made, removed or renamed.

## Consequences

- A query on an unchanged project answers at once from the published revision. A
  query after a change costs what an explicit `r` costs.
- Text searches also refresh first after a change, so they search the files selected
  now.
- One-shot queries refresh when they start, since no process outlives them to keep a
  watch. The editor integrations use sessions for this reason
  ([ADR 0014](0014-own-editor-integrations.md)).
- inotify reports only changes made through this machine, so sources on a network file
  system refresh before every query. So does a tree with more directories than
  `fs.inotify.max_user_watches` allows.
- A tool that sets a directory's times back after writing into it, such as bsdtar or
  7-Zip, hides the names it made there from a directory checked by its times until the
  next explicit `r`.
- Windows needs the `notify` crate's 9.0 line, which reports a buffer overflow so the
  watch can rescan. Earlier versions drop it silently. Linux uses inotify through
  `rustix`.

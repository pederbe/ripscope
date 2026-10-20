# Sessions

A session keeps one Ripscope process open and answers many queries. This is how editors
use Ripscope, exactly as they use `cscope -l`.

```text
ripscope -l [-f INDEX] [-d] [--auto-refresh | --no-auto-refresh] [-C] [-P PATH] [--json | --vim-errors]
```

Without `-d`, the session brings the index up to date at startup and again before the
first query after its sources change. With `-d`, it uses the index as it is until an
explicit `r` command. A missing index, or one that has published nothing yet, cannot be
queried.

`--auto-refresh` explicitly enables updates, including startup, even if an editor adds
`-d`. `--no-auto-refresh` explicitly disables them. Either control takes precedence over
`-d` regardless of argument order. The two controls cannot be combined. Explicit `r`
works with either setting.

## Commands

Each command is one line; as in cscope, its first character decides what it is:

| Command | Effect |
| --- | --- |
| `0PATTERN` ... `9PATTERN` | Run a query; the digit is the [field](queries.md#query-fields) |
| `c`, Ctrl-C | Toggle ASCII case-insensitive matching |
| `r`, `R`, Ctrl-R | Bring the index up to date from the saved selection rule |
| `q`, Ctrl-D, Ctrl-Z or end of input | Quit |

cscope's `C` and `F` commands, which change its file list, are answered with an empty
line and change nothing: the saved selection rule decides the files. Other commands are
reported on standard error.

In the default line format, the session prints the prompt `>> `, then for each query
`cscope: N lines` followed by N result rows. A query with no matches, or one that cannot
be answered, prints `Unable to search database`; for the latter, the reason follows on
standard error. `r` is answered with an empty line, as in cscope.

```console
$ ripscope -l -f project.db
>> 1greet
cscope: 1 lines
greet.c greet 6 void greet(const char *name)
>> q
```

A request line is at most a field digit and 4096 bytes of pattern; a longer one is read
to its end and refused as a whole.

## Seeing updates

An updating session watches what its index was made from: the indexed files and their
directories, the file list, the directories discovery searches and every include it
looked for, found or not. Before each query it takes in every change saved so far, and
when one could change an answer it refreshes first, analysing only the changed files.
So a saved edit, a new or removed file, a changed file list, a listed file that appears
or a header that appears outside the project shows in the next search, whether it came from your editor, another
editor, a generator or a branch switch. Explicit source lists stay fixed.

Queries between changes answer at once from the published revision, without looking at
the files. Swap and backup files, build output and other files a refresh would not read
change nothing, except in a directory checked by its times (below), where any file
made or removed makes the next query refresh. Several saves before a query make one
refresh, and a refresh that finds nothing changed publishes no new revision. An edit
saved while a refresh runs is taken in by the next query. Text searches read the
current files, from the file list as of the last refresh.

The watch uses inotify on Linux, and `ReadDirectoryChangesW` on Windows, where each
query briefly makes a marker file, `.ripscope-PROCESS-N.sync`, in the project directory
to know that every earlier change has been reported. When events are lost, a directory
above the watched files is made, removed or renamed (a clean build, an SDK swapped for
another, a directory link pointed elsewhere), or another process publishes a revision,
the next query refreshes anyway and the watch starts again where it must. Where the
sources cannot be watched, every query refreshes, which is slower on large projects,
and the first answer says so. That happens past Linux's `fs.inotify.max_user_watches`,
for sources on a network or FUSE file system (NFS, SMB, sshfs, WSL's Windows drives and
the like), which may not report changes made elsewhere, and on a Windows drive that
reports no changes at all. Names in a directory that cannot be watched, or outside the
project on such a file system, are checked before each query. So are include
directories outside the project on Windows. Where many names looked for in such a
directory are missing, its times are checked instead, with the names found in it. A
network file system mounted inside the project directory is not noticed: send `r` after
changes made there from another machine. A directory reached through a link is watched
where the link led when the watch began: when the directory it leads through is renamed
or replaced, rather than the link itself, send `r`. On Windows, a link to a file is not
followed: send `r` after changing the file it names.

`-u` applies at startup and to `r`. Refreshes before queries analyse only what changed.

Every query reads the newest published revision. If another process (or your editor's
index build command) updates the index, the next query sees the change, also when the
index was deleted and built again. While the index cannot be read, or is being built
afresh and has published nothing yet, a query fails and says why, as a one-shot query
does. If a refresh before a query fails or another process is updating, the query
answers from the last published revision with a warning that freshness is unconfirmed,
and the next query tries again. An unreadable file keeps its last facts, with a warning,
until a refresh can read it. Warnings use the same transport as other answer warnings.
If explicit `r` fails or is skipped, its diagnostic goes to standard error or a JSON
error frame.

## Warnings

Warnings about an answer, such as a file a text search could not read, are written to
standard error before the answer, where cscope writes its own messages and Vim passes
over them.

The [editor diagnostic guide](../editors/diagnostics.md) describes how Vim, Neovim
and Emacs distinguish warnings, failures and valid empty answers, and how to recover.

## Vim error handshake

`--vim-errors` is for Vim's built-in cscope support. After a query that cannot be
answered, Ripscope prints a short message on one line (with the reason, when the index
cannot be read) and `Press the RETURN key to continue:`, then waits for one empty line; Vim shows the message as E609, so an unusable query is
told apart from an empty result (E259). After an answer that came with warnings, a short
fixed notice does the same. Other clients should not use this flag.

## JSON sessions

With `--json`, every line Ripscope writes is a JSON value. See
[JSON interface](json.md#sessions).

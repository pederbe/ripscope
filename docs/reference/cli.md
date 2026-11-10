# Command line

The `ripscope` command builds an index, answers one search, runs a session for an
editor, checks an index or serves the Language Server Protocol. The options
below say which forms they apply to, followed by the environment variables it reads
and its exit statuses.

## Forms

```text
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] [--] [SOURCE...]
ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] -i LIST
ripscope -L [-d] [--auto-refresh | --no-auto-refresh] [-C] [-P PATH] [--json] [-f INDEX] -FIELD PATTERN
ripscope -l [-d] [--auto-refresh | --no-auto-refresh] [-C] [-P PATH] [--json | --vim-errors] [-f INDEX]
ripscope --verify [-d] [-f INDEX]
ripscope --project PATH [-f INDEX] [-kR] [-i LIST] [FILE...]
ripscope lsp
ripscope -h | --help
ripscope -V | --version
```

Ripscope reads its options as cscope does, so it accepts what Vim, Neovim, Emacs and
`cscope -Rbqk`-style scripts send.

- Single-letter options can be grouped (`-Rbqk`, `-dL`).
- An option's value may follow it directly or as the next argument (`-fINDEX`,
  `-f INDEX`, `-0name`).
- `--` ends the options.
- Options that change nothing in cscope's line and one-shot output are accepted and
  ignored. Options Ripscope cannot honour faithfully are refused, never ignored.
- `-h` and `-V` are read where they appear. The first of them ends the command line,
  and what follows it is not read. Unlike cscope, both print to standard output and
  exit with status 0.

## Options

| Option | Applies to | Meaning |
| --- | --- | --- |
| `-f INDEX` | all | The index file. `ripscope.db` when not given. |
| `-b` | index build | Build or update the index, then stop. |
| `-R` | index build | Discover sources in subdirectories too. |
| `-k` | index build | Leave out the system include directory (`/usr/include`, or `INCDIR`). |
| `-I DIR` | index build | Add an include directory. Repeatable, searched in order. |
| `-i LIST` | index build | Read the sources from a [file list](../guide/building.md#file-lists). File names beside it are not read. |
| `-u` | index build, query, session | Update unconditionally. Every file is read and analysed again, and nothing is kept. With `-b`, repairs a damaged index. In a session it applies at startup and to `r`. |
| `--` | index build | End of options. Later arguments are sources even if they start with `-`. |
| `-L` | query | Run one query and print its rows. Updates the index first unless `-d`. |
| `-l` | session | Start a [session](../guide/sessions.md). Updates the index first, and again before a query after the sources change, unless `-d`. |
| `-d` | query, session, `--verify` | Do not update. Use the index as it is, and never create one. Explicit `r` remains available. |
| `--auto-refresh` | query, session | Update as without `-d`, even when an editor supplies `-d`, in either argument order. |
| `--no-auto-refresh` | query, session | Never update, as with `-d`, in either argument order. Cannot be combined with `--auto-refresh`. |
| `-0` ... `-9` | query | [Query field](../guide/queries.md#query-fields) followed by the pattern. |
| `-C` | query, session | Ignore ASCII case. |
| `-P PATH` | query, session | cscope's prepend path. Text searches read relative names below it and show them with it. |
| `--json` | query, session | [JSON output](../guide/json.md). |
| `--vim-errors` | session | Vim error handshake (see [Sessions](../guide/sessions.md#vim-error-handshake)). |
| `--verify` | alone with `-f`, `-d` | [Check](../guide/recovery.md) the index. |
| `--project PATH` | alone | Report the project of a file or directory, for [editor integrations](#editor-integrations). |
| `-q`, `-e`, `-U`, `-v`, `-c` | any | Accepted and change nothing. Ripscope always has a fast index, these concern cscope's screen, timestamps and progress messages, and `-c` only stops cscope compressing its own database. |
| `-p N`, `-F FILE` | any | Accepted with their value and change nothing. Only cscope's full-screen display reads them. |
| `-T`, `-s DIR` | | Refused. They change cscope's answers (symbols cut to eight characters, more source directories) in ways Ripscope does not follow. |

## Editor integrations

`ripscope --project PATH` tells an editor integration which project a file belongs to
and where its index is, so that Vim, Neovim and Emacs find the same ones. It prints
one line of JSON and changes nothing but making the cache directory. Run it yourself
to see which project and index an editor uses for a file:

```console
$ ripscope --project src/main.c
{"index":"/home/me/.cache/ripscope/tool-1a2b3c4d5e6f7a8b.db","published":true,"root":"/home/me/src/tool","selection":["-k","-R"]}
```

- `root` is the nearest directory above PATH holding `cscope.files`, else the nearest
  Git repository, else the current directory when PATH is inside it, else PATH's own
  directory (PATH itself when it is a directory). A Git repository and a current
  directory at the home directory or a file system root are skipped.
- `index` is in the cache directory, `$XDG_CACHE_HOME/ripscope` (else
  `~/.cache/ripscope`) on Linux and `%LOCALAPPDATA%\ripscope` on Windows, or the
  absolute directory `RIPSCOPE_CACHE_DIR` names. Where the Linux cache directory is
  on NFS or another file system an index cannot live on, it is
  `/var/tmp/ripscope-UID` instead, private to your user. Its name is
  the root's name, with every character other than ASCII letters, digits, `.`, `_`
  and `-` made `_` and at most 40 of them, then `-` and the first 16 hexadecimal
  digits of the SHA-256 of the root's path. On Windows that path has forward slashes
  and is in lower case, by Unicode's full lowercase mapping (Rust's
  `str::to_lowercase`, which also applies the final-sigma rule).
- `selection` holds the arguments an index build (`-b`) or a session in `root` takes:
  `-k -i cscope.files`, `-k -R`, or `-k` alone where the root is the home directory or
  a file system root.
- `published` says whether the index has a published revision made with that
  selection. An index build that stopped before publishing leaves one that has none,
  and adding or removing `cscope.files` changes the selection.

After PATH, an index build's `-f INDEX` names another index, and its `-k`, `-R`,
`-i LIST` and file arguments name other sources.

In an editor on Windows, each search briefly makes a
[marker file](../guide/sessions.md#how-the-watch-works) in the project directory.

## Language server

`ripscope lsp` serves the Language Server Protocol on standard input and output, for
editors such as Helix and Neovim. It takes no options, apart from an ignored `--stdio`
that some editors add. Each request goes to the project `--project` would report for
its file. The [language server guide](../editors/language-server.md) covers setup and
what it answers.

## Environment

| Variable | Meaning |
| --- | --- |
| `INCLUDEDIRS` | Include directories searched after `-I` and a list's `-I`, as in cscope. Read when an index is built. |
| `INCDIR` | The system include directory, `/usr/include` by default. |

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success, including queries with no matches or with a pattern that cannot be used. |
| 1 | Any error. A message prefixed with `ripscope:` is written to standard error. |

A successful index build prints nothing, except a line on standard error for each named
file it cannot find or read.

When something fails, [troubleshooting](../help/troubleshooting.md) lists the common
messages and their fixes.

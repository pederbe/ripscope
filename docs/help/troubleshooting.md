# Troubleshooting

Ripscope says what went wrong in a message, in your terminal or in your editor.
Each message below comes with its cause and the fix.

## Command not found

```text
ripscope: command not found
```

The directory Ripscope was installed into is not on your `PATH`. The installer
puts it in `~/.local/bin` on Linux and `%USERPROFILE%\.local\bin` on Windows.
[Add that directory to PATH](../guide/installation-options.md#add-ripscope-to-path)
and open a new terminal. The editor plugins find Ripscope there either way.

## The full-screen interface is not supported

```text
ripscope: cscope's full-screen interface is not supported; use -b, -L or -l
```

You ran `ripscope` without saying what to do. Search with `-L`, for example
`ripscope -L -1 main`, or search from your [editor](../editors/vim.md).

## No source files found

```text
ripscope: no source files found
```

Ripscope found no C files to index. Check that you are in your project directory.
Add `-R` when the sources are in subdirectories. When a `cscope.files` is present,
Ripscope indexes only the files it names, so check its entries.

## Cannot find file

```text
ripscope: cannot find file src/old.c
```

A file named on the command line or in the file list does not exist. Ripscope
indexes the rest. Remove or correct the entry. Names in a file list are relative
to the directory you build the index in.

## No index

```text
ripscope: no index at ripscope.db
```

`-d` answers from an existing index and never creates one. Run the search without
`-d`, or build the index first with `ripscope -b`. If your index has another name,
give it with `-f`.

## Not a ripscope index

```text
ripscope: cscope.out is not a ripscope index; it was left unchanged
```

The path you gave with `-f` holds a cscope database or some other file. Ripscope
keeps its own index and does not read cscope's. Leave out `-f` to use
`ripscope.db`, or name a new file.

## A search prints nothing

A search that finds no matches prints nothing and exits with status 0. If you
expected a match, check these in turn.

1. **The pattern.** A symbol pattern must match the whole name. `-1 reet` does not
   find `greet`, but `-1 '.*reet'` does. Use `-4` to search for text inside names.
2. **The files.** `ripscope -dL -7 .` lists every indexed file. If yours is
   missing, build the index again with `-R` or with the file named.
3. **The headers.** A header is indexed when an indexed file includes it and
   Ripscope can find it. Add its directory with `-I DIR`.
4. **Standard error.** `ripscope: invalid query pattern` there means the pattern
   could not be used, which also prints no rows.

## Invalid query pattern

```text
ripscope: invalid query pattern: invalid syntax
```

The pattern is not a valid regular expression, or the search cannot take it. For
example, a definition search takes a name or a pattern that matches whole names,
not text with spaces. [Queries](../guide/queries.md#patterns) lists what each
search accepts.

## The index is on a network or synchronised folder

```text
ripscope: /mnt/share/ripscope.db is on NFS, where the index's write-ahead log is unsafe; keep the index in a local folder that is not synchronised
```

The index must be on a local disk. Your sources can stay where they are. Name an
index in a local folder with `-f`, such as `-f ~/.cache/project.db`. The editor
plugins already keep their indexes in your local cache directory.
[Where the index may live](../guide/building.md#where-the-index-may-live) has the
full list of places Ripscope refuses.

## Damaged index

```text
ripscope: damaged index: its name postings cannot be read; rebuild it with -bu and the options it was built with
```

Build the index again from the sources with `-bu` and the options you first built
it with, such as `ripscope -bu -R -k`. To check an index, run `ripscope --verify`.
See [Check and repair an index](../guide/recovery.md).

## The inotify watch limit is reached

A warning with a search result contains this text:

```text
the inotify watch limit is reached; raise fs.inotify.max_user_watches
```

Linux limits how many files and directories one user can watch. Past the limit,
a session checks the sources before every search, which is slower on a large
project. Raise the limit, for example with
`sudo sysctl fs.inotify.max_user_watches=524288`, and restart your editor.

## The first search in an editor takes a while

```text
ripscope: indexing /home/me/project for the first time...
```

The first search in a project builds its index, which takes a moment on a large
project. Later searches reuse it and read only the files you changed.

## The editor cannot find Ripscope

```text
ripscope: the ripscope executable was not found; install it with :RipscopeInstall
Ripscope executable not found; install it with :RipscopeInstall
```

Vim prints the first message and Neovim the second.

Run `:RipscopeInstall` in Vim or Neovim, or `M-x ripscope-install` in Emacs. Each
downloads the executable that matches the plugin.

## Vim reports E609 or E259

E259 means the search ran and found nothing. E609 means the search could not be
answered, or that its answer came with a warning.
[Results, warnings and failures](../editors/diagnostics.md) explains each case in
Vim, Neovim and Emacs and how to recover.

## Still stuck

[Open an issue](https://github.com/pederbe/ripscope/issues/new/choose) with the
command you ran, what it printed and the output of `ripscope --version`.

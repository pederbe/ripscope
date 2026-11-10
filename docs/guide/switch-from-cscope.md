# Switch from cscope

Ripscope takes the command lines you already give cscope and prints rows in the
same format, so switching is mostly a matter of changing the program name.
Scripts, Vim and Emacs each take one small step, and a few differences are worth
knowing before you start.

## In scripts

Replace `cscope` with `ripscope` and keep the options.

```console
$ ripscope -Rbqk
$ ripscope -dL -3 greet
main.c main 5 greet("world");
```

The index is `ripscope.db` instead of `cscope.out`. Ripscope never reads or
overwrites a cscope database, so both tools can run side by side in one directory
while you compare them. Pointing Ripscope at an existing `cscope.out` fails and
leaves the file alone:

```console
$ ripscope -b -f cscope.out
ripscope: cscope.out is not a ripscope index; it was left unchanged
```

Your `cscope.files` works as it is, including the `-I` and `-k` lines in it. So do
`INCLUDEDIRS` and `INCDIR`.

You can drop the rebuild step from most scripts. A `-L` search brings the index up
to date first and reads only the files that changed. Keep `-d` where you want an
answer from the index as it is.

## In Vim

Install the [Vim plugin](../editors/vim.md#set-up-in-one-step). Your `:cs find`
commands and mappings keep working, and you no longer build or add a database
yourself.

Remove the lines in your vimrc that run `cs add` or set `cscopeprg`. Ripscope
stays out of the way while another cscope connection is open or `cscopeprg` names
another program.

In Neovim, which has no built-in cscope support, use the
[Neovim plugin](../editors/neovim.md) and `:RipscopeFind`.

## In Emacs

Install the [Emacs package](../editors/emacs.md#set-up-in-one-step). `M-.` and
`M-?` work at once. To keep the keys xcscope gave you, bind its prefix to
Ripscope's command map:

```elisp
(define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)
```

## What is different

- **No full-screen interface.** Running `ripscope` without `-b`, `-L` or `-l`
  prints `ripscope: cscope's full-screen interface is not supported; use -b, -L or -l`.
  Search from your editor or with `-L`. Field 5 is a text search, as in cscope's
  line mode, and never changes text.
- **Two options are refused.** `-T` and `-s` would change the answers, so Ripscope
  stops with a message instead of ignoring them.
- **Some options do nothing.** `-q`, `-e`, `-U`, `-v`, `-c`, `-p N` and `-F FILE`
  are accepted so your command lines keep working. The index is always fast, with
  or without `-q`.
- **Some answers change.** Ripscope finds increments, indexed stores and
  definitions that cscope misses. It also works out each row's function within
  its own file, so some rows name a different function than cscope's do. Where
  alternative function heads under `#if` share one body, a call names only the
  last head as its caller. A script that compares output byte for byte with
  cscope's can see those rows change.

[Differences from cscope](../reference/differences-from-cscope.md) lists every
deliberate difference with examples.

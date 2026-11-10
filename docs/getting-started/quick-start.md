# Quick start

A minute from now you will have found a definition in your own C project and
followed its callers. You need Ripscope [installed](installation.md) and a
terminal open in the project.

## 1. Find a definition

Search for a function in your project. This example finds `main`.

```sh
ripscope -R -L -1 main
```

Ripscope indexes the C files here and in subdirectories, then prints one row for
each definition, like this one:

```text
main.c main 3 int main(void )
```

A row is the file, the function, the line number and the source line. Replace
`main` with another function name to explore your code.

If you see `ripscope: no source files found`, the directory holds no C files.
Change to your project directory and run the command again.

## 2. Find more code

Use the search that fits your question.

| Find | Command |
| --- | --- |
| A definition | `ripscope -L -1 NAME` |
| Callers of a function | `ripscope -L -3 NAME` |
| Functions it calls | `ripscope -L -2 NAME` |
| Assignments to a variable | `ripscope -L -9 NAME` |

Replace `NAME` with a function or variable from your project. You only need `-R`
the first time, because the index remembers which files you chose. Every search
brings the index up to date with your edits before it answers.
[Queries](../guide/queries.md) describes all nine searches.

## 3. Search from your editor

With the [installer](installation.md) or one plugin-manager line, open a C file in
your project and search: `:cs find g main` in Vim, `:RipscopeFind g main` in Neovim,
or `M-.` on `main` in Emacs. The first search indexes the project into your cache
directory, and later searches see your saved edits. The [Vim](../editors/vim.md),
[Neovim](../editors/neovim.md) and [Emacs](../editors/emacs.md) guides explain how
the project is found, file-list selection and optional manual updates.

## Next steps

- Follow the [example project](example-project.md) for a guided tour with known
  answers.
- [Build an index](../guide/building.md) covers include directories, file lists
  and where the index may live. Keep it on a local disk.
- Coming from cscope? Read [Switch from cscope](../guide/switch-from-cscope.md).

# Quick start

[Install Ripscope](installation.md), then open a terminal in your C project.
Keep the project index on a local disk. For other storage arrangements, see
[index locations](../guide/building.md#where-the-index-may-live).

## 1. Find a definition

Search for a function in your project. This example finds `main`.

```sh
ripscope -R -L -1 main
```

Ripscope creates the index, searches subdirectories and prints each matching
file, function, line number and source line. Replace `main` with another function
name to explore your code.

## 2. Find more code

Use the search that fits your question.

| Find | Command |
| --- | --- |
| A definition | `ripscope -L -1 NAME` |
| Callers of a function | `ripscope -L -3 NAME` |
| Functions it calls | `ripscope -L -2 NAME` |
| Assignments to a variable | `ripscope -L -9 NAME` |

Replace `NAME` with a function or variable from your project. Command-line
searches refresh the index automatically after edits.

## 3. Search from your editor

With the [installer](installation.md) or one plugin-manager line, open a C file in
your project and search: `:cs find g main` in Vim, `:RipscopeFind g main` in Neovim,
or `M-.` on `main` in Emacs. The first search indexes the project into your cache
directory, and later searches see your saved edits. The [Vim](../editors/vim.md),
[Neovim](../editors/neovim.md) and [Emacs](../editors/emacs.md) guides explain how
the project is found, file-list selection and optional manual updates.

Prefer a guided example? Follow the [example project](example-project.md).
For include directories, file lists and other project options, see
[building an index](../guide/building.md).

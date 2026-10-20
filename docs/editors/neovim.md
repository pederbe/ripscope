# Neovim

Ripscope's Neovim plugin searches with Ripscope and shows the results in quickfix.
It keeps one Ripscope session per project through the [JSON interface](../guide/json.md),
so paths with spaces and Unicode work. It needs no cscope executable, shell or
Python.

**Requirements:** Neovim 0.10 or newer on Linux or Windows. Workflow checks cover
Neovim 0.12.5 on Windows and a 0.13.0 development build on Linux, with lazy.nvim too.

## Set up in one step

With [lazy.nvim](https://github.com/folke/lazy.nvim), add this to your plugin
list. It installs the plugin and the `ripscope` executable.

```lua
{ 'pederbe/ripscope', branch = 'release', build = ':RipscopeInstall' },
```

The `release` branch holds the latest release, and `:RipscopeInstall` installs the
executable of that same version.

Or run the [installer](../getting-started/installation.md). It installs the
executable and puts the plugin where Neovim loads packages by itself, so there is
nothing to add to your configuration. lazy.nvim, which LazyVim, kickstart, AstroNvim
and NvChad use, loads only the plugins in its list, so where it finds lazy.nvim the
installer prints the line to add to that list instead:

```lua
{ dir = vim.fn.stdpath('data') .. '/site/pack/ripscope/start/ripscope' },
```

With another plugin manager, install the `release` branch of
`https://github.com/pederbe/ripscope` and run `:RipscopeInstall` as its build step.

## First search

Open a C file in your project and find a definition:

```vim
:RipscopeFind g main
```

The first time, Neovim says `Ripscope: indexing ... for the first time` and indexes
the project in the background before it searches. Press Enter on a quickfix result
to open its source. Save an edit and search again: the results include it.

Ripscope picks the project and keeps its index for you:

- **The project** is the nearest directory above the current file that holds a
  `cscope.files`, else the nearest Git repository, else Neovim's current directory
  when the file is inside it, else the file's own directory.
- **The sources** are the files `cscope.files` names, when there is one. Otherwise
  Ripscope finds the C sources in the project and its subdirectories.
- **The index** lives in your cache directory, `~/.cache/ripscope` on Linux and
  `%LOCALAPPDATA%\ripscope` on Windows, named after the project. On Windows a search
  briefly makes a marker file in the project directory.

Each search goes to the current file's project, so several projects can be open at
once, each with its own session. A search from a window with no file, such as the
results list, goes to the project searched last. Your home directory and a drive are never taken
from the current directory, and a Git repository there, such as one for dotfiles, is
not taken as a project. Where the project is your home directory or a drive, only
the files directly in it are indexed. `ripscope --project FILE` shows what Ripscope
picks for a file.

## Commands

| Command | Effect |
| --- | --- |
| `:RipscopeFind FIELD PATTERN` | Search; `FIELD` is a number or letter (`s g d c t e f i a`). The pattern is the rest of the line. |
| `:RipscopeBuild` | Update the current project's index now (or build it), in the background. |
| `:RipscopeLog` | Show the last operation's output. |
| `:RipscopeInstall` | Install the `ripscope` executable of the plugin's version, when it is missing or the one the installer put there is older. `:RipscopeInstall!` installs it anyway. |

| Field | Letter | Finds |
| --- | --- | --- |
| 0 | `s` | Symbol occurrences |
| 1 | `g` | Definitions |
| 2 | `d` | Functions called by a function |
| 3 | `c` | Callers |
| 4 | `t` | Text |
| 6 | `e` | Regular expression |
| 7 | `f` | Files |
| 8 | `i` | Files including a file |
| 9 | `a` | Assignments |

For example, `:RipscopeFind c main` finds the callers of `main`. From Lua, call
`require('ripscope').find(field, pattern)`; `require('ripscope').status()` returns the
last result. These optional mappings search for the word under the cursor:

```lua
local ripscope = require('ripscope')
vim.keymap.set('n', '<Leader>cg', function() ripscope.find('g', vim.fn.expand('<cword>')) end)
vim.keymap.set('n', '<Leader>cc', function() ripscope.find('c', vim.fn.expand('<cword>')) end)
vim.keymap.set('n', '<Leader>cs', function() ripscope.find('s', vim.fn.expand('<cword>')) end)
vim.keymap.set('n', '<Leader>ca', function() ripscope.find('a', vim.fn.expand('<cword>')) end)
```

Neovim finds `ripscope` on PATH, else where the installer puts it (`~/.local/bin`,
or `%USERPROFILE%\.local\bin` on Windows). If it finds none, a search says so and
suggests `:RipscopeInstall`.

## Settings

`setup` is optional. Without a `root`, its settings apply to every project:

```lua
require('ripscope').setup({ auto_refresh = false, include_dirs = { 'include' } })
```

With lazy.nvim, put them in the spec's `opts`. With a `root`, you choose one project
for every search, and its index is `ripscope.db` in that root unless `index` says
otherwise. Build it with `:RipscopeBuild` before the first search.

```lua
require('ripscope').setup({
  exec = '/home/me/.local/bin/ripscope',
  root = '/home/me/project',
  recursive = true,
})
```

| Setting | Purpose |
| --- | --- |
| `exec = '/path/to/ripscope'` | The executable, instead of finding it |
| `root = '/path/to/project'` | One project for every search, instead of the current file's |
| `index = 'ripscope.db'` | Index path, relative to the project, instead of the cache directory |
| `recursive = true` | Discover sources in subdirectories (the default without a `root`) |
| `include_dirs = { 'include' }` | Additional header directories |
| `sources = { 'src/main.c' }` | Select source files explicitly |
| `file_list = 'sources.list'` | Read a source-file list instead |
| `timeout = 120000` | Operation timeout in milliseconds |
| `open = true` | Open result and log windows automatically |
| `auto_refresh = true` | Refresh saved source changes before the next search. Set to `false` for manual updates with `:RipscopeBuild`. |

On Windows, use full drive paths, for example
`exec = [[C:\Users\me\.local\bin\ripscope.exe]]`. Lua long strings accept backslashes
literally, and forward slashes also work. Relative paths resolve against the project
root. Paths such as `C:relative` or `\rooted` are ambiguous and refused, and so are
device namespace paths and batch or cmd backends.

## Good to know

- Save and search. Searches refresh changed source automatically, including
  external saves and header edits. Discovery follows file additions, removals and
  renames. Explicit sources stay fixed, and new file-list entries must be added to
  the list. Saves and searches never rewrite it.
- Use `:RipscopeBuild` for a Selection rule change or an explicit update.
  `auto_refresh = false` keeps manual updates.
- Successive searches in a project reuse one backend process, including after index
  builds.
- Query warnings accompany the results; an invalid query reports its error and the
  session stays usable. A timeout or broken connection fails that search; the next
  search opens a new session. Results are published only after a complete reply.
- See [results, warnings and failures](diagnostics.md) for the shared behavior and
  recovery steps.
- A failed index build or search opens the log and never shows partial results.
- One operation runs at a time. Calling `setup` again closes the running sessions.
  Exiting Neovim closes them too.

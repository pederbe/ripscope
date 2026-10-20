# Vim

Ripscope works with Vim's built-in cscope support. Its plugin connects Vim to your
project as soon as you open a C file, so `:cs find` works right away.

**Requirements:** Vim with `+cscope` and `+quickfix` (and `+job` on Windows). The
workflows are checked with Vim 9.2.1164 on Linux and Windows 11.

## Set up in one step

With [vim-plug](https://github.com/junegunn/vim-plug), add this line to your vimrc
and run `:PlugInstall`. It installs the plugin and the `ripscope` executable.

```vim
Plug 'pederbe/ripscope', { 'branch': 'release', 'do': ':RipscopeInstall' }
```

The `release` branch holds the latest release, and `:RipscopeInstall` installs the
executable of that same version.

Or run the [installer](../getting-started/installation.md). It installs the
executable and puts the plugin where Vim loads packages by itself, so there is
nothing to add to your vimrc.

With another plugin manager, install the `release` branch of
`https://github.com/pederbe/ripscope` and run `:RipscopeInstall` as its build step.

## First search

Open a C file in your project and find a definition (a new file connects once you
save it):

```vim
:cs find g main
```

The first time, Vim says `ripscope: indexing ... for the first time` while it
indexes the project, which takes a moment on a large one. Then use `:copen` to see
the results and press Enter on one to jump to it. Save an edit and search again:
the results include it.

Ripscope picks the project and keeps its index for you:

- **The project** is the nearest directory above the file that holds a
  `cscope.files`, else the nearest Git repository, else Vim's current directory when
  the file is inside it, else the file's own directory.
- **The sources** are the files `cscope.files` names, when there is one. Otherwise
  Ripscope finds the C sources in the project and its subdirectories.
- **The index** lives in your cache directory, `~/.cache/ripscope` on Linux and
  `%LOCALAPPDATA%\ripscope` on Windows, named after the project. On Windows a search
  briefly makes a marker file in the project directory.

Your home directory and a drive are never taken from Vim's current directory, and a
Git repository there, such as one for dotfiles, is not taken as a project. Where the
project is your home directory or a drive, only the files directly in it are indexed.
`ripscope --project FILE` shows what Ripscope picks for a file.

## Searches

Search with Vim's own cscope commands:

| Command | Finds |
| --- | --- |
| `:cs find s NAME` | Symbol occurrences |
| `:cs find g NAME` | Definitions |
| `:cs find d NAME` | Functions called by this function |
| `:cs find c NAME` | Callers |
| `:cs find t TEXT` | Literal text |
| `:cs find e REGEX` | Regular expression |
| `:cs find f NAME` | Files |
| `:cs find i NAME` | Files including a file |
| `:cs find a NAME` | Assignments |

`:copen` shows results in the quickfix window; `:cnext` and `:cprevious` move through
them. These optional mappings search for the word under the cursor:

```vim
nnoremap <Leader>cg :cs find g <C-R><C-W><CR>
nnoremap <Leader>cc :cs find c <C-R><C-W><CR>
nnoremap <Leader>cs :cs find s <C-R><C-W><CR>
nnoremap <Leader>ca :cs find a <C-R><C-W><CR>
```

## Commands

| Command | Effect |
| --- | --- |
| `:RipscopeConnect` | Connect to the current file's project, indexing it first if needed. |
| `:RipscopeBuild` | Update the index now (or build it), then connect. |
| `:RipscopeDisconnect` | Close Ripscope's connection. |
| `:RipscopeInstall` | Install the `ripscope` executable of the plugin's version, when it is missing or the one the installer put there is older. `:RipscopeInstall!` installs it anyway. |

Vim searches one project at a time: the project of the first C file you open.
`:RipscopeConnect` switches to the current file's project. Ripscope leaves Vim alone
when `cscopeprg` names another program, such as `gtags-cscope`, and keeps your own
`cscopequickfix`.

Vim finds `ripscope` on PATH, else where the installer puts it (`~/.local/bin`, or
`%USERPROFILE%\.local\bin` on Windows). If it finds none, opening a C file says so
and suggests `:RipscopeInstall`.

## Choosing the project yourself

Set `g:ripscope` in your vimrc to change the defaults. Without a `root`, these
apply to every project:

```vim
let g:ripscope = {'auto_refresh': 0, 'include_dirs': ['include']}
```

| Key | Meaning |
| --- | --- |
| `backend` | Absolute path to the Ripscope executable |
| `auto_refresh` | Enabled by default. Set to `0` for manual updates with `:RipscopeBuild`. |
| `include_dirs` | Include directories, relative to the project, searched in order |

With a `root`, you choose the project, the index and its sources, and nothing
connects by itself: run `:RipscopeBuild` to build the index and connect.

```vim
let g:ripscope = {
      \ 'backend': '/home/me/.local/bin/ripscope',
      \ 'root': '/home/me/project',
      \ 'index': '/home/me/project/ripscope.db',
      \ 'sources': ['src/main.c']}
```

| Key | Meaning |
| --- | --- |
| `backend` | Absolute path to the Ripscope executable |
| `root` | Absolute project directory; relative names below resolve from here |
| `index` | Absolute path of the index |
| `sources` *or* `file_list` | Explicit source files, or a [file list](../guide/building.md#file-lists) such as `'cscope.files'` (exactly one) |
| `include_dirs` | Optional include directories, searched in order |
| `auto_refresh` | Enabled by default. Set to `0` for manual updates with `:RipscopeBuild`. |

On Windows use drive paths such as `'C:/Users/me/.local/bin/ripscope.exe'` and
`'C:/Projects/example'`; backslashes are also accepted. Settings are read when the
plugin loads, so restart Vim to change them. Without a plugin manager, set
`g:ripscope` and then `source` the plugin's `plugin/ripscope.vim`.

## Good to know

- Save and search. Searches refresh changed files automatically in the same
  connection, including external edits and header changes. Unmodified loaded
  buffers can be reloaded with `:checktime` after external edits.
- Explicit source lists stay fixed. With a file list, add or rename its entries when
  membership changes. Saves and searches never rewrite that list.
- `:RipscopeBuild` reads saved files and blocks Vim while it runs. The default
  automatic refresh explicitly overrides the `-d` Vim supplies to its backend.
- An invalid query raises E609; a valid query without matches raises E259.
- Warnings keep the available rows and raise E609 too. See
  [results, warnings and failures](diagnostics.md) for diagnostics and recovery.
- Ripscope leaves Vim alone when another cscope connection is open, and refuses to
  replace unrelated cscope connections.
- Because of how Vim starts cscope programs, the executable, project and index
  paths may contain only ASCII letters, digits, `/`, `.`, `_` and `-` (plus the drive
  colon and `\` on Windows). On Windows, Ripscope hands Vim the short 8.3 name of an
  executable or project path with spaces, which needs short names on that drive
  (Windows keeps them on the system drive by default). The index path cannot hold
  spaces, since Vim turns a short name back into the long one there. With a space in
  your Windows user folder's name, set the environment variable `RIPSCOPE_CACHE_DIR`
  to a folder without spaces, such as `C:\ripscope-cache`, and the one-step setup
  works as usual. For other paths, use [Neovim](neovim.md).
- Result paths use cscope's line format: printable ASCII without spaces. For other
  paths, use [Neovim](neovim.md) or another [JSON](../guide/json.md) client.

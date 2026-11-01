# Language server

`ripscope lsp` is a language server for C. Editors that speak the Language Server
Protocol, such as Helix and Neovim, can use Ripscope through it with no plugin. It
answers four requests:

| Request | Answered by |
| --- | --- |
| Go to definition | The definition search (field 1) |
| Find references | The symbol search (field 0) |
| Incoming and outgoing calls (call hierarchy) | The callers and callees searches (fields 3 and 2) |
| Workspace symbols | The definition search, for every definition whose name holds what you type |

Results are the same rows the searches give, with Ripscope's paths and lines. Other
requests, such as hover and completion, are not answered.

**Requirements:** the `ripscope` executable on PATH, from the
[installer](../getting-started/installation.md) or a system package. It has been
checked with Helix 25.07.1 and Neovim 0.12.6 on Linux.

## Helix

Add this to your `languages.toml`, in `~/.config/helix/` on Linux or
`%AppData%\helix\` on Windows:

```toml
[language-server.ripscope]
command = "ripscope"
args = ["lsp"]

[[language]]
name = "c"
language-servers = ["ripscope"]
```

Open a C file and press `gd` on a name to go to its definition, `gr` to find its
references, or `Space S` to search the workspace symbols. `hx --health c` shows
whether Helix finds the server. Helix 25.07 has no call hierarchy.

To keep clangd for everything else, list Ripscope first. Helix asks the first server
in the list for definitions and references, and shows workspace symbols from every
server unless you turn them off for clangd:

```toml
[[language]]
name = "c"
language-servers = ["ripscope", { name = "clangd", except-features = ["workspace-symbols"] }]
```

## Neovim

Neovim 0.11 or newer starts the server with this in your `init.lua`:

```lua
vim.lsp.config('ripscope', {
  cmd = { 'ripscope', 'lsp' },
  filetypes = { 'c' },
  root_markers = { 'cscope.files', '.git' },
})
vim.lsp.enable('ripscope')
```

Neovim maps `grr` to find references, and `Ctrl-]` goes to a definition.
`:lua vim.lsp.buf.incoming_calls()` and
`:lua vim.lsp.buf.outgoing_calls()` show the call hierarchy, and
`:lua vim.lsp.buf.workspace_symbol()` searches the definitions. `:checkhealth vim.lsp`
shows whether the server is running. Neovim lists outgoing calls in the file you are
in, so ask for them on the function's definition rather than on a call to it.

Neovim asks every server attached to a file and lists all their answers. Beside
clangd, you can leave these requests to Ripscope by turning them off for clangd:

```lua
vim.lsp.config('clangd', {
  on_init = function(client)
    local capabilities = client.server_capabilities
    capabilities.definitionProvider = false
    capabilities.referencesProvider = false
    capabilities.callHierarchyProvider = false
    capabilities.workspaceSymbolProvider = false
  end,
})
```

You can use the server and the [Neovim plugin](neovim.md) together. They share the
index.

## Projects and the index

Each request goes to the project of the file it names, found by the same rule the
[editor integrations](neovim.md#first-search) use. The server keeps one
[session](../guide/sessions.md) per project, so several projects can be open at once.
The index lives in your cache directory, and the Vim, Neovim and Emacs integrations
use the same one.

The editor's workspace folder matters only where that rule would use the editor's
current directory: a file outside any `cscope.files` project or Git repository
belongs to the innermost workspace folder holding it. A workspace symbol search names
no file, so it searches every project you have opened a file in.

When you open the first file of a project that has no index yet, the server starts
indexing it in the background, and the editor's log shows
`ripscope: indexing ... for the first time`. A request waits up to 10 seconds for
that to finish. On a large project it may say `still indexing ... for the first time`
instead, so try again a little later. Later requests see your saved edits, as a
session does.

## Names and ranges

The name under the cursor comes from the text in the editor, so an unsaved edit does
not move it. Results come from the saved files.

The index keeps lines, not columns. The server finds the name in each result's line
as the file is saved now and marks it. Where that line no longer holds the name, the
result covers the whole line.

Workspace symbols ignore letter case and take only letters, digits and `_`. An empty
search lists nothing, rather than every definition in every project. A symbol's kind,
such as function or variable, is a guess from its line.

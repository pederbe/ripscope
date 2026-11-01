# The language server chooses each request's Project from its file

Editors such as Helix, Zed and VS Code reach code navigation through the Language
Server Protocol, which starts one server per editor and gives it workspace folders.
Ripscope's searches are per Project ([ADR 0014](0014-own-editor-integrations.md)), and
the Vim, Neovim and Emacs integrations find a file's Project from the file itself.

## Decision

`ripscope lsp` finds the Project of the file each request names, by the rule the
editor integrations use, and keeps one Session per Project
([ADR 0008](0008-sessions-refresh-on-watched-changes.md)). The Session runs in the
server's own process over the [JSON interface](0013-json-interface.md), so the server
answers as the editor integrations' sessions answer, from the same index, kept fresh
the same way. A workspace folder takes the place of the editor's current directory in
that rule, and matters nowhere else. A workspace symbol request names no file, so it
searches every Project the server has opened.

The name under the cursor comes from the text the editor sent. Results come from the
saved files: the index keeps lines, so the server finds each result's name in its
line as saved, and covers the whole line when it cannot.

## Consequences

- A file in a Git repository nested inside a workspace folder is searched in that
  repository, as in Vim, and not in the folder.
- The language server and the Neovim plugin share one index for a Project.
- Workspace symbols find nothing in a Project until a request names one of its
  files.
- A result whose line changed since the last refresh can point at the wrong place in
  that line. Results follow the saved files, as every other search does.

## Considered options

- **One Project per workspace folder.** It would index a different directory than the
  editor integrations for the same file, so the two would build separate indexes, and
  a file outside every folder would have no Project.
- **Ask the editor for the folders' files.** The protocol has no request that lists a
  workspace's C files, and a `cscope.files` list would be ignored.
- **One server process per Project.** Editors start one server per language, so it
  would need a launcher of its own and gain nothing over a Session per Project.

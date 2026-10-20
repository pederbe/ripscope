# A JSON interface beside the cscope line protocol

cscope's line protocol separates fields with spaces and has no quoting. A reader
cannot tell where a path with a space ends, and a path with a control character can
break the row.

## Decision

Ripscope keeps the line protocol unchanged and adds an opt-in `--json` mode. One-shot
rows are `[path, context, line, text]`, and sessions exchange newline-delimited JSON
frames. JSON rows keep every byte, and text that is not UTF-8 is written as an array
of byte values. The line protocol writes paths raw, as cscope does, and skips a row
whose path holds a control character, with one warning per answer that points to
`--json`.

## Consequences

- The Neovim plugin and the Emacs package's sessions use the JSON interface. Vim uses
  the line protocol through its built-in cscope support.
- Other integrations can use the [JSON interface](../guide/json.md) directly.

## Considered options

- **Escape paths inside line-protocol rows.** Unmodified editors would open the
  escaped name literally.
- **Replace the line protocol.** It would break every existing cscope integration.

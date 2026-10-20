# Results, warnings and failures

Vim, Neovim and Emacs use the same query behavior, with messages delivered through
each editor's interface. A warning accompanies an answer; a failure means the
requested operation could not complete. Read the diagnostic even when rows are
available or the editor reports completion.

| Outcome | Vim | Neovim | Emacs |
| --- | --- | --- | --- |
| Matches | Quickfix results | Quickfix results after the complete JSON response | The xref buffer after the complete JSON response, or a jump to a single definition |
| Valid query, no matches | E259, without E609 | `No matches` | `Ripscope found no ...` in the echo area |
| Answer with warnings | Available rows and E609: `this answer may be incomplete` | Available rows and a warning notification; details in `:RipscopeLog` | Available rows, and the warning in the echo area and `*Messages*` |
| Invalid pattern | E609: `the query pattern cannot be used` | Failed query with its reason in `:RipscopeLog` | `Ripscope could not search for ...` with `invalid query pattern` in the echo area |
| Index cannot be read | E609 with the reason, or a connection error at startup | Failed query with diagnostics in `:RipscopeLog` | An error in the echo area with the session's diagnostics |
| Index build fails | `RipscopeBuild` reports the exit status and output | Failed build with output in `:RipscopeLog` | `Ripscope could not index ...`, with the output in `*ripscope-index*` |

CLI one-shot queries, including `--json`, retain cscope's status-0 response to an
invalid pattern: check standard error as well as the exit status. Sessions tell the
invalid request apart directly, through Vim's error handshake or a JSON `error` frame,
which Neovim and Emacs show as a failed search.

## Warnings keep the available results

A live text query skips a source file it cannot read and names it in a warning.
Rows from readable files remain available. Even an answer with zero rows can
carry a warning: it does not establish that the text is absent from the skipped
file. In manual mode, Stored queries continue to use the index's facts after source
deletion. Automatic Refresh applies the Selection rule and removes deleted files when
it can complete.

If automatic Session Refresh fails or another process is updating, Vim, Neovim and
Emacs keep navigable rows from the last published revision and show a warning that
freshness is unconfirmed. Fix the reported problem and search again.

Vim's built-in cscope reader shows only a short warning notice. Repeat the same
query with the CLI to see the details, for example:

```text
ripscope -dL -f /absolute/project/ripscope.db -P /absolute/project -4 text
```

Use the same index, field, pattern and case option as the editor. Under `silent!`,
Vim may leave E259 in `v:errmsg` after E609; inspect `:messages` or use an ordinary
`try`/`catch` to distinguish the diagnostic from a valid empty answer.

## Recovery

Fix an invalid pattern and search again. Restore an unreadable source file and
retry a text query; it reads the live file without an index build. Run the
normal stored search after a saved edit to refresh navigation automatically. Use the
editor's index command for Selection rule changes, explicit updates
or intentional manual mode. Open Sessions see newly published revisions without a
reconnect, including an index built afresh at the same path.

A failed refresh leaves the previous committed revision available. A killed or
timed-out index build has unconfirmed completion: its exit alone does not tell
whether it committed. Inspect the diagnostic and retry the index build before
relying on freshness.

Neovim and Emacs show query rows only after the response's final `ready` frame. A
timeout, truncated response or broken connection fails that search; the next
explicit search opens a new session. Vim owns its result reader; treat any rows
already shown by a failed or interrupted search as incomplete, and reconnect Vim if
its backend exited.

# Results, warnings and failures

Vim, Neovim and Emacs each show in their own way whether a search found matches,
found nothing, came with a warning or failed. A warning accompanies an answer. A failure means the search or index build could not complete. Read the
message even when rows are available.

| Outcome | Vim | Neovim | Emacs |
| --- | --- | --- | --- |
| Matches | Quickfix results | Quickfix results after the complete JSON response | The xref buffer after the complete JSON response, or a jump to a single definition |
| Valid query, no matches | E259, without E609 | `No matches` | `Ripscope found no ...` in the echo area |
| Answer with warnings | Available rows and E609: `this answer may be incomplete` | Available rows and a warning notification, with details in `:RipscopeLog` | Available rows, and the warning in the echo area and `*Messages*` |
| Invalid pattern | E609: `the query pattern cannot be used` | Failed query with its reason in `:RipscopeLog` | `Ripscope could not search for ...` with `invalid query pattern` in the echo area |
| Index cannot be read | E609 with the reason, or a connection error at startup | Failed query with diagnostics in `:RipscopeLog` | An error in the echo area with the session's diagnostics |
| Index build fails | `RipscopeBuild` reports the exit status and output | Failed build with output in `:RipscopeLog` | `Ripscope could not index ...`, with the output in `*ripscope-index*` |

On the command line, one-shot queries, including `--json`, keep cscope's status 0
for an invalid pattern, so check standard error as well as the exit status. Sessions tell the
invalid request apart directly, through Vim's error handshake or a JSON `error` frame,
which Neovim and Emacs show as a failed search.

## Warnings keep the available results

A live text query skips a source file it cannot read and names it in a warning.
Rows from readable files remain available. Even an answer with zero rows can
carry a warning, and then it does not show that the text is absent from the skipped
file. With automatic updates turned off, searches answered from the index keep working
after a source file is deleted. An automatic update removes deleted files from the
index when it can complete.

If an automatic update fails or another process is updating the index, Vim, Neovim
and Emacs keep the rows from the last complete index and show a warning that
freshness is unconfirmed. Fix the reported problem and search again.

Vim's built-in cscope reader shows only a short warning notice. Repeat the same
query on the command line to see the details, for example:

```text
ripscope -dL -f /absolute/project/ripscope.db -P /absolute/project -4 text
```

Use the same index, field, pattern and case option as the editor. Under `silent!`,
Vim may leave E259 in `v:errmsg` after E609. Inspect `:messages` or use an ordinary
`try`/`catch` to tell the diagnostic from a valid empty answer.

## Recovery

- **Invalid pattern.** Fix the pattern and search again.
- **Unreadable source file.** Restore the file and repeat the text search. It reads the
  file directly, without an index build.
- **Stale results.** Save the file and search again. The search updates the index first.
- **Changed file selection.** Use the editor's index command after changing which files
  the project covers, for an explicit update, or when automatic updates are off. Open
  editors see the new index without reconnecting, including an index built afresh at
  the same path.
- **Failed update.** The previous complete index stays available. When an index build
  was killed or timed out, its exit alone does not tell whether it finished. Read the
  message and run the index build again before relying on fresh results.
- **Timeout or broken connection.** Neovim and Emacs show rows only after the complete
  response, so a timeout, truncated response or broken connection fails that search,
  and the next search opens a new session. Vim reads results itself. Treat any rows
  already shown by a failed or interrupted search as incomplete, and reconnect Vim with
  `:RipscopeConnect` if Ripscope exited.

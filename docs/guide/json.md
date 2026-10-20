# JSON interface

cscope's line format separates fields with spaces and has no quoting, so a row whose
path holds a control character cannot be carried, and a line's bytes may be shown
escaped. Add `--json` to any query or session to get exact output instead. Nothing else
about the query changes: the same rows, in the same order, with the same contexts and
text.

`--json` cannot be combined with `--vim-errors`, index builds or `--verify`.

## One-shot queries

Each result is one line containing a JSON array `[path, context, line, text]`:

```console
$ ripscope --json -dL -f "navigation index.db" -1 greet
["src/café/greet.c","greet",6,"void greet(const char *name)"]
```

- `path` is a string. Relative paths are relative to the directory the index was built
  from, or below `-P PATH` for text searches.
- `context` is a string and `line` a positive integer.
- `text` is a string when the line is valid UTF-8, and otherwise an array of byte values
  (0 to 255), so nothing is lost. A text search's line of a CR LF file keeps its
  carriage return.

An empty standard output with no diagnostic means no matches. Warnings and errors
go to standard error. A query that cannot read the index, or whose output fails,
exits with status 1. An unusable pattern follows cscope's one-shot behavior: no rows,
a diagnostic on standard error, and status 0. Check both streams; status 0 alone
does not establish that the pattern was valid.

## Sessions

In a `--json` session, every line Ripscope writes is one JSON value:

| Frame | Meaning |
| --- | --- |
| `{"type":"ready"}` | Ready for the next command (at startup and after each command). |
| `{"type":"result","count":N}` | A query's answer; exactly N row arrays follow. N may be 0. |
| `{"type":"error","message":"..."}` | A request that could not be answered, or an `r` that did not bring the index up to date: it failed, another process was updating the index, or the index is kept where it cannot be updated. The session continues. |
| `{"type":"warning","message":"..."}` | Something worth knowing about the answer before it, such as a file a text search could not read or a refresh before the query that did not complete. Zero or more follow the response and all its rows, before `ready`. |
| `{"type":"rebuilt"}` | The `r` command brought the index up to date (warnings may follow). |

Parse frames as JSON objects; do not rely on key order, and ignore keys you do not know.

```console
$ printf '1greet\nq\n' | ripscope --json -dl -f project.db
{"type":"ready"}
{"type":"result","count":1}
["greet.c","greet",6,"void greet(const char *name)"]
{"type":"ready"}
```

Commands are the same as in [line-format sessions](sessions.md#commands), one line of at
most a field digit and 4096 bytes each. A client should treat a missing `ready` frame as
a failure.

A JSON session refreshes before a query after a change as a [line session
does](sessions.md#seeing-updates). A refresh that fails still yields the query's result
from the last published revision, followed by a warning that its freshness is
unconfirmed. A refresh that succeeds adds no frame. The `rebuilt` frame belongs only to
explicit `r`.

## Path rules

- Paths may contain spaces and any Unicode characters except NUL.
- Paths are stored as the index build named them (on Windows with `/` as the separator), not
  case-folded.
- A path that is not valid UTF-8 (possible on Linux) is written as an array of byte
  values, as text is.

## Editor integration

The [Neovim adapter](../editors/neovim.md) uses the JSON interface directly. Other
integrations can launch Ripscope and decode these responses with their language's
JSON support.

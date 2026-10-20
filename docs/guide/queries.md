# Queries

```text
ripscope  -L [-f INDEX] [-C] [-P PATH] [--json] -FIELD PATTERN     update, then query
ripscope -dL [-f INDEX] [-C] [-P PATH] [--json] -FIELD PATTERN     query without updating
```

## Query fields

| Field | Finds | Vim letter | Pattern | Reads |
| --- | --- | --- | --- | --- |
| `-0` | All occurrences of a C symbol | `s` | symbol | index |
| `-1` | Definitions of a symbol | `g` | symbol | index |
| `-2` | Functions called by a function | `d` | symbol | index |
| `-3` | Functions calling a function | `c` | symbol | index |
| `-4` | A literal text string | `t` | literal text | current files |
| `-5` | A literal text string (cscope's "change text" in line mode) | | literal text | current files |
| `-6` | An extended regular expression | `e` | regex | current files |
| `-7` | Files by name | `f` | regex on the path | index |
| `-8` | Files that `#include` a file | `i` | regex on the include name | index |
| `-9` | Assignments to a symbol | `a` | symbol | index |

Each result row has four parts, separated by single spaces:

```text
greet.c greet 6 void greet(const char *name)
│       │     │ └── displayed source text
│       │     └──── line number
│       └────────── context: enclosing function or macro, <global>, or <unknown>
└────────────────── file path
```

For field 2 the context is the *called* function. For field 1 it is the pattern you
searched for, as in cscope; a field-1 pattern with blanks cannot be shown that way, so
it is refused with a pointer to `--json`.

Rows come in cscope's order: rows at file scope first, then the rest. Each row's
context is the function or macro its name sits in, worked out within its file; cscope's
contexts can differ, depending on how its database was built (see
[Differences from cscope](../reference/differences-from-cscope.md#query-results)).

## Patterns

**Symbol patterns** (fields 0, 1, 2, 3, 9). A plain C identifier matches exactly.
If the pattern contains any of `^ . [ { * + $ | (`, it is a POSIX extended regular
expression that must match the **whole** name: `greet.*` matches `greet` and
`greet_count`, but `reet` matches nothing. Trailing blanks are ignored.

**Text** (fields 4 and 5) is a literal byte string, including spaces.

**Regular expressions** (field 6) are POSIX extended regular expressions applied to
each line. `^` and `$` anchor to line boundaries, and `.` never matches a newline. A
line of a CR LF file ends with its carriage return, as in cscope.

**File and include patterns** (fields 7 and 8) are regular expressions that may match
anywhere in the path or include name: `-7 greet` finds `greet.c` and
`include/greet.h`.

Patterns are limited to 4096 bytes. Matching is byte-based in the C locale: a pattern in
fields 4 to 6 may hold any byte but NUL and a line break, UTF-8 or not, one in fields 7
and 8 any but those and a carriage return, and symbol patterns are printable ASCII. A
pattern that cannot be used gives no rows, as in cscope, with the reason on standard
error; a few that cscope does answer, such as back-references, are refused too (see
[Differences from cscope](../reference/differences-from-cscope.md#query-results)).

## Case-insensitive matching

Add `-C` (or send `c` in a [session](sessions.md)) to ignore ASCII letter case in every
field. Non-ASCII letters are always compared exactly. As in cscope, the pattern itself is
lowercased, escapes and classes included, so under `-C` `\W` means `\w`, `\S` means `\s`,
and `[[:upper:]]` matches nothing.

## Updating or not

- `-L` first brings the index up to date from the saved selection rule, then answers
  from the new revision. If another process is updating the index, it answers from the
  last published revision and says so.
- `-dL` answers from the index as it is, and never creates one.
- An index that has published nothing yet, because another process is building it
  afresh or its first index build failed, is no index: `-dL` fails and says so, as does `-L`
  while another process is building it; otherwise `-L` builds it, as it builds a missing
  one.

## Stored and live fields

Fields 0, 1, 2, 3, 7, 8 and 9 are answered from the index, so they keep working when
sources are moved or deleted. Text searches (fields 4, 5 and 6) read the **current**
contents of the indexed files, as cscope does, even with `-d`. A file that cannot be
read is skipped, with a warning on standard error.

Live searches read files relative to the directory the index was built in. `-P PATH`
does what it does in cscope: relative names are read below `PATH`, and text rows show
them with it (`PATH/src/main.c`). Stored rows keep the names the index holds.

## Output

Paths are printed as they are, spaces and non-ASCII characters included, as cscope
prints them. A row whose path holds a control character cannot be carried by the line
format and is skipped, with one warning. Displayed text is compacted as cscope compacts
it; control bytes in it are shown as `\xHH`. A line a text search finds keeps its tabs
and a CR LF ending, as cscope prints it. [`--json`](json.md) carries every byte.

Warnings go to standard error before the first row, where xcscope and Vim pass over
them.

## Exit status

Ripscope exits with status 0 when the query ran, including when nothing matched or the
pattern could not be used, and 1 on any other error, with a message prefixed
`ripscope:` on standard error.

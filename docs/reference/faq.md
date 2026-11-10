# FAQ

Ripscope follows cscope closely, keeps its index current and is fast. The short
answers follow.

## Is Ripscope a drop-in replacement for cscope?

Yes for searching from an editor or a script. Ripscope takes cscope's query fields,
its `-L` and `-l` output, its common options (`-b`, `-R`, `-k`, `-I`, `-i`, `-d`, `-C`,
`-P`) and `cscope.files`. It has no full-screen interface, does not change source text
and keeps its own index instead of `cscope.out`. Some answers differ, because
Ripscope works out each row's function in its own way and reads specific constructs
differently where cscope misreads them. [Switch from cscope](../guide/switch-from-cscope.md) walks
through the move, and [Differences from cscope](differences-from-cscope.md) lists
the deliberate differences.

## Can I point my existing cscope setup at Ripscope?

Yes. Scripts that run `cscope -Rbqk` can run `ripscope -Rbqk`. The index is
`ripscope.db` unless `-f` names another. In Vim, the [plugin](../editors/vim.md) sets
`cscopeprg` for you, so `:cs find` keeps working. Vim's `cscopeprg` can also run
Ripscope directly, without the plugin:

```vim
set cscopeprg=ripscope
cs add ripscope.db
```

Vim starts it with `-d`, so run your index build again to take in edits. For Emacs
and Neovim, use the included packages, which search through xref and the quickfix
list.

## My query returned nothing. Is that "no matches" or an error?

No matches exits with status 0 and no output, as does a pattern that cannot be used
(with the reason on standard error). An error exits with status 1 and prints a
`ripscope:` message on standard error. In sessions, `--json` distinguishes an empty
`result` from an `error` frame, and Vim with `--vim-errors` reports E259 versus E609.

## Do I need to rebuild the index after editing?

No. Save the source, then search. A `-L` query brings the index up to date first, and a
session does so before the first query after a change, analysing only changed files.
Plain `-d` keeps manual control with `r` or an index build, and `--auto-refresh`
overrides an editor's `-d`. See [Sessions](../guide/sessions.md#seeing-updates).

## What if the index is damaged?

`ripscope --verify` checks it. `ripscope -bu`, with the options you built the index
with, rebuilds it from the sources. See
[Check and repair an index](../guide/recovery.md).

## Can several editors use one index at once?

Yes. Any number of sessions and queries can read one index, and each query reads a
complete revision. One process updates it at a time. Index builds wait for each other,
and a query or session that finds another process updating answers from the last
published revision, with a note. While another process builds an index for the first
time, there is no revision yet, and a query says so.

## Does Ripscope support C++?

No. C++ and Java come much later, if at all.

## Is Ripscope faster than cscope?

Yes. On Linux, with 3,579 files of the Linux kernel, symbol, definition and caller lookups take
about a tenth of a millisecond, hundreds of times faster than plain cscope. Against
`cscope -q`, text searches run about 6 times faster, regular-expression searches 10
times and assignment searches over 70 times. The [benchmarks](benchmarks.md) show
every search on Linux and Windows, including the few that `cscope -q` wins.

## Does Ripscope send data anywhere?

No. The `ripscope` executable reads your source files and writes the index. It never
uses the network. Only installing does: the install scripts, and the editor
integrations' install command, download a release.

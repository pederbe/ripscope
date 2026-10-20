# FAQ

## Is Ripscope a drop-in replacement for cscope?

Ripscope supports editor-driven, read-only C navigation through cscope's query
fields, `-L`/`-l` row and session protocol, common options (`-b`, `-R`, `-k`, `-I`,
`-i`, `-d`, `-C`, `-P`) and selection rules (`cscope.files`, `-R`, `-I`). Its own
query contexts and bounded Corrections can change answers. It has no terminal UI,
does not change source text and uses its own index format. Read
[Differences from cscope](differences-from-cscope.md) and
[limitations](limitations.md) before changing an existing setup.

## Can I point my existing cscope setup at Ripscope?

Vim's `cscopeprg` can run Ripscope directly; the [Vim page](../editors/vim.md) has a ready
configuration. For Emacs and Neovim, use the included packages, which search through
xref and the quickfix list. Scripts that run `cscope -Rbqk` can run `ripscope -Rbqk`; the index is
`ripscope.db` unless `-f` names another.

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

`ripscope --verify` checks it; `ripscope -bu` with the options you build the index with
rebuilds it from the sources. See [Checking and repairing](../guide/recovery.md).

## Can several editors use one index at once?

Yes. Any number of sessions and queries can read one index, and each query reads a
complete revision. One process updates it at a time: index builds wait for each other, and a
query or session that finds another process updating answers from the last published
revision, with a note (while another process builds an index afresh, there is none yet,
and a query says so).

## Does Ripscope support C++?

No. C++ and Java come much later, if at all.

## Is Ripscope faster than cscope?

The [search benchmarks](benchmarks.md) compare Ripscope with cscope and its
indexed search on Linux and Windows. They include timings for definitions,
callers, text, patterns and the other navigation queries.

## Does Ripscope send data anywhere?

No. The `ripscope` executable reads your source files and writes the index. It never
uses the network. Only installing does: the install scripts, and the editor
integrations' install command, download a release.

# Comparison with other tools

Ripscope is for C navigation through cscope-style queries: definitions, callers,
callees, assignments and includes, from the editor you already use. It works on
written source without a compiler, preprocessor or compile database. cscope,
Universal Ctags, GNU Global and clangd each fit other work better.

| Tool | Where it fits |
| --- | --- |
| Ripscope | C-only cscope queries with atomic index updates, stored symbol answers and live text searches. [Editor setup](../editors/vim.md) and [limitations](limitations.md) describe the supported workflow. |
| [cscope](https://cscope.sourceforge.net/) | Familiar symbol, definition, caller, callee and include searches, with a curses terminal interface as well as command-line front ends. Keep it if you rely on its terminal UI or text-changing features. Ripscope's [intentional differences](differences-from-cscope.md) include its own index and query semantics. |
| [Universal Ctags](https://docs.ctags.io/en/latest/man/ctags.1.html) | Tag files let editors locate language objects and jump to definitions across many languages. Its parsers and output formats are documented in the manual. |
| [GNU Global](https://www.gnu.org/software/global/manual/global.html) | Source tagging for definitions, references and other symbols, with editor, command-line and browser interfaces. |
| [clangd](https://clangd.llvm.org/design/compile-commands) | Compiler-based navigation, completion and diagnostics. Project compile flags provide include paths, language settings and preprocessor definitions. clangd also has fallback commands when a compilation database is absent. |

## When to choose Ripscope

Choose Ripscope when you want to explore C you have not configured a build for. It
reads every `#if` branch as written, so code for other platforms and configurations
is searchable too. On Linux, with 3,579 files of the Linux kernel, symbol, definition and caller
lookups take about a tenth of a millisecond, and the index follows your saved edits
without a rebuild step. The
[benchmarks](benchmarks.md) compare it with cscope and `cscope -q` on the Linux
kernel, Redis and zlib.

## When to choose another tool

- **You need types, macro expansion, completion or diagnostics.** Use clangd. Ripscope
  does not resolve types or expand macros, which leaves
  [recognition limits](limitations.md#language-and-analysis) a compiler does not have.
- **You work in C++ or another language.** Ripscope reads C only. Universal Ctags and
  GNU Global cover many languages.
- **You rely on cscope's full-screen interface or its text-changing feature.** Keep
  cscope.
- **You work on macOS.** Ripscope runs on Linux and Windows.

## Two things to know about freshness

An ordinary update finds changed files by their size, times and identity, and hashes
recently modified files. It does not hash every source on every update. On Windows, an
in-place edit that keeps its size and restores its modification time can be missed,
and `--verify` or `-bu` reads every file.

A query always reads a complete index revision. That is not the same as always reading
the latest source bytes. See
[stored and live fields](../guide/queries.md#stored-and-live-fields).

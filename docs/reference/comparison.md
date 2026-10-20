# Comparison with other tools

Ripscope is for C navigation through cscope-style queries: definitions, callers,
callees, assignments and includes, from the editor you already use. It works on
written source without a compiler, preprocessor or compile database.

Different tools serve different workflows. The links below describe each tool in
its own documentation; this is not a benchmark or a complete feature matrix.

| Tool | Where it fits |
| --- | --- |
| Ripscope | C-only cscope queries with atomic index updates, stored symbol answers and live text searches. [Editor setup](../editors/vim.md) and [limitations](limitations.md) describe the supported workflow. |
| [cscope](https://cscope.sourceforge.net/) | Familiar symbol, definition, caller, callee and include searches, with a curses terminal interface as well as command-line front ends. Keep it if you rely on its terminal UI or text-changing features. Ripscope's [intentional differences](differences-from-cscope.md) include its own index and query semantics. |
| [Universal Ctags](https://docs.ctags.io/en/latest/man/ctags.1.html) | Tag files let editors locate language objects and jump to definitions across many languages. Its parsers and output formats are documented in the manual. |
| [GNU Global](https://www.gnu.org/software/global/manual/global.html) | Source tagging for definitions, references and other symbols, with editor, command-line and browser interfaces. |
| [clangd](https://clangd.llvm.org/design/compile-commands) | Compiler-based navigation, completion and diagnostics. Project compile flags provide include paths, language settings and preprocessor definitions; clangd also has fallback commands when a compilation database is absent. |

## Updates and analysis in Ripscope

Ripscope reads all written `#if` branches and does not resolve types or arbitrary
macro expansions. That can help when inspecting unfamiliar C without configuring
a build, while leaving [recognition limits](limitations.md#language-and-analysis)
that a compiler-based tool can approach differently.

An ordinary Refresh detects changes from size, file times and identity, and
hashes recently modified files. It does not hash every source on every update.
On Windows, an in-place edit that keeps its size and restores modification time
can be missed; `--verify` or `-bu` reads every file. Atomic publication guarantees
a complete index revision for a query, not that every stored query always reads
the latest source bytes. See [stored and live fields](../guide/queries.md#stored-and-live-fields).

Vim.s cscope row parser splits paths at spaces. Ripscope.s
[JSON interface](../guide/json.md), used by its Neovim and Emacs packages, preserves
exact path and line bytes. Choose the integration that fits your paths and editor.

The [search benchmarks](benchmarks.md) compare Ripscope, cscope and cscope `-q`
on three pinned C selections, with timing and source-backed answer judgments.
The [proof page](differences-from-cscope.md) shows further bounded behavior
evidence and its limits.

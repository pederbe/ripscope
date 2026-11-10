# Ripscope documentation

<p class="docs-lead">Find your way around C code with <strong>familiar cscope searches</strong>, a single executable and your favorite editor.</p>

Ripscope finds definitions, callers and assignments in C, from Vim, Neovim, Emacs
or the command line. It keeps its index up to date as you save, and it needs no
compiler or build setup.

```console
$ ripscope -R -L -3 greet
main.c main 5 greet("world");
```

<div class="docs-grid">
<a class="docs-card" href="getting-started/installation.md">
<span class="card-label">01 / Install</span>
<strong>Get Ripscope</strong>
<span class="card-description">Install on Linux or Windows and check that it works.</span>
<span class="card-action">Install Ripscope →</span>
</a>
<a class="docs-card" href="getting-started/quick-start.md">
<span class="card-label">02 / First search</span>
<strong>Explore your project</strong>
<span class="card-description">Find a definition and follow the code in your own C project.</span>
<span class="card-action">Run your first search →</span>
</a>
<a class="docs-card" href="#keep-navigation-in-your-editor">
<span class="card-label">03 / Your editor</span>
<strong>Jump straight to the code</strong>
<span class="card-description">Connect Vim, Neovim or Emacs and navigate from your editor.</span>
<span class="card-action">Choose your editor →</span>
</a>
<a class="docs-card" href="reference/cli.md">
<span class="card-label">04 / Reference</span>
<strong>Find an option quickly</strong>
<span class="card-description">Look up commands and options when you need them.</span>
<span class="card-action">Open the command reference →</span>
</a>
</div>

## Keep navigation in your editor

Jump from a symbol to its definition, open a caller or inspect a result list
without leaving your code. Each editor guide includes setup and useful commands.

<div class="editor-links">
<a href="editors/vim.md">Vim →</a>
<a href="editors/neovim.md">Neovim →</a>
<a href="editors/emacs.md">Emacs →</a>
</div>

## Where to start

- **New to Ripscope?** [Install it](getting-started/installation.md), then follow
  the [quick start](getting-started/quick-start.md).
- **Coming from cscope?** [Switch from cscope](guide/switch-from-cscope.md) shows
  what carries over and what changes.
- **Using it in a script or CI job?** Go to [Use Ripscope in scripts](guide/scripts.md).
- **Looking up an option?** See the [command line reference](reference/cli.md).
- **Something not working?** See [troubleshooting](help/troubleshooting.md).
- **Want to contribute?** Start with [Contributing](project/contributing.md).

## When Ripscope is the wrong tool

Ripscope reads C only and runs on Linux and Windows. It reads source as written,
so it does not expand macros or resolve types the way a compiler-based tool such
as clangd does. It has no full-screen terminal interface. The
[comparison with other tools](reference/comparison.md) says where each one fits.

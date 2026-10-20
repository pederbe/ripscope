# Ripscope documentation

<p class="docs-lead">Find your way around C code with <strong>familiar cscope searches</strong>, a single executable and your favorite editor.</p>

Install Ripscope, search your C project and connect your editor. Start with the
steps below, then use the guides and reference as you need them.

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

## A little more context

- **Coming from cscope?** Read [Differences from cscope](reference/differences-from-cscope.md)
  for the familiar parts and the changes.
- **Automating a workflow?** The [JSON interface](guide/json.md) gives scripts
  structured results, including paths with spaces and Unicode.
- **Keeping an editor connected?** [Sessions](guide/sessions.md) explains the
  line protocol and index refreshes.
- **Want to contribute?** Start with [Contributing](project/contributing.md)
  and the [architecture overview](architecture/overview.md).

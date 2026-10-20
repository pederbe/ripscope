# Summary

[Introduction](README.md)

# Getting started

- [Installation](getting-started/installation.md)
- [Quick start](getting-started/quick-start.md)
  - [Example project](getting-started/example-project.md)

# User guide

- [Building an index](guide/building.md)
- [Queries](guide/queries.md)
- [Sessions](guide/sessions.md)
- [JSON interface](guide/json.md)
- [Checking and repairing](guide/recovery.md)
- [Other installation methods](guide/installation-options.md)
- [Uninstalling](guide/uninstalling.md)

# Editors

- [Results, warnings and failures](editors/diagnostics.md)
- [Vim](editors/vim.md)
- [Neovim](editors/neovim.md)
- [Emacs](editors/emacs.md)

# Reference

- [Command line](reference/cli.md)
- [Differences from cscope](reference/differences-from-cscope.md)
- [Limitations](reference/limitations.md)
- [Comparison with other tools](reference/comparison.md)
- [Benchmarks](reference/benchmarks.md)
  - [Benchmark answer evidence](reference/benchmark-answers.md)
- [FAQ](reference/faq.md)
- [Glossary](reference/glossary.md)

# Design

- [Architecture overview](architecture/overview.md)
  - [Index storage and lifecycle](architecture/storage.md)
  - [How Ripscope reads C](architecture/reading-c.md)
  - [How Ripscope answers stored queries](architecture/answering-queries.md)
  - [Quality and compatibility method](architecture/quality.md)
- [Architecture decisions](adr/README.md)
  - [0001 Pinned reference cscope](adr/0001-pinned-reference-cscope.md)
  - [0002 Measured compatibility](adr/0002-measured-compatibility.md)
  - [0003 Source read as written](adr/0003-read-source-as-written.md)
  - [0004 Own scanner](adr/0004-own-scanner.md)
  - [0005 One SQLite file per index](adr/0005-one-sqlite-file-per-index.md)
  - [0006 One update, one revision per query](adr/0006-one-update-one-revision-per-query.md)
  - [0007 Metadata-based refresh](adr/0007-metadata-based-refresh.md)
  - [0008 Sessions refresh on watched changes](adr/0008-sessions-refresh-on-watched-changes.md)
  - [0009 Stored and live queries](adr/0009-stored-and-live-queries.md)
  - [0010 Own query semantics](adr/0010-own-query-semantics.md)
  - [0011 Blocks of lines with their scope](adr/0011-blocks-of-lines-with-their-scope.md)
  - [0012 Written assignment search](adr/0012-written-assignment-search.md)
  - [0013 JSON interface](adr/0013-json-interface.md)
  - [0014 Own editor integrations](adr/0014-own-editor-integrations.md)

# Project

- [Roadmap](project/roadmap.md)
- [Contributing](project/contributing.md)
- [How Ripscope is built](project/how-it-is-built.md)
- [Website hosting and approval](project/website-hosting.md)
- [Changelog](project/changelog.md)

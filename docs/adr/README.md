# Architecture decisions

These records explain decisions that shape Ripscope's code and would surprise someone
reading it for the first time. Each one says what was decided, why, and what follows
for the code. A new decision takes the next number. When it replaces an earlier one,
the earlier record names its replacement.

## Compatibility with cscope

- [0001 A pinned cscope executable defines expected results](0001-pinned-reference-cscope.md)
- [0002 Compatibility with cscope is measured, and its misreadings are corrected](0002-measured-compatibility.md)

## Reading C

- [0003 Each file is read as written, without a compiler or preprocessor](0003-read-source-as-written.md)
- [0004 Ripscope's own scanner reads every file as cscope does](0004-own-scanner.md)

## Storage and freshness

- [0005 An index is one SQLite file in Ripscope's own format](0005-one-sqlite-file-per-index.md)
- [0006 One update at a time, and each query reads one revision](0006-one-update-one-revision-per-query.md)
- [0007 A refresh re-applies the selection rule and trusts unchanged metadata](0007-metadata-based-refresh.md)
- [0008 Sessions refresh when a watch reports a change](0008-sessions-refresh-on-watched-changes.md)

## Queries

- [0009 Stored queries read the index, and text searches read the files](0009-stored-and-live-queries.md)
- [0010 Ripscope's own query semantics give each name the context it sits in](0010-own-query-semantics.md)
- [0011 The index keeps lines in blocks with their scope, and records answer rows](0011-blocks-of-lines-with-their-scope.md)
- [0012 Assignment search includes subscript stores, increments and decrements](0012-written-assignment-search.md)

## Interfaces and editors

- [0013 A JSON interface beside the cscope line protocol](0013-json-interface.md)
- [0014 Ripscope ships its own editor integrations, set up in one step](0014-own-editor-integrations.md)
- [0015 Emacs searches through xref, with no package dependencies](0015-native-emacs-package.md)

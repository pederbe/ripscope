# Stored queries read the index, and text searches read the files

cscope answers its symbol, definition, call, file, includer and assignment searches
from its database, and reads the source files again for text and regular-expression
searches.

## Decision

Ripscope divides the query fields the same way. Fields 0, 1, 2, 3, 7, 8 and 9 are
answered from the index revision. Fields 4, 5 and 6 read the current contents of the
indexed files, limited to the indexed file list, even when `-d` skips the refresh. In
the line protocol field 5, cscope's "change this text", is a text search, as in
cscope.

## Consequences

- Stored queries keep working after source files are deleted, so the index stores each
  line's displayed text.
- A text search sees a file as it is now, which can differ from the stored facts of
  the last refresh.

## Considered options

- **Answer text searches from the index.** The index would need a full copy of every
  source, and a text search would show the text of the last refresh instead of the
  file as it is.

# One update at a time, and each query reads one revision

An index serves editor sessions while index builds and refreshes change it. cscope's
sessions keep reading the database they opened until they are asked to rebuild it.

## Decision

Every update is one SQLite transaction that holds the write lock from start to commit.
The commit publishes a new index revision. Until then queries read the previous one,
and if the update fails the previous one stays published.

- One process updates an index at a time. A second update waits up to a minute for the
  first to finish.
- Every query reads one committed revision, even if a newer one is published while it
  runs. A session's next query sees the newest revision.
- A query never fails because another process is updating. It answers from the last
  published revision, with a note on standard error, and fails only when nothing has
  been published yet.
- A refresh keeps the last facts of a file it cannot read and tries the file again next
  time.

## Consequences

- A failed or interrupted update leaves the previous revision in place, with nothing
  to repair.
- Only the `store` module opens the index, so its locking and transaction rules live in
  one place.

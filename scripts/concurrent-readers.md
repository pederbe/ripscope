# Concurrent stored queries and index builds

`stress-readers` exercises the CLI with separate query and writer processes. It
starts each scenario with a published index, then repeatedly runs `-dL` stored
queries while another process runs ordinary `-bk` or forced `-buk` index builds.
`-buk` combines the forced `-bu` operation with `-k` to keep this fixture free of
system-header discovery. The two-line C source alternates between two complete
versions before each index build, so ordinary builds actually publish revisions.
Every query must return exactly one of the two independently labelled answers.

Build the product and example together, so both use the same bundled SQLite:

```sh
cargo build --release --bin ripscope --example stress-readers
target/release/examples/stress-readers target/release/ripscope /tmp/ripscope-readers-run
```

The evidence directory must not exist. On Windows, choose a new local writable
directory instead of `/tmp`, and use the `.exe` executable names. The index and
its companion files stay inside the evidence directory.

The defaults stop each scenario at 10,000 queries, 1,000 index builds or 180
seconds, whichever comes first. Each query has a five-second deadline and each
index build a fifteen-second deadline, also capped by the scenario deadline.
Both loops stop on a failed exit, unexpected answer, diagnostic or deadline.
Any active child finishes within its deadline or is killed and reaped. The
second scenario still runs so an ordinary failure does not hide the forced
scenario. Three optional final arguments set the query, build and second limits:

```sh
target/release/examples/stress-readers target/release/ripscope /tmp/ripscope-readers-smoke 20 20 10
```

The directory retains:

- `metadata.json`: platform, executable SHA-256 and version output, the example's
  bundled SQLite version, and the labelled query answers. For an executable built
  separately, the example's SQLite version does not identify that executable's
  SQLite version.
- Each scenario's `setup.jsonl`, `queries.jsonl` and `builds.jsonl`: every process
  exit, diagnostic, output, latency and timeout. Non-UTF-8 output remains a byte
  array. Query records indicate whether a writer process was active when the
  query started or finished.
- Each scenario's `summary.json`: successful and failed counts, query latency
  percentiles, observed writer-process overlap, stop reason, elapsed time and
  revision numbers before and after the experiment. The revision difference
  counts publications, including successful writes that overlap a failed query.

Writer-process overlap is evidence of concurrent process lifetimes. It does not
identify exactly when SQLite held a write or checkpoint lock. The deterministic
Store reader tests separately cover reads during a write transaction and
publication with a reader still active.

The experiment deliberately keeps no index connection between queries or builds.
A monitoring connection would change the last-connection WAL cleanup the run
needs to exercise. A failure establishes a reproducible symptom, not its cause.
If a busy failure occurs, retain a temporary instrumentation patch and its
executable identity with the evidence, then capture the failing SQLite operation
and extended result code in a focused comparison before choosing a remedy.
A run with zero failures covers only its bounded workload.

## Connection-setup contention

The initial `synchronous=FULL` pragma reads the schema before a query transaction
begins. That access can meet SQLite's last-connection WAL cleanup, which briefly
takes an exclusive lock on the main file. SQLite documents this source of
[busy errors in WAL mode](https://www.sqlite.org/wal.html#sometimes_queries_return_sqlite_busy_in_wal_mode).

Connection setup uses a 50 ms SQLite busy timeout for its first schema access,
then resets it to zero. This bounds accumulated SQLite retry sleeps for a lock
event. Scheduling and other work can extend elapsed time. Ordinary reader waits
and the separate zero or 60-second writer-lock wait are not affected.

The controlled regression holds a main-file exclusive lock until opening returns
Busy, verifies that the finite wait occurred, then releases it and reads the
published revision. It also confirms that successful setup resets the timeout.
Other tests retain successful reads during an ordinary
write transaction and publication while a streamed reader remains active.

Queries use one committed revision and keep no index connection between
queries. Their query-triggered updates keep a zero writer-lock wait, and an
explicit index build keeps its 60-second writer-lock wait. A lock that outlasts
the short opening allowance produces the usual busy error. Missing, unpublished,
damaged and inaccessible indexes keep their usual failure paths. WAL recovery and the checkpoint policy stay enabled.

On Unix, SQLite can also unlink a WAL or shared-memory file between the safety
check's no-follow open and its descriptor check. A regular descriptor with zero
links is detached from that path. The safety check closes it and checks the path
again, with at most three total open/check attempts per companion. An absent
path is allowed, but a replacement must still be a regular file with one name.
Symbolic links, hard links and other file types are refused. Repeated unlink
contention produces the usual busy error rather than an unsafe-file error.
Permission repair uses only a descriptor that passed those checks.

Source fixtures, index files and records remain in the evidence directory for
inspection. Remove that directory once the evidence is no longer needed.

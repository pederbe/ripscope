# Replay frozen C navigation workflows

The `assess-workflows` Rust example replays independently prepared source edits,
file selection, persistent queries and real editor jumps on disposable copies.
It records operational outcomes and checks the declared expectations. Navigation
accuracy needs an independently labelled Assessed scope, as described in
[ADR 0002](../docs/adr/0002-measured-compatibility.md) and [GLOSSARY.md](../GLOSSARY.md). Use `assess-navigation` for source accuracy scoring.

Build the example in the checkout whose adapters will be used:

```sh
cargo build --locked --example assess-workflows
target/debug/examples/assess-workflows --frame /absolute/frozen-frame \
  --project project-name --backend /absolute/ripscope --repo /absolute/checkout \
  --work-root /absolute/local-disk-directory --output /absolute/new-results \
  --mode cli
```

All seven options are required. `--mode` accepts `cli`, `vim`, `neovim` or `emacs`.
Editor modes also require `--editor /absolute/editor-executable`. Windows uses the
native `.exe` with the same options. Run as an ordinary user, reserve at least
4 GiB free space and use local disk storage. Windows requires NTFS. Linux rejects
tmpfs and the identified NFS/SMB filesystems. The product's index storage checks
also apply. Existing output directories are rejected without modification.

## Frozen inputs

The input is a sealed workflow inventory version 3. `SEAL.json` names and hashes
the input hash list. That list identifies `workflow-inventory.json`, project plans,
the added-file plan and the native editor scripts. Project records declare the
relative source paths, canonical targets, link observations and byte hashes.
Replay supports the declared frames without traversed symbolic links.
Missing, changed, escaping or unlisted sources stop before launching a backend.
The project plan supplies ordered include directories, explicit index files,
expected membership, the exact original and edited source hashes, and independently
written neighbor/call expectations. Include order must agree with the inventory.

Keep the frame and result tree outside the source checkout. Prepare it before
observing product output. Never revise an expectation to match a capture. The
driver checks metadata and original source identities before copying. It checks
the resulting source inventory and exact edited bytes before deleting a successful
runtime copy. Backend, driver, plan, scripts and adapters have recorded hashes.
Retain the source revision separately for the product, driver and adapter checkout.
An externally supplied executable's source revision need not equal the replay checkout.

## CLI and editor observations

CLI mode runs explicit file-list and recursive discovery workflows sequentially
on separate source copies. Each keeps one JSON session through the head edit,
Refresh, no-change Refresh, empty/invalid queries, live source deletion and repair,
and addition/deletion of a C seed and its literal include. Stored queries and live
text queries remain separate before Refresh. The transcript preserves requests,
rows, diagnostics and process identity, and raw stdout and stderr remain byte captures.
Only path separators are normalized when comparing platform paths.

Completed Refresh stages also record the public `--verify -d` output identifying
the committed revision. Read-mode verification deliberately follows completed
Refresh stages and is not invoked during edited/deleted before-Refresh observations.
Verification without `-d` can rebuild the index. A no-change comparison
uses the observable revision number in that output. This is separate from checking
that the same session produced the same ordered answer.

Editor mode loads the frame's script through the shipped adapter and exercises
actual jump, buffer edit, save, index update and diagnostic interfaces. Configuration
and home paths are isolated, and inherited `RIPSCOPE_WORKFLOW_VALIDATE_ONLY` is removed.
Completion requires concrete jump and diagnostic observations and the exact edited
source hash. Query recordings identify field and pattern as well as rows. The
stored definition must have the frozen file, physical line and name, with source
text differing only in whitespace introduced by the documented stored rendering.
The repaired live row must have the exact edited physical line and live-text context.
A warning with rows must retain a declared warning and at least one source-grounded
readable row. Other returned rows remain unassessed. Vim and Emacs scripts record
canonical row payloads alongside their native quickfix/buffer evidence. Neovim
records one session identity and its raw JSON rows. An empty
successful editor report fails validation. The broader synthetic diagnostic suites
remain separate. Their commands are in [tests/editor/README.md](../tests/editor/README.md).
The Emacs diagnostic example accepts `--repo DIR` when a cross-built executable is
run from a different checkout. Its default is the build-time checkout.

## Bounds, cleanup and evidence

Commands use file-backed captures with a combined 2 GiB stream cap and 2 GiB guards
on the runtime/result trees. Index builds have an 1800-second deadline, queries
120 seconds, Vim/Neovim processes 30 seconds and Emacs 90 seconds. The replay has
a 3600-second execution deadline, and cleanup has its own bounded reporting period.
Limits and native storage/user observations are recorded in `report.json`.

Exit status and descendant cleanup are separate. Linux uses an 8 GiB user systemd
scope with swap disabled where available. Cleanup requires a terminal launcher,
no live original process-group member, inactive/failed scope state and a complete
bounded inspection of the unique scope subtree, including descendant cgroups.
A collected scope is inspected through the ordinary user's manager hierarchy.
This assumes matching manager/hierarchy visibility, unique unit names, ordinary
trusted commands and no deliberate cgroup migration or external task injection.
Without that proof, process-group observation alone reports unknown cleanup.

Windows owns a suspended native launch, a process wrapper and a separate proof Job
with kill-on-close and no breakaway. Native parent status never proves descendant
completion. Confirmation requires terminal launch ownership, accounted startup
failures and successful empty proof membership after explicit termination. A pending
launcher always reports unknown even when the Job is empty. Deadline reporting does
not join an unresolved launcher. Late-child cancellation and kill-on-close retain
ownership. Membership errors also remain unknown. There is no Windows per-process
memory cap in this helper, so record the native environment's memory limit separately.
Run the native early-exit, descendant, detach, delayed-launch, failure and deadline
fixtures before relying on Windows cleanup confirmation.

`report.json` preserves each stage and its `expectation_match`, public revision
observations, cleanup state, runtime location and failure reason. `complete` means
the evidence was captured through the required operational stages. CLI expectation
mismatches remain measured outcomes under `expectations_match: false`.
Malformed responses, failed editor expectations, changed inputs, limit failures
and unconfirmed cleanup remain incomplete. Failed runtime copies are retained.
Successful copies are removed only after confirmed cleanup and source checks.
Closed index, WAL and SHM hashes are retained first. Preserve raw captures and
failed attempts separately from corrected runs.

The independent native fixture executable is development tooling. Focused tests
use hand-written C facts and protocol/editor answers through the same command and
driver interfaces, without calling the production analyzer:

```sh
cargo test --locked --example assess-workflows -- --test-threads=1
```

For exported native Windows test executables, place `workflow-fixture.exe` and
`assess-workflows.exe` beside the test executable, and set `WORKFLOW_FIXTURE_REPO`
to the native checkout. The tests then reuse those binaries without guest Cargo.
Capture `--nocapture` output to retain positive and final Job membership observations.

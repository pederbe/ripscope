# Broader C coverage campaigns

The `coverage-campaign` Rust example acquires pinned C selections and draws a
repeatable source sample. It complements the compatibility compass and the
[source-labelled assessor](navigation-assessment.md). Compatibility with the
reference and correctness against written source are separate measurements
(ADR 0002 and ADR 0003).

Keep working inventories, labels, reports and raw captures outside the repository.
Keep the original compatibility corpus unchanged.

## Inventory and sampling

`coverage-campaign acquire ROSTER OUTPUT` reads a JSON `projects` array. Entries
provide `name`, `family`, `group` (`development` or `reserved`), `url`, and a
`tag_pattern` matching stable release tags. An explicit `release` chooses a
verified tag. When tags are absent, `allow_head` permits a pinned commit only
after checking the upstream release channel. An `archive_url`, `release`, and
fresh `source_directory` support upstream release archives. Archives are hashed,
and checkout revisions and every selected source file are pinned.

`exclude` lists excluded path prefixes. `deep_exclude` lists individual files
ineligible for the independent sample, with reasons recorded in the roster.
Selection includes checked-in generated files and separately records notices,
tests, examples, exclusions and duplicate bytes. Review this classification
before measuring anything: file extensions alone do not establish C language
or first-party ownership. No configuration or compilation is required.

The fixed seed is `ripscope-coverage-2026-10-v1`. The lowest SHA-256 rank chooses
one project per family/group, including acquisition failures. Within each project,
the same rule draws one complete production `.c` and `.h` from the frozen frame:
at most 500 lines, 65,536 bytes, and no declared deep exclusion. Empty
frames stay empty. The draw retains the full eligibility frame. Correct inventory
mistakes before navigation output is inspected, preserving earlier inventory
versions and reasons. Results never justify replacement.

Acquisition snapshots previous metadata in `inventory-versions/` before a resumed
run. Record selection changes and their reasons in the new roster. It runs in an
owned 8 GiB scope and monitors the complete campaign tree against 100 GiB.
Preserve closures and query inventories when regenerating them separately.
Measurement resume checks do not authorize changing an inventory that has already
been measured. An unsupported source encoding produces an explicit assessment
failure rather than silently narrowing the eligibility frame.

## Source expectations

`coverage-campaign inventory OUTPUT` freezes conservative literal include
closures, missing operands, cross-project duplicate bytes, larger-file challenge
candidates, and nine exact-name controls per project. These are derived from
source, not navigation output.

Closure inventory version 3 keeps each normalized discovery spelling separately
from its canonical target, SHA-256 content identity and every traversed symbolic
link, including directory aliases and chained links. Retargeting to equal bytes,
changing a link's spelling, changing its target's content or removing its target
invalidates the frozen frame. The method identity is version 3. The source draw's
seed and declared project selection remain unchanged. A frozen inventory cannot
be regenerated in place: use a new evidence root and retain the previous results.

The `project-only` input policy records literal operands that escape the declared
project, including absolute host paths and external aliases, as `unsupported_inputs`.
It admits no host header bytes. Measurement repeats a conservative literal-include safety pass
before launching any index command, also for legacy frames without that metadata.
The pass follows local aliases and catches directives in the middle of a line.
Include-looking text in comments or strings can also stop it. Such an outcome is
an assessment limitation, with no project comparison denominator. A separate explicitly declared and hash-pinned header
selection would need a new inventory and method before those comparisons could
be completed.

Supply independently audited `source-facts/NAME.json` for each deep selection.
Use the assessor's definition/call label format, `contexts` for every written
function and macro including empty bodies, and optional bounded `extra_queries`
for symbols and assignments. Unresolved labels need explicit `unscored_queries`
triples `[field, pattern, reason]`. They are captured without accuracy scoring.

`coverage-campaign prepare OUTPUT` combines those facts with the complete index
selection. It generates definition queries for all written ASCII words, including
comment/string words as conservative controls, all call contexts, absent-name
controls, literal includers, text fields 4/5, a regular expression, and the complete
file inventory. Seal and audit the resulting manifests before inspecting output.
Generated expectations for includes also need review for directives inside
comments and unusual preprocessing syntax.

`index_files` and `include_directories` describe the complete project being
indexed. `files` describes only the independently assessed files. `closure_files`
pins additional discovered inputs. Legacy manifests without those optional keys
retain their original selection and query inventories. All source hashes are
checked before and after queries. A newly discovered unpinned input stops accuracy
measurement until a new manifest and method version are recorded.

## Measurement and evidence

The Linux campaign driver reads `identities.json` with absolute `binary`,
`reference` paths and their frozen SHA-256 identities. An optional, nonempty
`pilot_projects` array chooses the pilot from development projects in the frozen
roster. Names must be unique and known. Reserved projects cannot become pilots.
The identity file is hashed with the method, so changing pilots invalidates frozen
measurement and resume identities without changing the acquired source inventory.
When the field is absent, the pilot projects are memcached, cJSON,
libgit2, TinyUSB, MicroPython and WinFsp. Run `measure OUTPUT pilot`,
then `freeze OUTPUT` after evaluating the pilot and sealing all source labels. Run `measure OUTPUT
development`, followed by `measure OUTPUT reserved`. Pilot projects are excluded
from the subsequent development phase. `deep OUTPUT PHASE`
runs sealed manifests in the same phases. Preserve labels for reserved projects
before viewing their navigation results, and make no intervening product fixes.

The driver reuses the [compass-scale example](compass-scale.md) for seven stored
fields, scanner records, file discovery, broad queries and exact controls, and the
`assess-navigation` example for deep runs. Build all three in release mode first
(`cargo build --release --locked --example coverage-campaign --example compass-scale
--example assess-navigation`). Their executables are part of the method identity. It
requires native Linux, GNU sort, `/usr/bin/time`, user systemd scopes and the pinned
reference from ADR 0001. Windows repeats use the same sealed source selections and the
portable assessor. Record Windows as unmeasured when a suitable environment is
unavailable.

Only one heavy command runs at a time. Limits are 1,800 seconds per index build,
900 seconds per broad query, 120 seconds per exact query, 8 GiB per process scope,
2 GiB per command capture, 20 GiB per project's generated artifacts and 100 GiB
campaign scratch. The driver reserves 24 GiB before starting a project, caps raw
measurement trees at 18 GiB, and captures compressed archives through the 2 GiB
output cap while monitoring campaign scratch. This leaves space for both copies
and command metadata. Breaches retain incomplete evidence rather than an archive.
Deep queries use fixed field/pattern order and a 5,000-query limit. Limit breaches,
malformed streams and reference failures remain explicit incomplete measurements.
Partial captures cannot enter usable-reference agreement denominators.
Unsupported-input outcomes retain `complete: false`, `run_finished: false` and
`comparison_eligible: false`, with no index or query commands. Reports preserve
their operands and reasons and exclude them from scanner, discovery and query
agreement counts. A completed run still admits each query comparison only when
both complete streams are usable. Ordinary full-tree and pinned system-header
compass workflows keep their own source roots. The campaign safety pass applies to
explicit project manifests.

`assess-challenges OUTPUT` runs separately audited range manifests from
`challenges/NAME/manifest.json` for the six-project pilot's five additional
source challenges. It uses the same resource guards, retains compressed raw
captures, and removes disposable indexes. Every query needs its declared range
in `scopes`. Challenge results stay separate from the complete-file sample.
`report-coverage OUTPUT` combines finalized summaries into `report.json`, keeping
per-project contributions, reference eligibility and source accuracy separate.
Provide `windows.json` with the matching selections and their measured or
unmeasured outcomes. Keep working assessment prose and findings with the other
working evidence.

Per-project summaries survive compressed evidence archives. Disposable product
indexes are hashed before removal. Raw commands, stdout/stderr, source identities,
query expectations and scored rows remain. Resume only when the frozen identities
and method still match. Retained summaries prevent repeating finalized projects.
An unfinished directory requires inspection before starting another attempt.

Reports must retain project-level denominators as well as aggregate row counts,
source accuracy by field/construct, unscored and unassessed rows, reference
usability, operational warnings and missing inputs. Challenge results remain
separate from the deterministic file sample. Reduce confirmed defects to C
fixtures with neighboring controls and create separate product-fix tickets.

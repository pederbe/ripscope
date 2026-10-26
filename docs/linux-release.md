# Linux release preparation

The `linux` command of the release tool in `tools/release` builds an x86-64 archive
against glibc 2.28 with the Rust, Mold and image identities in
`tools/release/linux-release-profile.json`. It records the exact source, dependency and
tool inputs, runs locked offline product release tests, inspects ELF imports, and
smokes the extracted archive. Initial vendoring and image acquisition can use the
network; the build and runtime containers cannot. This prepares local files without
publishing them.

Use Git, binutils (`readelf`), GNU tar and GNU timeout, rootless Podman, the pinned Rust 1.99.1
toolchain and the Mold 3.0.0 archive, plus Rust 1.98 or later to build the release
tool itself. GNU timeout is a prerequisite of the reference and release tests, not
of the installed Ripscope executable. The tool uses the default `timeout` when its
version identifies GNU coreutils, otherwise `gnutimeout` (provided by Ubuntu's GNU
coreutils package). It records the selected executable and version, then stages it
as `timeout` only on its native test processes' PATH, including nested deadlines.
Other timeout providers are refused because their descendant cleanup can differ.
Native developer-proof tests also require disk-backed `/var/tmp` with at
least 4 GiB free, a reachable user systemd manager, and a visible unified cgroup
hierarchy under that manager. The tool checks those prerequisites without recording
the manager's environment. It does not expose the host's user bus, PID namespace or
cgroup controls to a container. Download Mold from the profile's `mold_url`; the tool
checks `mold_archive_sha256` before extraction. Select a new external work directory:

```sh
cargo run --locked --manifest-path tools/release/Cargo.toml -- linux \
  --work /var/tmp/ripscope-linux-candidate \
  --toolchain "$HOME/.rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu" \
  --mold-archive /path/to/mold-3.0.0-x86_64-linux.tar.gz
```

The tool preserves logs and `receipt.json` there. The archive contains the
executable, installation guidance, `BUILD.json`, `MANIFEST.sha256`, dependency
notices, Rust and SQLite notices, and editor integration sources.
`dist/` holds the archive, its `.sha256` sidecar and `SHA256SUMS`.

The pinned container builds the artifact and runs release tests for the product
binary and integration targets. All auxiliary examples are separately built and
tested in native debug mode, using the same copied source, vendor tree and pinned
Rust toolchain. The native tests include ignored checks and run in uniquely named
user scopes. The receipt labels the two phases separately; native proof coverage
is not artifact-baseline coverage. Both generated Cargo configurations are saved
and hashed, and the container-built executable must remain byte-identical.

Build/test commands have 30-minute deadlines, with bounded cleanup of their
exact owned container or outer user scope. Any failure, timeout or unresolved
cleanup leaves validation incomplete. Failed build/test phases stop before archive
assembly. Auxiliary tests
also create separate proof scopes: stopping the outer scope alone cannot prove
those scopes terminated after a hard timeout. Such captures remain observations;
preserve the work and resolve the exact surviving units/processes before retrying
or removing it. No unrelated matching units or processes are terminated. An
interrupt stops the tool at once without recording cleanup; the terminal's interrupt
also reaches the running container or scope, and the receipt's last command names
the `ripscope-release-*` container or unit to check before removing the work.

The ELF gate rejects required glibc symbols above 2.28, foreign shared libraries,
build-path RPATH/RUNPATH, a different interpreter, a missing pinned Mold marker,
and advertised CPU requirements above baseline x86-64. It does not certify every
instruction or old kernel. Runtime smoke uses the archive's exact executable
with no compiler or SQLite tool on PATH. It covers LF and CRLF, stored queries
after source deletion, available live rows with warnings, changed source,
session `r` and an unchanged Refresh that preserves the index revision.

Stable archive ownership and timestamps do not promise identical recompilation.
Keep the recorded source and tool inputs when repeating a build.

## Debian and RPM packages

Build both packages from the already inspected archive directory:

```sh
bash scripts/package-linux.sh 0.1.0 \
  /var/tmp/ripscope-linux-candidate/ripscope-0.1.0-x86_64-linux-gnu \
  /var/tmp/ripscope-linux-candidate/dist
```

This requires `dpkg-deb`, `rpmbuild`, `rpm2cpio` and `cpio`. Missing RPM tools fail
the command. Both extracted packages must contain the byte-identical executable
and complete notices/editor source. No stripping or recompilation is allowed.
The declared dependencies are glibc 2.28 or newer and the system GCC runtime.
Install, query and remove each package in its selected offline runtime before
accepting it; leave a project index and unrelated file in place to check removal.
The prepared workflow runs these checks on Debian 12 and AlmaLinux 8.

## Installer checks

Exercise the script against a loopback mirror of the exact archive:

```sh
cargo run --locked --example check-installers -- \
  --archive /var/tmp/ripscope-linux-candidate/dist/ripscope-0.1.0-x86_64-linux-gnu.tar.gz \
  --installer "$PWD/install.sh" --work /var/tmp/ripscope-linux-installer-check
```

The work directory must be new. Captures retain installer/archive hashes and
outcomes for replacement, checksum/path/link refusals, mandatory notices,
interruption after rename, the editors' `--plugin` install decisions, and selective
removal. These are functional checks;
they do not measure performance or establish broad platform qualification.

## Weekly releases

Every Monday at 06:43 UTC, `.github/workflows/weekly-release.yml` checks for
unreleased code, dependency, editor or packaging changes. Documentation and website
changes alone do not create a release. The workflow remains enabled until disabled
in GitHub Actions. You can also dispatch it from `main` for an urgent release.

The workflow prepares a new patch version by default. Commit subjects with `!`
before their colon, or bodies containing `BREAKING CHANGE:` or `BREAKING-CHANGE:`,
advance the minor version before 1.0 and the major version after 1.0. Feature commits
advance the minor version after 1.0. Mark compatibility changes when merging their
pull request. A manual dispatch can request a minor or major increment. An explicitly
prepared newer version in `Cargo.toml` takes precedence.

Release metadata lives on the version tag. Its commit changes `Cargo.toml`,
`Cargo.lock`, the Emacs version header and `CHANGELOG.md`, leaving the development
version on `main` unchanged. Each changelog carries all previous release sections,
new authored entries and grouped commit summaries with comparison links. The next
release compares the two development snapshots, excluding generated metadata.

The scheduler dispatches `.github/workflows/release.yml` at the exact tag and source
SHA. That workflow builds, tests, inspects and packages both platform archives before
attesting their provenance and publishing. Failed runs keep their tag. The next
weekly or manual run resumes that same version before preparing another. If a
failure needs a source or workflow fix, merge the fix and manually dispatch with
`supersede_failed=true`. Once the old run has stopped, this prepares a higher version
from current `main` and preserves the abandoned tag and its evidence. Successful
build jobs are retained when retrying failed jobs. Releases are serialized and an
older tag cannot replace a newer published version or its editor branch.

Automatic publication uses the `automated-release` environment with no approval or
wait rules and exactly one custom deployment policy, tag `v*`. Its preflight checks
those settings, the public repository, the existing tag and exact dispatch SHA.
Configure the environment once before enabling the schedule. The Windows runner
must provide the pinned MSVC/SDK profile. Linux must satisfy the release tool's disk,
user-manager and cgroup prerequisites. Missing prerequisites stop publication.

Configure the crates.io trusted publisher for owner `pederbe`, repository `ripscope`,
workflow `release.yml` and environment `automated-release`. After both platform jobs
pass, the automatic publish job checks the source package's required files and runs
`cargo publish --dry-run` before requesting a short-lived registry token. It publishes
the crate before making the GitHub release public. Retries accept an existing,
non-yanked crate version only when its registry checksum matches the package from
the frozen release source. Missing registry authorization or a mismatch stops the
release before GitHub publication. Configure the publisher before enabling releases
from a workflow containing this step. Older tags keep their original workflow.

The existing manual release dispatch remains available. With `publish=false` it
only builds test artifacts. With `publish=true` and `automatic=false`, it requires
the original `release` environment's reviewer and creates a draft with the actual
changelog section. Manual drafts do not publish to crates.io.

## Editor plugin branch

Plugin managers follow the `release` branch. After automatic publication, the
workflow moves that branch to the published tag using a lease to protect concurrent
updates. Generated release commits can have sibling histories, so this update can
be non-fast-forward. The Emacs version header and crate version match on every tag.

## Website deployment

After a successful Release run, the separate Website deployment workflow requests
Cloudflare's website build. It follows the delivered `release` branch and skips
stale release completions. Cloudflare reports the actual build and deployment
result after accepting the request.

A website failure does not fail package publication or block the next weekly
release. Retry Website deployment manually in GitHub Actions, without rebuilding
archives or republishing the crate. Weekly release recovery still repairs failed
package publication and an editor branch that did not advance.

## Attestation verification

After an authorized release has generated attestations, verify each downloaded
archive or package against the expected repository, workflow, tag and full source
commit. Replace `FULL_SOURCE_COMMIT` with the independently selected 40-character
commit, and use the matching release filename:

```sh
gh attestation verify ripscope-0.1.0-x86_64-linux-gnu.tar.gz \
  --repo pederbe/ripscope \
  --signer-repo pederbe/ripscope \
  --signer-workflow pederbe/ripscope/.github/workflows/release.yml \
  --signer-digest FULL_SOURCE_COMMIT \
  --source-ref refs/tags/v0.1.0 \
  --source-digest FULL_SOURCE_COMMIT \
  --deny-self-hosted-runners
```

Use the same flags for the `.deb`, `.rpm` and Windows ZIP. A missing or failed
attestation is not successful verification. Checksums alone identify bytes; they
do not prove which repository or workflow built them. Available verification flags
are documented in [the GitHub CLI manual](https://cli.github.com/manual/gh_attestation_verify).

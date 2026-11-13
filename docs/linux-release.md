# Linux release preparation

The `linux` command of the release tool in `tools/release` builds an x86-64 archive
against glibc 2.28 with rolling stable Rust and the Zig, cargo-zigbuild and image identities in
`tools/release/linux-release-profile.json`. It records the actual compiler and
exact source, dependency and tool inputs, runs locked offline product release tests,
inspects ELF imports, and
smokes the extracted archive. Initial vendoring and image acquisition can use the
network. The build and runtime containers cannot. This prepares local files without
publishing them.

Use Git, binutils (`readelf`), GNU tar and GNU timeout, rootless Podman, the stable Rust
toolchain and the Zig and cargo-zigbuild archives identified by the profile. The workflow updates
stable through rustup before building. GNU timeout is a prerequisite of the
reference and release tests, not of the installed Ripscope executable.
The tool uses the default `timeout` when its
version identifies GNU coreutils, otherwise `gnutimeout` (provided by Ubuntu's GNU
coreutils package). It records the selected executable and version, then stages it
as `timeout` only on its native test processes' PATH, including nested deadlines.
Other timeout providers are refused because their descendant cleanup can differ.
Native developer-proof tests also require disk-backed `/var/tmp` with at
least 4 GiB free, a reachable user systemd manager, and a visible unified cgroup
hierarchy under that manager. The tool checks those prerequisites without recording
the manager's environment. It does not expose the host's user bus, PID namespace or
cgroup controls to a container. Download the archives from the profile's `zig_url`
and `zigbuild_url`. The tool checks their SHA-256 identities before extraction.
Select a new external work directory:

```sh
rustup update stable
toolchain=$(dirname "$(dirname "$(rustup which --toolchain stable rustc)")")
env RUSTFLAGS='' cargo run --locked --manifest-path tools/release/Cargo.toml -- linux \
  --work /var/tmp/ripscope-linux-candidate \
  --toolchain "$toolchain" \
  --zig-archive /path/to/zig-archive.tar.xz \
  --zigbuild-archive /path/to/cargo-zigbuild-archive.tar.xz
```

The tool preserves logs and `receipt.json` there. The archive contains the
executable, installation guidance, `BUILD.json`, `MANIFEST.sha256`, dependency
notices, Rust, SQLite and Zig runtime notices, and editor integration sources.
`dist/` holds the archive, its `.sha256` sidecar and `SHA256SUMS`.

The pinned Ubuntu container builds the artifact and runs release tests for the product
binary and integration targets. Zig supplies the C compiler and LLD linker for
baseline x86-64 and the `x86_64-linux-gnu.2.28` target, including bundled SQLite.
Both Cargo phases use the same wrappers and environment. cargo-zigbuild's linker
adapter replaces Rust's GCC unwinder request with Zig's static LLVM libunwind.
The executable therefore needs no shared GCC runtime. Builds use disk-backed
temporary files and caches, and do not depend on an old-glibc build image.
All auxiliary examples are separately built and
tested in native debug mode, using the same copied source, vendor tree and selected
Rust toolchain. The native tests include ignored checks and run in uniquely named
user scopes. The receipt labels the two phases separately. Native proof coverage
is not artifact-baseline coverage. Both generated Cargo configurations are saved
and hashed, and the container-built executable must remain byte-identical.

Build/test commands have 30-minute deadlines, with bounded cleanup of their
exact owned container or outer user scope. Any failure, timeout or unresolved
cleanup leaves validation incomplete. Failed build/test phases stop before archive
assembly. Auxiliary tests
also create separate proof scopes: stopping the outer scope alone cannot prove
those scopes terminated after a hard timeout. Such captures remain observations.
Preserve the work and resolve the exact surviving units/processes before retrying
or removing it. No unrelated matching units or processes are terminated.

An interrupt stops the running command, records the interruption and attempts
the same bounded cleanup. Let the tool return, then inspect `owned_cleanup` in
the receipt before removing work. An interrupted native command still leaves
separate proof scopes unverified. After a forced termination, use the recorded
command's `ripscope-release-*` container or unit identity to inspect what remains.

The ELF gate rejects required glibc symbols above 2.28, foreign shared libraries,
RPATH/RUNPATH, the shared GCC runtime, a different interpreter, a missing LLD marker,
and advertised CPU requirements above baseline x86-64. It does not certify every
instruction or old kernel. A separate pinned Debian 10 container verifies that
its runtime is glibc 2.28, then uses the archive's exact executable
with no compiler or SQLite tool on PATH. It covers LF and CRLF, stored queries
after source deletion, available live rows with warnings, changed source,
session `r` and an unchanged Refresh that preserves the index revision. A safe Rust
probe built with the same cross-linker verifies main-thread and worker-thread
panic unwinding, including destructors, on that runtime.

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
The declared dependency is glibc 2.28 or newer.
Install, query and remove each package in its selected offline runtime before
accepting it. Leave a project index and unrelated file in place to check removal.
The prepared workflow runs these checks on Debian 12 and AlmaLinux 8.

## Signed apt and dnf repository

The repository at `packages.ripscope.com` lets Debian, Ubuntu and RHEL-family users
install Ripscope once and then receive new releases through `apt upgrade` or
`dnf upgrade`. Standalone package downloads remain available for users who do not
want to add a repository.

The [Package repository workflow](https://github.com/pederbe/ripscope/actions/workflows/package-repository.yml)
adds a published release's `.deb` and `.rpm` after verifying their checksums and
attestations. It never rebuilds the executable. Repository publication preserves
these constraints:

1. It checks every `.deb` already in the published repository against
   its release's SHA-256 file, since storage credentials alone must not be able to
   add a package.
2. `package-repository` publishes the `.deb` unchanged, adds a header
   signature to the `.rpm` (checking that its header and payload digests stay the
   same), refuses any file that is not a Ripscope package matching its name, and
   regenerates and signs the apt and dnf metadata.
3. `package-repository-check` installs the release from the generated
   repository in offline Debian 12, Ubuntu 22.04, Ubuntu 24.04 and AlmaLinux 8
   containers with signature checks on. When the release is the newest, it installs
   the previous version first and upgrades with `apt-get upgrade` or `dnf upgrade`. It checks
   the version, every installed file against the release archive's manifest
   (including the editor sources under `/usr/share/ripscope`), indexing and search,
   and that removal leaves the project, its index and the editor configuration. A
   client holding another key must refuse the repository.
4. It uploads packages, apt indexes by hash and complete dnf metadata snapshots, then
   the current apt index and dnf mirrorlist, and compares what is served with what
   it signed.

Every earlier package, index and snapshot stays published. A client that read the
previous generation, or a publication that stopped part way, still finds consistent
signed files. To repeat a publication, run the workflow from its page with the
release tag. That signs and uploads again without changing the release or rebuilding
anything. Runs take turns, so start the next one after the previous has finished.

The workflow's nonpublishing checks use a throwaway key and repeat publication
to verify that existing packages remain unchanged.

The repository holds:

```text
ripscope.asc, ripscope.gpg   the signing key, armored and binary
ripscope.repo                dnf repository definition
apt/dists/stable/            InRelease, Release, Release.gpg and the Packages indexes
apt/pool/main/r/ripscope/    every published .deb
rpm/                         every published .rpm, signed
rpm/snapshots/<sha256>/      permanent dnf metadata, with repomd.xml.asc
rpm/mirrorlist               URL of the current dnf snapshot
```

### Signing and live verification

The committed public key is `tools/release/package-signing-key.asc`. Publication
requires the signing key and bucket-scoped storage credentials used by the
[workflow](../.github/workflows/package-repository.yml). RSA signing preserves
AlmaLinux 8 and RHEL 8 compatibility. Clients retain their own copy of the key,
so key expiry affects their ability to receive updates. Keep an offline backup
of the secret key and its revocation certificate outside the publishing environment.

After publication, dispatch the same workflow with `verify_only=true` and the
latest tag. It reuses the published packages and checks installation, upgrade,
search and removal through the live HTTPS endpoint on all four clients, without
signing or uploading. The default pre-publication checks remain offline.

The [user installation guide](guide/installation-options.md#apt-and-dnf-repositories)
owns repository setup commands and the signing-key fingerprint.

### Checking locally

`package-repository` needs gpg, gpgv, `apt-ftparchive`, `rpm`, `rpmsign` and
`createrepo_c`, as in Debian's `apt-utils`, `rpm`, `createrepo-c`, `gnupg` and
`gpgv` packages. `package-repository-check` needs gpg and Podman (or set
`CONTAINER_ENGINE=docker`). With a throwaway key in `GNUPGHOME`, its public half in
`test-key.asc`, and two releases' packages and the newer one's extracted archive in
the current directory:

```sh
release=tools/release/target/debug/ripscope-release
env RUSTFLAGS='' cargo build --locked --manifest-path tools/release/Cargo.toml
$release package-repository repo test-key.asc ripscope_0.2.3_amd64.deb ripscope-0.2.3-1.x86_64.rpm
$release package-repository repo test-key.asc ripscope_0.2.4_amd64.deb ripscope-0.2.4-1.x86_64.rpm
$release package-repository-check repo ripscope-0.2.4-x86_64-linux-gnu check
```

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
removal. These functional checks do not measure performance or establish broad
platform qualification.

### Installer replacement

The shell installer needs curl, GNU tar, gzip and GNU coreutils. It verifies the
archive and stages the new executable on the destination filesystem before
renaming it into place. Directory and symbolic-link executable destinations are
refused. An already running Linux process can finish using its old executable.

A successful installation retains a `.ripscope-notices-*` directory beside the
executable. An interrupted replacement can retain it too, because the executable
may already have changed. Its
`ORIGINAL-ARCHIVE-MANIFEST.sha256` describes the archive before the executable
moved. `BUILD.json` and `INSTALLATION.txt` identify the installed executable by
SHA-256. Earlier notices directories are not pruned during an update.

## Weekly releases

The [release preparation workflow](../.github/workflows/release-plz.yml)
generates the version, dated changelog and matching Emacs header in a release
pull request. Notes come from merged pull request titles, using
[git-cliff rules](../cliff.toml) over the complete release range, including
changes outside the Cargo package. Keep Unreleased empty. Preparation and
publication both check notes against the release source.

The [Release workflow](../.github/workflows/release.yml) builds and qualifies
the exact tagged commit before publishing. Its explicit dispatch is needed
because a workflow-token tag push does not start another workflow.

Retry a failed Release run's failed jobs to keep successful builds and the original
tag. If tag creation succeeded but dispatch failed, dispatch Release with that tag,
its resolved commit as `source_sha`, and `publish=true` and `automatic=true`.
Package, WinGet and website delivery each have their own workflow retry. If a source
fix is needed, merge it and prepare a higher version through a new release pull
request, preserving the old tag. Releases are serialized and an older tag cannot
replace a newer published version or its editor branch.

Publication preflight checks the protected environment, public repository,
existing tag and exact dispatch SHA. Each platform must satisfy its native
build prerequisites. Missing prerequisites stop publication.

Automatic publication uses crates.io trusted publishing. It checks the source
package and runs `cargo publish --dry-run` before requesting a short-lived
registry token. The crate is published before the GitHub release becomes public.
Retries accept an existing, non-yanked crate version only when its registry
checksum matches the package from the frozen release source. Missing registry
authorization or a mismatch stops GitHub publication. Older tags keep their
original workflow.

The existing manual release dispatch remains available. With `publish=false` it
only builds test artifacts. With `publish=true` and `automatic=false`, it requires
the original `release` environment's reviewer and creates a draft with the actual
changelog section. Manual drafts do not publish to crates.io.

## Editor plugin branch

Plugin managers follow the `release` branch. After automatic publication, the
workflow moves that branch to the published tag using a lease to protect concurrent
updates. Older release tags can have sibling histories, so this update can be
non-fast-forward. The Emacs version header and crate version match on every tag.

## Website deployment

Website delivery is independent of package publication. Follow the
[site-tool guide](../tools/site/README.md#combined-build) to retry it without
rebuilding archives or republishing the crate. Retry Release if publication or
the editor branch update failed.

## Attestation verification

After an authorized release has generated attestations, verify each downloaded
archive or package against the expected repository, workflow, tag and full source
commit. Replace `FULL_SOURCE_COMMIT` with the independently selected 40-character
commit, and use the matching release filename:

```sh
gh attestation verify ripscope-0.1.0-x86_64-linux-gnu.tar.gz \
  --repo pederbe/ripscope \
  --signer-workflow pederbe/ripscope/.github/workflows/release.yml \
  --signer-digest FULL_SOURCE_COMMIT \
  --source-ref refs/tags/v0.1.0 \
  --source-digest FULL_SOURCE_COMMIT \
  --deny-self-hosted-runners
```

Use the same flags for the `.deb`, `.rpm` and Windows ZIP. A missing or failed
attestation is not successful verification. Checksums alone identify bytes. They
do not prove which repository or workflow built them. Available verification flags
are documented in [the GitHub CLI manual](https://cli.github.com/manual/gh_attestation_verify).

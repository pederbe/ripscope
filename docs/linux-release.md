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

## Signed apt and dnf repository

The repository at `packages.ripscope.com` lets Debian, Ubuntu and RHEL-family users
install Ripscope once and then receive new releases through `apt upgrade` or
`dnf upgrade`. It is not published yet, so the user guides still describe only the
downloaded packages. Those downloads remain available either way.

After each published release, the
[Package repository workflow](https://github.com/pederbe/ripscope/actions/workflows/package-repository.yml)
adds the release's `.deb` and `.rpm` to it. The workflow takes these steps.

1. It downloads the packages, checks their SHA-256 files and their attestations from
   `release.yml`, and never rebuilds the executable.
2. It fetches the published repository and checks every `.deb` already there against
   its release's SHA-256 file, since storage credentials alone must not be able to
   add a package.
3. It runs `package-repository`, which publishes the `.deb` unchanged, adds a header
   signature to the `.rpm` (checking that its header and payload digests stay the
   same), refuses any file that is not a Ripscope package matching its name, and
   regenerates and signs the apt and dnf metadata.
4. It runs `package-repository-check`, which installs the release from the generated
   repository in offline Debian 12, Ubuntu 22.04, Ubuntu 24.04 and AlmaLinux 8
   containers with signature checks on. When the release is the newest, it installs
   the previous version first and upgrades with `apt-get upgrade` or `dnf upgrade`. It checks
   the version, every installed file against the release archive's manifest
   (including the editor sources under `/usr/share/ripscope`), indexing and search,
   and that removal leaves the project, its index and the editor configuration. A
   client holding another key must refuse the repository.
5. It uploads packages, apt indexes by hash and complete dnf metadata snapshots, then
   the current apt index and dnf mirrorlist, and compares what is served with what
   it signed.

Every earlier package, index and snapshot stays published. A client that read the
previous generation, or a publication that stopped part way, still finds consistent
signed files. To repeat a publication, run the workflow from its page with the
release tag. That signs and uploads again without changing the release or rebuilding
anything. Runs take turns, so start the next one after the previous has finished.

Pull requests that change the workflow or `tools/release/src/repository.rs` run
steps 1, 3 and 4 on the two latest releases with a throwaway key, publish the newest
twice to check that a repeat keeps every package, and upload nothing.

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

### One-time setup

Publication stays off until the signing key's public half is committed as
`tools/release/package-signing-key.asc`. Until then each run reports that the
repository is not set up and succeeds.

The package signing key has fingerprint
`3710 BE46 B974 AAB7 390D 225B FAFC E829 50BF F0D6`.
The `ripscope-packages` bucket uses the existing Cloudflare account, Standard
storage and `packages.ripscope.com`, with TLS 1.2 or later and public `r2.dev`
access disabled. The `packages` GitHub environment permits only `main` and `v*`
tags and has no approval or wait rule. Storage credentials and the first live
publication must be verified before this channel is advertised to users.

After publication, dispatch the same workflow with `verify_only=true` and the
latest tag. It reuses the published packages and checks installation, upgrade,
search and removal through the live HTTPS endpoint on all four clients, without
signing or uploading. The default pre-publication checks remain offline.

1. **Signing key.** Create a dedicated RSA 4096 key with no expiry. RSA keeps
   AlmaLinux 8 and RHEL 8 able to check RPM signatures. An expiry would strand apt
   users, who keep their own copy of the key. Keep an offline backup of the secret
   key and its revocation certificate (`openpgp-revocs.d/` in the GnuPG home), which
   is how the key is retired if it is ever exposed.
2. **Storage.** In [Cloudflare R2](https://dash.cloudflare.com/?to=/:account/r2/overview),
   create the bucket `ripscope-packages` with Standard storage. Connect the custom
   domain `packages.ripscope.com` with TLS 1.2 or later and leave public `r2.dev`
   access off. Then
   [create an R2 API token](https://dash.cloudflare.com/?to=/:account/r2/api-tokens)
   with Object Read & Write access to that bucket only. Ripscope's packages are a
   few MiB per release, well inside the
   [free tier](https://developers.cloudflare.com/r2/pricing/).
3. **Environment.** In the
   [repository's environments](https://github.com/pederbe/ripscope/settings/environments),
   create `packages` without required reviewers, limited to the `main` branch and
   `v*` tags, with these secrets:

   | Secret | Value |
   | --- | --- |
   | `PACKAGE_SIGNING_KEY` | the armored secret key |
   | `PACKAGE_SIGNING_PASSPHRASE` | its passphrase |
   | `R2_ACCOUNT_ID` | the Cloudflare account ID |
   | `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | the R2 token's S3 credentials |

4. **Key and first publication.** Commit the public key through a pull request and
   check its fingerprint with `gpg --show-keys --fingerprint
   tools/release/package-signing-key.asc`. Run the workflow for the previous release,
   then for the latest, so the newest is published last. From a clean Debian,
   Ubuntu and AlmaLinux machine, install through `https://packages.ripscope.com`,
   upgrade and remove with signature checks on. Only then add the setup below and
   the key fingerprint to the user guides.

### For users, once published

```sh
# Debian and Ubuntu: check that the fingerprint matches the one in the guide
curl -fsSLo ripscope.gpg https://packages.ripscope.com/ripscope.gpg
gpg --show-keys --fingerprint ripscope.gpg
sudo install -m 644 ripscope.gpg /usr/share/keyrings/ripscope.gpg
echo "deb [signed-by=/usr/share/keyrings/ripscope.gpg] https://packages.ripscope.com/apt stable main" |
  sudo tee /etc/apt/sources.list.d/ripscope.list
sudo apt update && sudo apt install ripscope

# RHEL, AlmaLinux and Fedora
sudo curl -fsSLo /etc/yum.repos.d/ripscope.repo https://packages.ripscope.com/ripscope.repo
sudo dnf install ripscope
```

dnf shows the key's fingerprint and asks for confirmation the first time. To stop
receiving updates, remove the package, then `/etc/apt/sources.list.d/ripscope.list`
and `/usr/share/keyrings/ripscope.gpg`, or `/etc/yum.repos.d/ripscope.repo`.

### Checking locally

`package-repository` needs gpg, gpgv, `apt-ftparchive`, `rpm`, `rpmsign` and
`createrepo_c`, as in Debian's `apt-utils`, `rpm`, `createrepo-c`, `gnupg` and
`gpgv` packages. `package-repository-check` needs gpg and Podman (or set
`CONTAINER_ENGINE=docker`). With a throwaway key in `GNUPGHOME`, its public half in
`test-key.asc`, and two releases' packages and the newer one's extracted archive in
the current directory:

```sh
release=tools/release/target/debug/ripscope-release
cargo build --locked --manifest-path tools/release/Cargo.toml
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

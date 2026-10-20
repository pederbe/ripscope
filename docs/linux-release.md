# Linux release preparation

The `linux` command of the release tool in `tools/release` builds an x86-64 archive
against glibc 2.28 with the Rust, Mold and image identities in
`tools/release/linux-release-profile.json`. It records the exact source, dependency and
tool inputs, runs locked offline product release tests, inspects ELF imports, and
smokes the extracted archive. Initial vendoring and image acquisition can use the
network; the build and runtime containers cannot. This prepares local files without
publishing them.

Use Git, binutils (`readelf`), GNU tar, rootless Podman, the pinned Rust 1.98.1
toolchain and the Mold 2.40.4 archive, plus Rust 1.98 or later to build the release
tool itself. Native developer-proof tests also require disk-backed `/var/tmp` with at
least 4 GiB free, a reachable user systemd manager, and a visible unified cgroup
hierarchy under that manager. The tool checks those prerequisites without recording
the manager's environment. It does not expose the host's user bus, PID namespace or
cgroup controls to a container. Download Mold from the profile's `mold_url`; the tool
checks `mold_archive_sha256` before extraction. Select a new external work directory:

```sh
cargo run --locked --manifest-path tools/release/Cargo.toml -- linux \
  --work /var/tmp/ripscope-linux-candidate \
  --toolchain "$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu" \
  --mold-archive /path/to/mold-2.40.4-x86_64-linux.tar.gz
```

The tool preserves logs and `receipt.json` there. The archive contains the
executable, installation guidance, `BUILD.json`, `MANIFEST.sha256`, dependency
notices, Rust and SQLite notices, and editor integration sources. The Emacs
integration includes separately licensed xcscope source and `COPYING` unchanged.
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

## Guarded release workflow

`.github/workflows/release.yml` runs only through manual dispatch at an existing
version tag. The supplied full source SHA must equal both that tag and the
workflow's dispatch commit. `publish` defaults to false. Build jobs prepare and
test artifacts without creating a release or attestation.

Before enabling its publication path, configure the fixed `release` environment
with at least one required reviewer. The preflight runs without environment
secrets and fails closed unless the environment API returns that exact name and
valid nonempty reviewers. Publication also requires a public repository and a
successful preflight, then enters the fixed protected environment. The sole
maintainer can permit self-approval; disable administrator bypass manually.
The API check does not prove those dashboard settings. The hosted Windows runner
must supply the exact pinned MSVC/SDK profile or its build fails.
The hosted Linux runner's disk storage, user-manager and cgroup prerequisites
must also be established before use. Local validation does not verify that
hosted setup; the release tool fails closed when its prerequisites are unavailable.

An approved publication run creates attestations and a draft GitHub release.
It does not publish the crate. `Cargo.toml` keeps `publish = false` until the
maintainer explicitly starts and authorizes that separate release action.
GitHub's account and repository requirements must be checked at that point;
private preparation does not establish attestation availability or create any.
See [GitHub's attestation documentation](https://docs.github.com/en/actions/concepts/security/artifact-attestations).

## Editor plugin branch

The editor plugin lines follow the public repository's `release` branch, and their
build step installs the executable whose version the branch's `Cargo.toml` names.
Before tagging, keep the `Version:` header of `contrib/emacs/ripscope.el` equal to
the crate version. After the release is published, move the branch to the release's
tag, so that plugin managers update to it:

```sh
git push origin 'v0.1.0^{commit}:refs/heads/release'
```

The first release creates the branch. Until then the plugin lines find nothing.

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

# Binary distribution preparation

The [Linux release tooling](../docs/linux-release.md#debian-and-rpm-packages)
builds Debian/Ubuntu `.deb` and RHEL-family `.rpm` packages from the checked Linux
glibc archive. It verifies the byte-identical executable and complete
notices/editor sources after package extraction, and writes individual SHA-256
sidecars. The runtime dependency is glibc 2.28 or newer. Older archives also need
the system GCC runtime.
A workflow installs, queries and removes them on Debian 12 and AlmaLinux 8.

The Linux tarball and checksum-verified shell installer are also prepared.
Native Windows uses the Windows ZIP and PowerShell installer, described in
[Windows preparation](../docs/windows-release.md). That page also covers the
[WinGet manifests](../docs/windows-release.md#winget-package), which reuse the same ZIP
and are generated and checked after each release. There are no macOS, ARM64 or musl
builds. [Limitations](../docs/reference/limitations.md#platforms-and-storage) lists
the supported platforms.

Additional recipes repackage the same Linux archive for Arch, a Homebrew tap, Nix
and a conda channel. Generating recipes does not publish a package.

## Prepare additional recipes

Use the latest validated Linux release archive and independently verify its checksum
and, when available, its attestation. The preparation command records the bytes
you provide. It does not check where they came from. It derives the package version
from the archive's filename and runtime dependencies from its `BUILD.json`.
Use a new output directory whose
parent exists:

```sh
cargo run --locked --example prepare-packages -- \
  --archive /path/to/latest-release/ripscope-*-x86_64-linux-gnu.tar.gz \
  --out /path/to/new-recipes
```

The command generates four pinned recipes, their installed checks and
`PREPARATION.json`, containing the archive and generated-file SHA-256 hashes.
It refuses an existing output directory. Keep generated recipes and validation
captures outside the source tree. Generate again in a new directory whenever
the archive, version or recipe templates change. Never reuse an earlier hash
for a rebuilt archive. No tag, release, Cargo publication or channel is changed.

The release workflow runs the same preparation against its newly built Linux
archive. It adds `ripscope-VERSION-arch-recipe.tar.gz` and its `.sha256` sidecar
to the release assets. The bundle contains `aur/PKGBUILD` and
`aur/check-installed.sh`, ready for `makepkg --check`. It receives the same
provenance attestation as the other release archives.

Each package retains the full archive below its private installation prefix and
exposes one command on PATH. The retained directory contains `BUILD.json`, the
file manifest, dependency/Rust/SQLite notices and editor sources (ADR 0014,
"Ripscope ships its own editor integrations, set up in one step"). A package does
not set up editors. The install scripts and plugin managers do.

| Route | Installed support directory | Removal |
| --- | --- | --- |
| Arch `ripscope-bin` | `/usr/lib/ripscope` | `pacman -R ripscope-bin` |
| Homebrew tap formula | Formula's `libexec/ripscope` (`brew --prefix ripscope` plus `/libexec/ripscope`) | `brew uninstall ripscope` |
| Nix `ripscope-bin` | Output's `lib/ripscope` | Remove its entry with `nix profile remove`. Old generations/store objects can remain |
| conda `ripscope` | Environment's `lib/ripscope` | `conda remove ripscope` |

These names describe generated recipes, not published installation commands.
Project indexes and unrelated files are outside package ownership.

## Validate before submission

Use a disposable environment for each package manager. The recipes' checks use
the installed public CLI, exact expected Result rows, stored queries after source
deletion, live-text diagnostics and Refresh. The installed manifest and mandatory
licences/editor sources must pass too. No compiler, external SQLite or cscope
is required at runtime. Shell and core utilities used by package tests are test
dependencies, rather than Ripscope runtime requirements.

- **Arch:** in the generated `aur/` directory, run `makepkg --check`. It downloads
  the release archive, verifies both source checksums and runs `check-installed.sh`
  before packaging. Install the resulting package with `pacman -U` in a disposable
  Arch environment, repeat the check against `/usr/bin/ripscope` and
  `/usr/lib/ripscope`, then remove it with `pacman -R ripscope-bin`.
- **Homebrew:** put `homebrew/ripscope.rb` in a local tap's `Formula/` directory.
  Use `brew install --build-from-source OWNER/LOCAL-TAP/ripscope` (this installs
  the upstream binary and does not compile Rust), then `brew test` and the tap's
  applicable `brew audit`/`brew style` checks. For local validation, change only
  the URL in a temporary recipe copy to a local mirror of the exact archive.
  The formula retains the full archive in `libexec/ripscope`. For older archives
  that need GCC, it also declares Homebrew's GCC package and sets its runtime path.
- **Nix:** use `pkgs.callPackage /path/to/new-recipes/nix/package.nix {}` with a
  selected Nixpkgs revision and build with sandboxing enabled. A local test can
  override `src` to the local archive while leaving the pinned recipe unchanged.
  Run the installed command and check profile installation/removal too.
- **conda:** run `rattler-build build --recipe conda/recipe.yaml`. A temporary
  local recipe copy can replace its URL with the local archive's `file:` URL,
  retaining the SHA-256. Install the resulting package in a fresh environment,
  repeat `check-installed.sh` there, inspect package contents/metadata and remove
  it. This is binary repackaging for a project channel, not a conda-forge
  source-build submission. Check channel policy before proposing that route.

For every route, create a project index and unrelated file outside the package
prefix, remove the package and check both files survive. Record actual manager,
environment, archive, installed executable and package hashes with the outcomes.
Report any unavailable native or sandbox check explicitly.

## Runtime relocation

Nix patches the ELF interpreter and runtime paths to its immutable glibc
runtime closure through `autoPatchelfHook`. Its installed executable consequently
differs from the archive's executable. After relocation, the recipe retains
`ORIGINAL-ARCHIVE-MANIFEST.sha256`, records the archive hash and installed
executable hash in `NIX-RELOCATION.txt`, and generates a new installed manifest.
`BUILD.json` describes the original archive build. Archive attestations do not
cover the modified Nix executable.

New GNU cross-built archives need no GCC runtime. Homebrew and conda install
their executable unchanged and retain the archive manifest. Nix still needs
interpreter relocation, but its generated recipe no longer needs GCC.

For older archives that import `libgcc_s.so.1`, the generator retains GCC
dependencies and the following relocation steps. Conda adds the relative
`$ORIGIN/../..` runtime path so that the
executable finds the environment's GCC library. It retains the original manifest,
records the changed executable in `CONDA-RELOCATION.txt` and regenerates its
installed manifest. It does not change the host's global library lookup.

Homebrew sets its GCC library's runtime path and records the changed executable
in `HOMEBREW-RELOCATION.txt`, also retaining the original manifest and generating
an installed manifest. `skip_clean` preserves the archive's licences and support
files at their manifested paths.

The conda recipe requires glibc 2.28 and adds the GCC runtime only for those older
archives. Its automatic prefix rewriting and binary relocation are disabled.

The implementations follow the [Homebrew formula API](https://docs.brew.sh/Formula-Cookbook),
[Nixpkgs relocation hook](https://nixos.org/manual/nixpkgs/stable/#sec-auto-patchelf)
and [Rattler-Build repackaging and recipe format](https://rattler-build.prefix.dev/latest/tutorials/repackaging/).

## Publication and updates

After a release, verify downloads from the final release URLs before copying the
generated recipes into their destinations. Keep the helper check beside the Arch,
Nix and conda recipes. Homebrew embeds it. Check applicable channel policies, maintainer
metadata and licensing before any upstream submission. The full bundled dependency
notices remain with each package.

For each later version, regenerate the recipes, reset package
revisions, repeat the installed checks and removal checks, then verify a fresh
installation from the published destination. Only then add that route to user
installation instructions. Publishing recipes to package-manager channels remains
a separate step from generating them and bundling the Arch recipe for release.

The Arch recipe builds a local pacman package without an AUR listing. To prepare
an AUR submission, run `makepkg --printsrcinfo > .SRCINFO` in the generated `aur/`
directory. See the [makepkg manual](https://man.archlinux.org/man/makepkg.8.en).

# Binary distribution preparation

The [Linux release tooling](../docs/linux-release.md#debian-and-rpm-packages)
builds Debian/Ubuntu `.deb` and RHEL-family `.rpm` packages from the checked Linux
x86-64 glibc archive. It verifies the byte-identical executable and complete
notices/editor sources after package extraction, and writes individual SHA-256
sidecars. Runtime dependencies are glibc 2.28 or newer and the system GCC runtime.
A workflow installs, queries and removes them on Debian 12 and AlmaLinux 8.

The Linux tarball and checksum-verified shell installer are also prepared.
Native Windows uses the Windows ZIP and PowerShell installer, described in
[Windows preparation](../docs/windows-release.md). There are no macOS, ARM64 or musl
builds. [Limitations](../docs/reference/limitations.md#platforms-and-storage) lists
the supported platforms.

Additional recipes repackage the same Linux archive for a Homebrew tap, Nix and
a conda channel. Generating recipes does not publish a package.

## Prepare additional recipes

First select a validated archive and independently verify its release checksum
and, when available, its attestation. The preparation command records the bytes
you select. It does not check where they came from. Its version must
match the archive's filename and executable. Use a new output directory whose
parent exists:

```sh
cargo run --locked --example prepare-packages -- \
  --version 0.1.0 \
  --archive /path/to/ripscope-0.1.0-x86_64-linux-gnu.tar.gz \
  --out /path/to/new-recipes
```

The command generates three pinned recipes, their installed checks and
`PREPARATION.json`, containing the archive and generated-file SHA-256 hashes.
It refuses an existing output directory. Keep generated recipes and validation
captures outside the source tree. Generate again in a new directory whenever
the archive, version or recipe templates change. Never reuse an earlier hash
for a rebuilt archive. No tag, release, Cargo publication or channel is changed.

Each package retains the full archive below its private installation prefix and
exposes one command on PATH. The retained directory contains `BUILD.json`, the
file manifest, dependency/Rust/SQLite notices and editor sources (ADR 0014,
"Ripscope ships its own editor integrations, set up in one step"). A package does
not set up editors. The install scripts and plugin managers do.

| Route | Installed support directory | Removal |
| --- | --- | --- |
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

- **Homebrew:** put `homebrew/ripscope.rb` in a local tap's `Formula/` directory.
  Use `brew install --build-from-source OWNER/LOCAL-TAP/ripscope` (this installs
  the upstream binary and does not compile Rust), then `brew test` and the tap's
  applicable `brew audit`/`brew style` checks. For local validation, change only
  the URL in a temporary recipe copy to a local mirror of the exact archive.
  The formula retains the full archive in `libexec/ripscope` and declares the GCC runtime
  through Homebrew's GCC package, with an explicit runtime path to that library.
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

Nix patches the ELF interpreter and runtime paths to its immutable glibc/GCC
runtime closure through `autoPatchelfHook`. Its installed executable consequently
differs from the archive's executable. After relocation, the recipe retains
`ORIGINAL-ARCHIVE-MANIFEST.sha256`, records the archive hash and installed
executable hash in `NIX-RELOCATION.txt`, and generates a new installed manifest.
`BUILD.json` describes the original archive build. Archive attestations do not
cover the modified Nix executable.

Conda similarly adds the relative `$ORIGIN/../..` runtime path so that the
executable finds the environment's GCC library. It retains the original manifest,
records the changed executable in `CONDA-RELOCATION.txt` and regenerates its
installed manifest. It does not change the host's global library lookup.

Homebrew sets its GCC library's runtime path and records the changed executable
in `HOMEBREW-RELOCATION.txt`, also retaining the original manifest and generating
an installed manifest. `skip_clean` preserves the archive's licences and support
files at their manifested paths.

The conda recipe's dependencies name glibc 2.28 and the GCC
runtime. Its automatic prefix rewriting and binary relocation are disabled:
only the explicit relative runtime path is added by the build script.

The implementations follow the [Homebrew formula API](https://docs.brew.sh/Formula-Cookbook),
[Nixpkgs relocation hook](https://nixos.org/manual/nixpkgs/stable/#sec-auto-patchelf)
and [Rattler-Build repackaging and recipe format](https://rattler-build.prefix.dev/latest/tutorials/repackaging/).

## Publication and updates

After a release, verify downloads from the final release URLs before copying the
generated recipes into their destinations. Keep the helper check beside the Nix and
conda recipes. Homebrew embeds it. Check applicable channel policies, maintainer
metadata and licensing before any upstream submission. The full bundled dependency
notices remain with each package.

For each later version, regenerate the recipes, reset package
revisions, repeat the installed checks and removal checks, then verify a fresh
installation from the published destination. Only then add that route to user
installation instructions. This helper adds no recurring workflow or automated
submission.

## AUR template

The preparation command does not use `templates/PKGBUILD` and generates no `aur/`
directory. To use the template, pin it to the selected release archive, generate
`.SRCINFO` and repeat package creation, installed checks and removal checks.

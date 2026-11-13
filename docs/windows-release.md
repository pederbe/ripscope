# Windows release preparation

The `windows` command of the release tool in `tools/release` builds, inspects and
smokes the exact x86-64 MSVC archive. It uses rolling stable Rust and the installed
stable Visual Studio developer environment. The target and static C runtime flags
are recorded in `tools/release/windows-release-profile.json`. It bundles
SQLite and the C runtime. The executable still needs Windows system DLLs.

Use native Windows, Git, stable Rust and Visual Studio's C++ build tools. Run from
Developer PowerShell for the installed stable Visual Studio, selecting a new
external directory on local NTFS:

```powershell
rustup update stable
$toolchain = Split-Path (Split-Path (rustup which --toolchain stable rustc))
cargo run --locked --manifest-path tools/release/Cargo.toml -- windows `
  --work C:/builds/ripscope-candidate `
  --toolchain $toolchain `
  --msvc $env:VCToolsInstallDir `
  --sdk $env:WindowsSdkDir
```

Use the actual installed tool paths. `--target-dir` can reuse an external Cargo
cache. The tool refreshes all copied Rust input timestamps before invoking
Cargo so cached test executables cannot retain a previous checkout. It does not
change global Cargo configuration. Vendoring can download missing locked crates.
compilation and tests use Cargo's offline mode. The Windows host itself is not
network-sandboxed.

The tool records actual compiler and SDK versions and exact source, dependency
and tool inventories, selects the
compiler/linker explicitly, and runs serial release tests. PE inspection rejects
dynamic CRT and foreign DLL imports, including delayed imports. Runtime smoke
uses only Windows system directories on PATH and checks actual loaded DLL paths
while a session answers queries. The product session keeps its restricted runtime
environment. The separate Windows PowerShell inspector uses the normal host
environment and a 30-second deadline. Its phase, JSON inventory and diagnostic log
are retained beside the smoke receipt, including on failure. LF and CRLF cases cover stored/live separation,
available rows with warnings, source deletion, changed-source Refresh and an
unchanged Refresh that preserves the revision.

The ZIP contains the executable, installation instructions, `BUILD.json`, file
manifest, licences and editor integration sources. `dist/` holds the ZIP,
`SHA256SUMS` and the individual `.zip.sha256` sidecar. Command logs, the build
receipt and the smoke receipt remain in the work directory. The smoke can be repeated
on its own against an archive with
`cargo run --locked --manifest-path tools/release/Cargo.toml -- windows-smoke ARCHIVE.zip NEW_DIR`.

Run the installer harness against that exact ZIP under an ordinary Windows
account with native symlink support:

```powershell
cargo run --locked --example check-installers -- `
  --archive C:/builds/ripscope-candidate/dist/ripscope-0.1.0-x86_64-windows-msvc.zip `
  --installer "$PWD/install.ps1" --work C:/builds/ripscope-installer-check
```

It captures checksum/path/link refusals, replacement and locked-executable
preservation, constructed recovery states, postcommit backup cleanup failure,
retained notices and selective removal. It never terminates existing Ripscope
processes. These checks cover the actual tested Windows environment. They do not
qualify every Windows endpoint or baseline-only processor.

### Installer replacement

The installer needs PowerShell 5.1 or newer. It validates the archive's paths, inventory and hashes,
then stages on the destination volume. It refuses directory and reparse-point
executable destinations. Replacing an executable uses `File.Replace`, while a
new installation uses a non-overwriting move. Close active sessions when Windows
locks the executable. The installer does not terminate processes.

A replacement error can occur after the old file moved to its backup. Recovery
restores it only when the destination is absent and the backup is a regular file
matching the recorded old hash. An uncertain outcome retains staging and backup
files and reports their absolute paths. Backup cleanup after a verified successful
replacement can leave a reported backup while installation still succeeds.

The retained `.ripscope-notices-*` directory contains licences, build identity
and editor sources. `ORIGINAL-ARCHIVE-MANIFEST.sha256` describes the original ZIP,
while `BUILD.json` and `INSTALLATION.txt` identify the moved executable by SHA-256.
Earlier notices directories remain after an update.

The [publication and recovery procedure](linux-release.md#weekly-releases)
describes the manual publication guard. Use the
[attestation verification flags](linux-release.md#attestation-verification) to
check the exact repository, workflow, tag and source.
The local build and smoke commands above do not sign, upload or publish artifacts.

## WinGet package

WinGet, Windows' package manager, can install the published Windows ZIP as a
portable package. It unpacks the whole ZIP, keeps its licences and editor sources,
and puts the `ripscope` command on PATH. `winget upgrade` and `winget uninstall`
replace and remove it. Project indexes and the editors' cache stay where they are.

The [WinGet workflow](../.github/workflows/winget.yml) writes manifests from a
published ZIP and checks them on native Windows. The check
validates the manifests, installs the previous release, upgrades to the new one,
finds `ripscope` on a freshly read PATH, indexes and searches a project, runs the
editors' install command and removes the package. The manifests are in the run's
`winget-manifests-vX.Y.Z` artifact, laid out as in
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). The workflow
runs after publication, so it never holds up or changes a release. Run it again for
any published tag from the [WinGet workflow page](https://github.com/pederbe/ripscope/actions/workflows/winget.yml).

To write the manifests for a downloaded ZIP and its `.sha256` sidecar yourself:

```sh
cargo run --locked --manifest-path tools/release/Cargo.toml -- winget \
  0.2.4 /path/to/ripscope-0.2.4-x86_64-windows-msvc.zip 2026-10-26 /path/to/new-dir
```

The date is the release's publication date. The command refuses an archive that
does not match its sidecar or has no `bin/ripscope.exe`, and an existing output
directory.

When running release-tool commands on Linux, use `env RUSTFLAGS='' cargo ...`.
The product's Linux executable flags do not apply to compiler plugins used by
the release tool.

### Submission

The package identifier is `PederBergan.Ripscope`. Submission opens a pull request
in `microsoft/winget-pkgs` with the checked version's manifest directory. WinGet's
validation and moderators review it there.

To retry submission or an unfinished description, run the WinGet workflow on
`main` for the same published tag with submission enabled. It reuses an existing
same-version PR and preserves manual description edits. Preparation without
submission makes no public changes.

Submission uses a public-only credential. Synchronizing the submission fork uses
a separate fork-write credential and never forces a divergent fork. See the
workflow for its authentication inputs. Preparation does not use the fork-write
credential.
The CLA checkbox reflects Microsoft's successful check and does not sign an agreement.
Once WinGet's source lists the version, check
`winget install PederBergan.Ripscope` from that source on a clean Windows account
before the user documentation mentions WinGet.

### Editors with a WinGet installation

The editor integrations find `ripscope` on PATH first, so they use the WinGet
executable. Plugin managers install the editor plugin from the `release` branch as
usual. Their `:RipscopeInstall` build step leaves an executable it did not install
alone, so it never adds a second copy beside the WinGet one. The one-line installer
always installs its own copy, so WinGet users set up their editors through a plugin
manager instead.

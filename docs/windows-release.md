# Windows release preparation

The `windows` command of the release tool in `tools/release` builds, inspects and
smokes the exact x86-64 MSVC archive. Its toolchain, MSVC directory, SDK and static C
runtime flags are pinned in `tools/release/windows-release-profile.json`. It bundles
SQLite and the C runtime; the executable still needs Windows system DLLs.

Use native Windows, Git and the recorded Rust/MSVC/SDK tools, plus Rust 1.98 or later
to build the release tool itself. Run from an ordinary shell, selecting a new external
directory on local NTFS:

```powershell
cargo run --locked --manifest-path tools/release/Cargo.toml -- windows `
  --work C:/builds/ripscope-candidate `
  --toolchain C:/Users/USER/.rustup/toolchains/1.99.1-x86_64-pc-windows-msvc `
  --msvc 'C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/MSVC/14.44.35207' `
  --sdk 'C:/Program Files (x86)/Windows Kits/10'
```

Use the actual installed tool paths. `--target-dir` can reuse an external Cargo
cache; the tool refreshes all copied Rust input timestamps before invoking
Cargo so cached test executables cannot retain a previous checkout. It does not
change global Cargo configuration. Vendoring can download missing locked crates;
compilation and tests use Cargo's offline mode. The Windows host itself is not
network-sandboxed.

The tool records exact source, dependency and tool inventories, selects the
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
processes. These checks cover the actual tested Windows environment; they do not
qualify every Windows endpoint or baseline-only processor.

The manual publication guard and exact repository/workflow/tag/source attestation
verification flags are in [Linux release preparation](linux-release.md#guarded-release-workflow).
The release tool never signs, uploads or publishes artifacts when run locally.

## WinGet package

WinGet, Windows' package manager, can install the published Windows ZIP as a
portable package. It unpacks the whole ZIP, keeps its licences and editor sources,
and puts the `ripscope` command on PATH. `winget upgrade` and `winget uninstall`
replace and remove it. Project indexes and the editors' cache stay where they are.

After each published release, the `WinGet` workflow writes the release's three
manifests from its published ZIP and checks them on GitHub-hosted Windows. The check
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

### Submission

The package identifier is `PederBergan.Ripscope`. Submitting a version means opening
a pull request in `microsoft/winget-pkgs` with that version's manifest directory.
WinGet's own validation and moderators review it there. Submission is a separate,
manual step after the workflow passes. Once WinGet's source lists the version, check
`winget install PederBergan.Ripscope` from that source on a clean Windows account
before the user documentation mentions WinGet.

### Editors with a WinGet installation

The editor integrations find `ripscope` on PATH first, so they use the WinGet
executable. Plugin managers install the editor plugin from the `release` branch as
usual. Their `:RipscopeInstall` build step leaves an executable it did not install
alone, so it never adds a second copy beside the WinGet one. The one-line installer
always installs its own copy, so WinGet users set up their editors through a plugin
manager instead.

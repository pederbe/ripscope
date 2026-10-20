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
  --toolchain C:/Users/USER/.rustup/toolchains/1.98.1-x86_64-pc-windows-msvc `
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
while a session answers queries. LF and CRLF cases cover stored/live separation,
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

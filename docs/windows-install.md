# Windows archives and source builds

The x86-64 MSVC archive bundles the C runtime and SQLite. Archive users need
Windows system DLLs, but no Rust toolchain, C compiler, Visual C++ Redistributable
or separate SQLite installation. The current native smoke runs on Windows 11;
it does not establish qualification of every Windows release or processor.

## Installer

After a release is published, run in PowerShell 5.1 or newer:

```powershell
irm https://ripscope.com/install.ps1 | iex
```

The installer downloads the selected ZIP and its SHA-256 sidecar, validates its
paths, file inventory and hashes, then stages on the destination volume. It
refuses directory and reparse-point executable destinations. The default is
`$HOME\.local\bin\ripscope.exe`; administrator rights are unnecessary.
Set `$env:RIPSCOPE_VERSION`, `$env:RIPSCOPE_INSTALL_DIR` or
`$env:RIPSCOPE_RELEASES` before invoking it to select a version, destination or
explicit mirror. The checksum checks the download against the release's checksum;
use the separate [attestation check](linux-release.md#attestation-verification)
when verifying its provenance.

It also places the Vim and Neovim plugins in `%USERPROFILE%\vimfiles\pack\ripscope`
and `%LOCALAPPDATA%\nvim-data\site\pack\ripscope`, where those editors load
packages by themselves, and the Emacs integration in `%LOCALAPPDATA%\ripscope\emacs`,
and prints the line Emacs needs. Set `$env:RIPSCOPE_EDITORS = 'none'` to leave the
editors alone.

The installer prints an absolute `.ripscope-notices-<version>.<random>` directory
beside the executable. Keep its licences, build identity and editor integrations.
`ORIGINAL-ARCHIVE-MANIFEST.sha256` describes the original ZIP; the executable has
moved to the printed destination. `BUILD.json` and `INSTALLATION.txt` identify its
SHA-256. Earlier notices directories are retained automatically.

Replacing an existing executable uses `File.Replace`; a new installation uses
a non-overwriting move. Close an active Ripscope session before updating if
Windows has locked its executable. The installer does not terminate processes.
A replacement error can occur after Windows has moved the old file to its backup.
Recovery restores that backup only when the destination is absent and the backup
is a regular file matching the recorded old hash. An uncertain outcome retains
staging and backup files and reports their absolute paths. Backup cleanup after a
verified successful replacement can also leave a reported backup; installation
still succeeds in that case.

Add the installation directory to your user PATH through Windows settings if
needed. To remove an installation, run the installer with `-Uninstall`:

```powershell
& ([scriptblock]::Create((irm https://ripscope.com/install.ps1))) -Uninstall
```

It removes the executable, every notices directory and the editor integrations. The
indexes the editors made remain in `%LOCALAPPDATA%\ripscope` (or the folder
`RIPSCOPE_CACHE_DIR` names), and project indexes and unrelated files are untouched.

## Source build

Source builds need Rust/Cargo for `x86_64-pc-windows-msvc`, Microsoft's Visual
Studio C++ Build Tools and the Windows SDK. In an x64 MSVC developer shell:

```powershell
$env:RUSTFLAGS = '-C target-cpu=x86-64 -C target-feature=+crt-static'
$env:CFLAGS = '/MT'
cargo build --release --locked --target x86_64-pc-windows-msvc
```

See [Windows release preparation](windows-release.md) for the pinned tool and
archive inspection recipe.

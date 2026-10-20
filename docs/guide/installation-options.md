# Other installation methods

For the standard installation, use the [install script](../getting-started/installation.md).
This page covers PATH setup, system packages, manual downloads, source builds
and installer options.

## Add Ripscope to PATH

PATH is the list of directories your terminal searches for commands. The
install scripts put Ripscope in `~/.local/bin` on Linux and
`%USERPROFILE%\.local\bin` on Windows.

### Linux

For Fish, run this once, then try `ripscope --version` again.

```fish
fish_add_path ~/.local/bin
```

For Bash, add this line to `~/.bashrc`. For Zsh, add it to `~/.zshrc`.
Then open a new terminal.

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Windows

In Start, open **Edit environment variables for your account**. Under user
variables, edit **Path**, choose **New** and add `%USERPROFILE%\.local\bin`.
Open a new terminal, then run `ripscope --version`.

## Debian/Ubuntu and RHEL-family packages

Select the version from [GitHub Releases](https://github.com/pederbe/ripscope/releases).
Debian/Ubuntu `.deb` and RHEL-family `.rpm` packages contain the inspected Linux
x86-64 executable, with glibc 2.28 or newer and the system GCC runtime required.
Verify the selected package's `.sha256` sidecar, then install it:

```sh
sha256sum -c ripscope_0.1.0_amd64.deb.sha256
sudo apt install ./ripscope_0.1.0_amd64.deb
```

```sh
sha256sum -c ripscope-0.1.0-1.x86_64.rpm.sha256
sudo dnf install ./ripscope-0.1.0-1.x86_64.rpm
```

Notices and editor sources are installed in `/usr/share/ripscope`.

## Archives and system packages

Select the version from [GitHub Releases](https://github.com/pederbe/ripscope/releases):

| Platform | Archive | Runtime |
| --- | --- | --- |
| Linux x86-64 | `ripscope-<version>-x86_64-linux-gnu.tar.gz` | glibc 2.28 or newer and the system GCC runtime; not musl/Alpine |
| Windows x86-64 | `ripscope-<version>-x86_64-windows-msvc.zip` | Windows system DLLs; the C runtime is bundled |

The archives include their executable, licences, editor sources and build
manifest. Compare the archive against its individual `.sha256` sidecar before
extracting it. Keep the complete extracted directory:

```sh
sha256sum -c ripscope-0.1.0-x86_64-linux-gnu.tar.gz.sha256
tar -xzf ripscope-0.1.0-x86_64-linux-gnu.tar.gz
./ripscope-0.1.0-x86_64-linux-gnu/bin/ripscope --help
```

```powershell
Get-FileHash .\ripscope-0.1.0-x86_64-windows-msvc.zip -Algorithm SHA256
Get-Content .\ripscope-0.1.0-x86_64-windows-msvc.zip.sha256
Expand-Archive .\ripscope-0.1.0-x86_64-windows-msvc.zip -DestinationPath .\ripscope-install
.\ripscope-install\ripscope-0.1.0-x86_64-windows-msvc\bin\ripscope.exe --help
```

Checksums identify the downloaded bytes. For repository and workflow provenance,
use the exact-tag/source
[attestation verification instructions](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md#attestation-verification).

## Building from source

You need a recent stable [Rust toolchain](https://rustup.rs/) and a C compiler
for bundled SQLite: GCC or Clang on Linux; Microsoft's MSVC Build Tools and the
Windows SDK on Windows. Use an x64 developer shell for an MSVC build.

```sh
git clone https://github.com/pederbe/ripscope.git
cd ripscope
cargo build --release --locked
```

Or install directly from the repository:

```sh
cargo install --locked --git https://github.com/pederbe/ripscope.git
```

A binary built on a newer Linux distribution can require that distribution's
newer glibc. The pinned archive recipes and their inspection limits are documented
in [Linux preparation](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md)
and [Windows preparation](https://github.com/pederbe/ripscope/blob/main/docs/windows-release.md).
A separate Rust toolchain or SQLite installation is unnecessary for archive users.


## Installer options

The install scripts select the latest release and install into your user account.
Set these environment variables before running a script to change that behavior.

| Variable | Purpose |
| --- | --- |
| `RIPSCOPE_VERSION` | Select a release version |
| `RIPSCOPE_INSTALL_DIR` | Choose another executable directory |
| `RIPSCOPE_RELEASES` | Use a release mirror |
| `RIPSCOPE_EDITORS` | `all` (the default) sets up the editors; `none` leaves them alone |

In PowerShell, set them through `$env:`.

### Editor integrations

Besides the executable, the scripts put the editor integrations where the editors
find them:

| Editor | Linux | Windows |
| --- | --- | --- |
| Vim | `~/.vim/pack/ripscope/start/ripscope` | `%USERPROFILE%\vimfiles\pack\ripscope\start\ripscope` |
| Neovim | `~/.local/share/nvim/site/pack/ripscope/start/ripscope` | `%LOCALAPPDATA%\nvim-data\site\pack\ripscope\start\ripscope` |
| Emacs | `~/.local/share/ripscope/emacs` | `%LOCALAPPDATA%\ripscope\emacs` |

Vim and Neovim load packages from these directories by themselves. Vim uses
`~/.config/vim` instead when that is where its vimrc is. For Emacs, the script
prints the line to add to your init file. Neovim set up with lazy.nvim loads only
the plugins in lazy.nvim's list, so where the script finds lazy.nvim it prints the
line to add there as well. Running the script again updates them with the executable.

With a system package or an archive, the same files are in its `contrib` directory:
copy `contrib/vim` to the Vim or Neovim directory above, and load
`contrib/emacs/ripscope.el` from Emacs as the [Emacs guide](../editors/emacs.md)
shows.

### Notices

The scripts retain a hidden `.ripscope-notices-*` directory beside the
executable. It contains licences, build information and a copy of the editor
integrations. The Emacs integration includes xcscope's source and its own
`COPYING` file.

To find the directories on Linux, run

```sh
ls -d ~/.local/bin/.ripscope-notices-*
```

On Windows, list them in PowerShell with

```powershell
Get-ChildItem "$HOME\.local\bin" -Force -Directory -Filter '.ripscope-notices-*'
```

Running the installer again updates the executable and retains earlier support
directories. On Windows, close active Ripscope sessions before replacing the
executable if it is locked.

To remove an installation, see [uninstalling Ripscope](uninstalling.md).

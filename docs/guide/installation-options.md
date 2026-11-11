# Other installation methods

Besides the [one-command installer](../getting-started/installation.md), Ripscope
installs from the apt and dnf repository, standalone packages, archives, Cargo and
source. You can also add Ripscope to PATH yourself and change what the installer
does.

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

## apt and dnf repositories

On Linux, use the signed apt or dnf repository to receive releases with
your normal package-manager updates. Packages include the executable, notices
and editor sources under `/usr/share/ripscope`.

### Debian and Ubuntu

Install the tools needed to download and inspect the repository key:

```sh
sudo apt install ca-certificates curl gnupg
curl -fsSLo ripscope.gpg https://packages.ripscope.com/ripscope.gpg
gpg --show-keys --fingerprint ripscope.gpg
```

Check that its fingerprint is:

```text
3710 BE46 B974 AAB7 390D 225B FAFC E829 50BF F0D6
```

Then add the repository and install:

```sh
sudo install -m 644 ripscope.gpg /usr/share/keyrings/ripscope.gpg
echo "deb [arch=amd64 signed-by=/usr/share/keyrings/ripscope.gpg] https://packages.ripscope.com/apt stable main" |
  sudo tee /etc/apt/sources.list.d/ripscope.list
sudo apt update
sudo apt install ripscope
```

Update with `sudo apt update` and `sudo apt upgrade ripscope`.
Remove the package with `sudo apt remove ripscope`.
Installation and upgrades are checked on Debian 12 and Ubuntu 22.04 and 24.04.

### RHEL, AlmaLinux and Fedora

```sh
sudo curl -fsSLo /etc/yum.repos.d/ripscope.repo https://packages.ripscope.com/ripscope.repo
sudo dnf install ripscope
```

On first installation, check that the signing-key fingerprint matches the one
above before accepting it. Package and repository signature checks stay enabled.
Update with `sudo dnf upgrade ripscope` and remove with `sudo dnf remove ripscope`.
The RHEL-family baseline is AlmaLinux 8.

Removing the package preserves your projects, indexes and editor settings.
For editor integration, follow the [plugin-manager instructions](../getting-started/installation.md#your-plugin-manager).
They reuse the executable installed by your package manager.

## Debian/Ubuntu and RHEL-family packages

For a standalone package without repository updates, download the `.deb` or `.rpm`
and its `.sha256` file from the
[latest release](https://github.com/pederbe/ripscope/releases/latest) into an empty directory. The packages need
glibc 2.28 or newer. Check the package against its
`.sha256` file, then install it:

```sh
sha256sum -c ripscope_*_amd64.deb.sha256 &&
sudo apt install ./ripscope_*_amd64.deb
```

```sh
sha256sum -c ripscope-*-1.x86_64.rpm.sha256 &&
sudo dnf install ./ripscope-*-1.x86_64.rpm
```

Notices and editor sources are installed in `/usr/share/ripscope`.

## Archives and system packages

Download the archive and its `.sha256` sidecar from the
[latest release](https://github.com/pederbe/ripscope/releases/latest) into an empty directory:

| Platform | Archive | Runtime |
| --- | --- | --- |
| Linux | `.tar.gz` | glibc 2.28 or newer. Not musl or Alpine |
| Windows | `.zip` | Windows system DLLs. The C runtime is bundled |

The archives include their executable, licences, editor sources and build
manifest. Compare the archive against its individual `.sha256` sidecar before
extracting it. Keep the complete extracted directory:

```sh
sha256sum -c ripscope-*-x86_64-linux-gnu.tar.gz.sha256 &&
tar -xzf ripscope-*-x86_64-linux-gnu.tar.gz &&
./ripscope-*-x86_64-linux-gnu/bin/ripscope --help
```

```powershell
$archive = Get-ChildItem -File -Filter 'ripscope-*-x86_64-windows-msvc.zip'
Get-FileHash $archive.FullName -Algorithm SHA256
Get-Content "$($archive.FullName).sha256"
```

After the two hashes match, extract and run it:

```powershell
Expand-Archive $archive.FullName -DestinationPath .\ripscope-install
& (Get-ChildItem .\ripscope-install\*\bin\ripscope.exe).FullName --help
```

Checksums identify the downloaded bytes. For repository and workflow provenance,
use the exact-tag/source
[attestation verification instructions](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md#attestation-verification).

## Cargo installation

If you already use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
install the published Linux or Windows binary without compiling:

```sh
cargo binstall ripscope
```

To build and install the latest stable crates.io release:

```sh
cargo install --locked ripscope
```

Cargo installs the executable into `~/.cargo/bin` by default. Make sure that
directory is on your `PATH`, including the path your editor uses.

For source builds, you need a recent stable [Rust toolchain](https://rustup.rs/)
and a C compiler for bundled SQLite. Use GCC or Clang on Linux. On Windows,
install Microsoft's MSVC Build Tools and the Windows SDK, then use an x64
developer shell.

These commands install the executable only. Add your editor's integration using
[your plugin manager](../getting-started/installation.md#your-plugin-manager).
The plugins reuse an executable on `PATH`. Updating a plugin leaves that
executable under Cargo's control. Run the same installation command again
to update it, or `cargo uninstall ripscope` to remove it.

A binary built on a newer Linux distribution can require that distribution's
newer glibc. The pinned archive recipes and their inspection limits are documented
in [Linux preparation](https://github.com/pederbe/ripscope/blob/main/docs/linux-release.md)
and [Windows preparation](https://github.com/pederbe/ripscope/blob/main/docs/windows-release.md).
A separate Rust toolchain or SQLite installation is unnecessary for archive users.


## Installer options

The install scripts select the latest release and install into your user account.
Set these environment variables before running a script to change its destination,
or editor setup.

| Variable | Purpose |
| --- | --- |
| `RIPSCOPE_INSTALL_DIR` | Choose another executable directory |
| `RIPSCOPE_EDITORS` | `all` (the default) sets up the editors. `none` leaves them alone |

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

With a system package or an archive, the same files are in its `contrib` directory.
Copy `contrib/vim` to the Vim or Neovim directory above, and load
`contrib/emacs/ripscope.el` from Emacs as the [Emacs guide](../editors/emacs.md)
shows.

### Notices

The scripts retain a hidden `.ripscope-notices-*` directory beside the
executable. It contains licences, build information and a copy of the editor
integrations.

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

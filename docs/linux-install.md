# Linux archives and source builds

The x86-64 Linux archive targets glibc 2.28 or newer. It includes SQLite and
its notices. It requires the GNU loader and system C libraries, including
`libgcc_s`. It does not need a separate SQLite installation.

## Installer

After a release is published, the installer downloads its archive and sidecar
SHA-256 checksum, verifies every archive file, and replaces the executable by
renaming a staged file on the destination filesystem:

```sh
curl -fsSL https://ripscope.com/install.sh | sh
```

It needs curl, GNU tar, gzip and ordinary GNU coreutils. The default destination
is `~/.local/bin/ripscope`. Set `RIPSCOPE_VERSION=0.1.0` to select a version,
`RIPSCOPE_INSTALL_DIR` for another directory, or `RIPSCOPE_RELEASES` for an
explicit mirror. An HTTPS mirror must keep redirects on HTTPS. The checksum
checks the download against the release's checksum, and the separate attestation
check in [release preparation](linux-release.md#attestation-verification) checks
its provenance.

It also places the Vim and Neovim plugins in `~/.vim/pack/ripscope` and
`~/.local/share/nvim/site/pack/ripscope`, where those editors load packages by
themselves, and the Emacs integration in `~/.local/share/ripscope/emacs`, and prints
the line Emacs needs. Set `RIPSCOPE_EDITORS=none` to leave the editors alone.

The installer prints an absolute `.ripscope-notices-<version>.<random>` directory
beside the executable. Keep it: it contains dependency licences, build identity,
and the editor integrations, including the complete separately licensed xcscope
source and `COPYING`. Its `ORIGINAL-ARCHIVE-MANIFEST.sha256` describes the original
archive, and the executable has moved to the printed destination. `BUILD.json` and
`INSTALLATION.txt` identify that executable by SHA-256.

Every successful installation keeps its own notices directory. An interrupted
replacement also retains it because the executable may already have changed.
The installer reports these paths and does not prune earlier copies. Directory
and symbolic-link executable destinations are refused. A running Linux copy can
finish using its existing executable while subsequent launches use the new one.

Add the installation directory to PATH through your shell configuration. For Fish:

```fish
fish_add_path ~/.local/bin
```

To remove a script installation, run the installer with `--uninstall`
(`curl -fsSL https://ripscope.com/install.sh | sh -s -- --uninstall`). It removes
the executable, every notices directory and the editor integrations. The indexes the
editors made remain in `~/.cache/ripscope` (or the folder `RIPSCOPE_CACHE_DIR` names),
and project indexes and source are untouched.

## Packages

The `.deb` and `.rpm` contain the same inspected executable as the tar archive.
They place it in `/usr/bin/ripscope` and retain notices and editor source under
`/usr/share/ripscope`. Download the package and its `.sha256` file from the chosen
release, check it with `sha256sum -c`, then install with your package manager:

```sh
sudo apt install ./ripscope_0.1.0_amd64.deb
# Or, on an RPM distribution:
sudo dnf install ./ripscope-0.1.0-1.x86_64.rpm
```

Use `sudo apt remove ripscope` or `sudo dnf remove ripscope` to remove the package.
Project indexes and unrelated files are outside the package.

## Source build

Source builds need Rust/Cargo and a C compiler for bundled SQLite. On x86-64 Linux,
the ordinary command is:

```sh
cargo build --release --locked
./target/release/ripscope --help
```

A binary built against a newer host glibc can require that version at runtime.
The pinned compatibility recipe is described in [Linux release preparation](linux-release.md).

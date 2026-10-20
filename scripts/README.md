# Development tools

Every tool here is Rust, like the product. The language check test,
`tests/language_check.rs`, lists the few files in other languages: the installers,
the editor plugins, the website and short build glue. These tools compare real
reference outputs and help maintain the checked-in fixtures:

| Tools | Purpose |
|---|---|
| `tools/release` | Build, inspect and smoke the Linux and Windows release archives. See [Linux preparation](../docs/linux-release.md) and [Windows preparation](../docs/windows-release.md). |
| `cargo run --example prepare-packages` | Pin local Homebrew Linux, Nix and conda recipes to a selected Linux archive. See [package-manager preparation](../packaging/README.md). |
| `cargo run --example assess-navigation` | Source-labelled definition and caller/callee accuracy, independently of the reference. See [navigation assessment](navigation-assessment.md). |
| `cargo run --example assess-workflows` | Replay frozen editing and navigation workflows on disposable copies. See [workflow replay](assess-workflows.md). |
| `cargo run --example coverage-campaign` | Acquire pinned C selections and draw a repeatable source sample. See [coverage campaigns](coverage-campaign.md). |
| `cargo run --example conditional-heads` | Inventory function heads written across conditionals. See [conditional heads](conditional-heads.md). |
| `cargo run --release --example stress-readers` | Stored queries under concurrent index builds in separate processes. See [concurrent readers](concurrent-readers.md). |
| `cargo run --release --example compass-scale` | Full-tree reference comparison, decoding the reference database with `examples/support/cscope_db.rs`. See [full-tree runs](compass-scale.md). |
| `cargo run --example scanner-probes`, `cargo run --example query-probes` | Capture reference answers for small synthetic C inputs, on native Linux or Ubuntu WSL from Windows. |
| `cargo run --example intended-answers`, `cargo run --example query-semantics` | Check intended answers against a second implementation of the query rules in ADR 0010, "Ripscope's own query semantics give each name the context it sits in". |

Ordinary `cargo test` runs these tools' own tests: comparison framing, counts, paths,
reference decoding and reference execution. The [compatibility method](../docs/architecture/quality.md)
describes the corpus checks. The [editor checks](../tests/editor/README.md) run real
editors separately from ordinary Cargo tests, through the `check-vim`, `check-neovim`
and `check-emacs` examples.

Reference captures keep their source inputs, queries, expected answers and reference
identities, and the approved-fixture replay reads the sources from those captures
directly. Expected query answers are checked by a separate implementation of the query
rules, not by the production query engine.

## Reference runtime

The pinned executable follows ADR 0001, "A pinned cscope executable defines expected
results". The probe tools and full-tree comparison verify its SHA-256 before each
reference command, including reads of cached captures. Linux runs it directly, and
Windows runs it inside Ubuntu WSL. The selection and C locale are the same on both.

After copying the executable from Windows, restore its execute permission:

```sh
package=.reference/cscope
chmod u+x "$package/extracted/usr/bin/cscope"
```

The default package directory is `.reference/cscope`, which Git ignores. Set
`RIPSCOPE_REFERENCE_DIR` to reuse an extracted package elsewhere. The executable stays
at `extracted/usr/bin/cscope` within that directory. Captured source, commands and
identities are indexed in [the evidence guide](../evidence/README.md).

On systems that provide only wide ncurses, the Ubuntu reference also needs private
`libncurses6` and matching `libtinfo6` libraries. Extract these official Ubuntu packages
beside the reference, keeping the system libraries intact. Their download pages record
the checksums ([libncurses6](https://packages.ubuntu.com/questing/amd64/libncurses6/download),
[libtinfo6](https://packages.ubuntu.com/questing/amd64/libtinfo6/download)).

```sh
mkdir -p "$package/runtime"
curl -fL "https://security.ubuntu.com/ubuntu/pool/main/n/ncurses/libncurses6_6.5+20250216-2ubuntu0.1_amd64.deb" -o "$package/libncurses6.deb"
curl -fL "https://security.ubuntu.com/ubuntu/pool/main/n/ncurses/libtinfo6_6.5+20250216-2ubuntu0.1_amd64.deb" -o "$package/libtinfo6.deb"
(cd "$package" && printf '%s\n' \
  '5e9eb640c0255fcf63d7070cff1dd11c35aaab244b5e53fd17f441fb0e61ca30  libncurses6.deb' \
  '889e2450c1009926d60774733ad66acc242793db6591645efe4ddad364008159  libtinfo6.deb' | sha256sum -c -) && \
for archive in "$package/libncurses6.deb" "$package/libtinfo6.deb"; do
  ar p "$archive" data.tar.zst | tar --zstd -xf - -C "$package/runtime"
done
```

The tools use `runtime/usr/lib/x86_64-linux-gnu` for the reference process only.
`RIPSCOPE_REFERENCE_LIBRARY_PATH` can select another private runtime, and an empty
value uses the system libraries. The full-tree comparison uses the same runtime. These
files are local prerequisites that Git ignores. The product has no ncurses dependency.

Reference commands use GNU `timeout` inside Linux, including under WSL. The outer
deadline kills the process group, and inner deadlines use `--foreground` to keep their
commands in that group. The full-tree comparison also needs GNU time at
`/usr/bin/time`. Install it from your distribution if it is missing, for example with
`sudo pacman -S --needed time` on Arch Linux.

Run the reference tools' tests after setup. On Linux, the timeout and transport tests
run the process-group deadline for real, with a stand-in for the reference:

```sh
cargo test --locked --example scanner-probes --example query-probes --example compass-scale
```

## Release preparation

The `tools/release` crate builds local archives with the pinned profiles beside it,
records exact inputs, inspects runtime imports, and smokes the analyzer, stored and live
queries, warnings and refresh. Its `linux` and `windows` commands share the source
inventory and notice delivery. `package-linux.sh` turns the inspected Linux executable
into Debian and RPM packages without rewriting it.

The `check-installers` example drives `install.sh` or `install.ps1` against a loopback
mirror of an exact archive, and checks refusal, replacement and removal. Neither the
release tool nor the installer harness is a runtime dependency.

See [Linux preparation](../docs/linux-release.md) and
[Windows preparation](../docs/windows-release.md). The release tool's own checks run
with `cargo test --locked --manifest-path tools/release/Cargo.toml`. Local commands
do not upload or publish. The manual release workflow checks that the tag matches the
source, and its publication step is off by default.

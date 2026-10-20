# Actual Vim capture

This standalone Rust tool captures the example-project workflow through real Vim and
its bundled project configuration. It owns one isolated tmux server and writes
to a new output directory. See [the media instructions](../../scripts/demo/README.md)
for prerequisites, exact commands, capture provenance and rendering.

```sh
cargo fmt --manifest-path tools/media/Cargo.toml --check
cargo clippy --locked --all-targets --manifest-path tools/media/Cargo.toml -- -D warnings
```

A successful capture is the integration check: actual saved source, navigation,
quickfix rows, unchanged connection and exact backend output must all match.

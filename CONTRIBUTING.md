# Contributing to Squeak

Thank you for helping improve Squeak!

## Development Rules

- **License:** All contributions are licensed under `GPL-3.0-or-later`. Do not copy code from non-compatible sources.
- **Toolchain:** Minimum Supported Rust Version (MSRV) is **1.89** (Edition 2024).
- **Zero-panic / zero-`unwrap()` policy:** `unwrap()`, `expect()`, and `panic!()` are denied outside `#[cfg(test)]`. Every `unsafe` block requires an explicit `// SAFETY:` justification.
- **Protocol independence:** Keep `crates/hidpp` free of GUI, udev, and D-Bus dependencies. Never hardcode feature indices or branch on device model names in protocol code.
- **Privacy:** Never include device serial numbers in logs, test fixtures, `dump-device` snapshots, or pull requests.

## Quality Gate

Before committing or opening a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check   # if cargo-deny is installed
```

Use conventional commit messages (`feat(hidpp): ...`, `fix(cli): ...`, `docs: ...`).

## Adding Support for a Device

Device definitions live in `data/devices/<slug>.toml` (see [data/devices/README.md](data/devices/README.md)):

1. Run `squeak dump-device --json` on your device and inspect the output to ensure **no serial numbers** are present.
2. Add `data/devices/<model>.toml` (and optionally `data/devices/<model>.svg` matching each button's `svg_id`).
3. Verify schema and uniqueness tests pass with `cargo test -p squeak-core`.
4. Open a pull request using the device contribution section of `.github/PULL_REQUEST_TEMPLATE.md`.

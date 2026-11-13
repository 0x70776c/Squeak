---
name: rust-quality-gate
description: Use before every commit and at the end of every milestone to verify code quality, tests, and CI parity.
---

# Quality gate (run in order, fix before proceeding)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps   # no broken intra-doc links
cargo deny check                  # licenses + advisories (if configured)
cargo machete                     # unused deps (if installed)
```

## Self-review checklist (do it as a hostile reviewer)
- Error paths: every I/O and parse has a typed error; no panics on malformed device data
- Bounds: all slice indexing guarded; frames validated for length/report-id first
- Concurrency: no blocking HID I/O on async executors; no locks held across `.await`
- Public API minimal (`pub(crate)` by default), documented, `#[non_exhaustive]` on wire enums
- Tests: happy path, malformed input, timeout, busy-retry, unknown-feature
- No dead code, no TODO without issue reference in `docs/PROGRESS.md`
- MSRV honored (see `rust-toolchain.toml` / `Cargo.toml rust-version`)

## Process
1. Gate fails -> fix root cause, never `#[allow]` without a justification comment
2. Gate green -> commit (`feat(hidpp): ...`, `fix(...)`, `test(...)`, `docs(...)`)
3. Update `docs/PROGRESS.md`

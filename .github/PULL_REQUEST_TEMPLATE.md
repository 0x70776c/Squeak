## Summary

<!-- Describe what this PR changes and why. Link relevant milestone/issue. -->

## Type of change

- [ ] Feature / protocol support
- [ ] Bug fix
- [ ] Device definition (`data/devices/`)
- [ ] Packaging / CI / documentation

## Quality checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --workspace --all-features` passes
- [ ] No `unwrap()` / `expect()` outside tests; no `unsafe` without `// SAFETY:` comment
- [ ] Unit/golden tests added for new protocol or logic paths
- [ ] `docs/` and `docs/PROGRESS.md` updated where applicable

## Device support contribution (fill out when adding/updating `data/devices/*.toml`)

- **Model name:**
- **USB / BT VID:PID (`lsusb`):**
- **Connected via:** [ ] Direct USB [ ] Unifying / Bolt receiver [ ] Bluetooth
- **Hardware verification:**
  - [ ] Attached sanitized `squeak dump-device` output (confirm NO serial numbers included)
  - [ ] Verified all `[[buttons]]` `cid` values match `REPROG_CONTROLS_V4` dump
  - [ ] Verified `svg_id` attributes match `data/devices/<slug>.svg` (if SVG included)
  - [ ] `cargo test -p squeak-core` device TOML validation passes

<details>
<summary>Sanitized `squeak dump-device` JSON output (no serial numbers)</summary>

```json
```

</details>

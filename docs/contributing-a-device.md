# Contributing a Device Definition

Squeak uses declarative TOML files under `data/devices/` (plus optional `320x520` SVG diagrams) to enrich dynamically discovered HID++ 2.0 features with friendly button names, sensor quirks, and clickable SVG layouts.

## 1. Capture a Serial-Free Device Dump

With your device connected and udev permissions configured (`squeak setup-udev`):

```bash
squeak dump-device --json > my-mouse-dump.json
```

`squeak dump-device` automatically omits all serial numbers and unit IDs.

## 2. Create `data/devices/<slug>.toml`

Use [g502-hero.toml](../data/devices/g502-hero.toml) and [data/devices/README.md](../data/devices/README.md) as references:

- Set unique `vendor_id` and `product_id` values.
- Optionally specify `svg = "<slug>.svg"` (must be a plain filename inside `data/devices/`).
- List `[[buttons]]` entries with `cid`, `label`, and matching `svg_id` values present in your SVG file.
- Do **not** include vendor trademarks or proprietary logos in SVG diagrams or labels.

## 3. Validate Locally

Run the workspace test suite — `squeak-core` automatically validates every TOML and SVG under `data/devices/`:

```bash
cargo test -p squeak-core
```

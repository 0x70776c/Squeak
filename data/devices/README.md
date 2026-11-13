# Device Definitions (`data/devices/*.toml`)

Device TOML files describe **presentation and proven firmware quirks only**.
All supported features, DPI ranges, LED zones, and programmable controls are always discovered at runtime via `IRoot` (`0x0000`) and `IFeatureSet` (`0x0001`).

Any unknown HID++ 2.0 device automatically falls back to **generic mode**, labeling controls by their reported Control ID (CID) and task ID.

## Schema
```toml
name = "G502 HERO"
vendor_id = 0x046d
product_ids = [0xc08b]
kind = "mouse"
svg = "g502-hero.svg" # optional; falls back to generic mouse SVG

[[buttons]]
cid = 0x0050          # Control ID reported by REPROG_CONTROLS_V4 (0x1B04)
label = "Left click"
svg_id = "btn-left"   # Must match an element id in `svg` if `svg` is set

[quirks]
# Only for proven firmware bugs, each with an explanatory comment + issue link
```

## Rules
- Every `.toml` file in this directory is validated by `cargo test` (`serde` schema, unique `(vendor_id, product_id)` pairs across files, valid `svg_id` references when an SVG file is provided).
- Never include device serial numbers in device definitions, snapshots, or PR submissions.

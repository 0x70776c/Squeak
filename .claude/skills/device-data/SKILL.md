---
name: device-data
description: Use when adding or changing device definitions (data/devices/*.toml), button layouts, SVGs, or the generic-mode fallback.
---

# Device data model

```toml
# data/devices/g502-hero.toml
name = "G502 HERO"
vendor_id = 0x046d
product_ids = [0xc08b]        # verify with lsusb
kind = "mouse"
svg = "g502-hero.svg"         # optional, else generic mouse SVG

[[buttons]]
cid = 0x0050                  # control ID from REPROG_CONTROLS_V4
label = "Left click"
svg_id = "btn-left"

[quirks]
# only for proven firmware bugs, each with a comment + issue link
```

## Rules
- Data describes presentation and quirks; capabilities always come from discovery.
- Schema validated via `serde` + a test that loads ALL files in `data/devices/`.
- Unknown device -> generic layout, buttons labeled by CID/task from the device.
- Contribution flow: `squeak dump-device` output (no serials) + TOML + SVG -> PR template.
- Test: every device TOML has unique PIDs and valid SVG ids.

# M2 Hardware Verification Guide

> **Hardware status:** Verified on a physical Logitech G502 SE HERO (`046d:c08b`, `HID_NAME` `"Logitech G502 HERO Gaming Mouse"`) on Fedora Linux. Hardware-dependent tests are never executed in CI.

## 1. Verify USB Vendor/Product ID
Confirm the USB VID:PID reported by the kernel (verified: `046d:c08b`):

```bash
lsusb | grep 046d
```

## 2. Check / Preview udev Permissions (`setup-udev --dry-run`)
Inspect the rule that grants unprivileged desktop-session access (`TAG+="uaccess"`) and verify whether `/dev/hidraw*` nodes are currently writable:

```bash
cargo run -p squeak-cli -- setup-udev --dry-run
```

If connected HID++ `hidraw` nodes report `permission denied`, install `/etc/udev/rules.d/70-squeak.rules` via PolicyKit (`pkexec`):

```bash
cargo run -p squeak-cli -- setup-udev
```

## 3. List Discovered HID++ Endpoints
Scan `/sys/class/hidraw` for vendor `0x046d` interfaces advertising HID++ report descriptors (`0xFF00..=0xFFFF` with `0x10`/`0x11`) and probe `IRoot::ping`:

```bash
cargo run -p squeak-cli -- list
cargo run -p squeak-cli -- list --json
```

Verified behavior:
- Boot mouse pointer node is ignored automatically.
- HID++ control node (e.g. `/dev/hidraw6`) reports `046d:c08b`, `Bus: USB`, HID++ protocol version `4.2`, matched name `G502 HERO`, and `"probe_status": { "state": "accessible" }` in `--json` (`accessible` in the table).

## 4. Inspect Capability Summary (`info`)
```bash
cargo run -p squeak-cli -- info
```

Verified output highlights:
- `Definition:       known TOML definition (mouse)`
- `Feature count:    20`
- Detected capability lines: `Device name/type (0x0005 v0)`, `Firmware info (0x0003 v2)`, `Adjustable DPI (0x2201 v1)`, `Report rate (0x8060 v0)`, `Color LED effects (0x8070 v3)`, `Onboard profiles (0x8100 v0)`.

## 5. Dump Full Runtime Feature Table (`dump-device`)
```bash
cargo run -p squeak-cli -- dump-device
cargo run -p squeak-cli -- dump-device --json
```

Verified checklist:
- JSON contains `"schema_version": 1`, `"vendor_id": "0x046d"`, `"product_id": "0xc08b"`, `"definition_match": "known"`, and 20 `"features"` starting at `"index": 0` (`"name": "IRoot"`).
- **Zero serial numbers** (`HID_UNIQ` / `serial`) or local `/dev/hidraw*` device paths appear anywhere in `dump-device --json` output.
- Notable finding for M3–M6: `FIRMWARE_INFO` is `v2`, lighting uses `COLOR_LED_EFFECTS` (`0x8070 v3`, not `0x8071`), and buttons use `ONBOARD_PROFILES` (`0x8100 v0`) + `MOUSE_BUTTON_SPY` (`0x8110 v0`) rather than `REPROG_CONTROLS_V4` (`0x1B04`).

# M2 Hardware Verification Guide (`[NEEDS-HW]`)

> **Hardware status:** All steps below require a physical Logitech G502 HERO connected on a Linux host (primary target: Fedora / Wayland / GNOME). None of the real hardware outputs below have been executed in CI.

## 1. Verify USB Vendor/Product ID
Confirm the USB VID:PID reported by the kernel (expected: `046d:c08b` for wired G502 HERO, `[NEEDS-HW]`):

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

Expected behavior (`[NEEDS-HW]`):
- Boot mouse pointer node (usually `/dev/hidraw0`) is ignored automatically.
- HID++ control node (usually `/dev/hidraw1`) reports `046d:c08b`, `Bus: USB`, HID++ protocol version (e.g. `4.2`), matched name `G502 HERO`, and `"probe_status": { "state": "accessible" }` in `--json` (`accessible` in the table).

## 4. Inspect Capability Summary (`info`)
```bash
cargo run -p squeak-cli -- info
```

Expected behavior (`[NEEDS-HW]`):
- `Definition:       known TOML definition (mouse)`
- Detected capability lines list discovered runtime features formatted as `<Label> (0x<ID> v<Version>)`, such as `Adjustable DPI (0x2201 v1)`, `Report rate (0x8060 v0)`, `Programmable buttons (v4) (0x1B04 v...)`, `RGB effects (0x8071 v...)`, and `Onboard profiles (0x8100 v...)` (exact feature set & versions come from live discovery).

## 5. Dump Full Runtime Feature Table (`dump-device`)
```bash
cargo run -p squeak-cli -- dump-device
cargo run -p squeak-cli -- dump-device --json
```

Verification checklist (`[NEEDS-HW]`):
- Confirm JSON contains `"schema_version": 1`, `"vendor_id": "0x046d"`, `"product_id": "0xc08b"`, `"definition_match": "known"`, and non-empty `"features"` starting at `"index": 0` (`"name": "IRoot"`).
- Confirm **zero serial numbers** (`HID_UNIQ` / `serial`) or local `/dev/hidraw*` device paths appear anywhere in `dump-device --json` output.
- Keep the sanitized JSON dump for verifying M3 (`0x2201`/`0x8060`) and M5 (`0x1B04` CIDs vs `data/devices/g502-hero.toml`) feature versions.

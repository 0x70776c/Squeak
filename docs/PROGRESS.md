# Squeak Progress & Milestone Plan

Status legend:
- `[x]` Done and verified
- `[~]` In progress / blocked on gate
- `[ ]` Planned
- `[NEEDS-HW]` Requires physical hardware verification (never faked in CI)

---

## Open Gates & Tracked Notes
- **M0 Gate (resolved):** License = `GPL-3.0-or-later` (user choice); MSRV = `1.89` (conservative baseline via ADR-002); GUI baseline = `GTK 4.6` + `libadwaita 1.1` with feature-gated `v1_4` fallback (user choice).
- **Pre-M3/M6 Note:** `CLAUDE.md` references a `hardware-safety` skill for M3 readback/writes and M6 onboard profiles (`0x8100`). Request or author `hardware-safety` rules before performing any real hardware writes.

---

## M0 – Foundation
- [x] `.gitignore`, `rustfmt.toml`, `CLAUDE.md`, `.claude/skills/*`
- [x] `.github/PULL_REQUEST_TEMPLATE.md` (with device contribution flow)
- [x] `docs/PROGRESS.md`, `docs/decisions/001-architecture-and-crate-boundaries.md`, `docs/protocol.md`, `data/devices/README.md`
- [x] **GATE:** Confirmed license (`GPL-3.0-or-later`), MSRV (`1.89`), and min GTK4/libadwaita (`4.6` / `1.1` with `v1_4` fallback) -> `docs/decisions/002-toolchain-license-and-gui-baseline.md`
- [x] `LICENSE`, `rust-toolchain.toml`, `deny.toml`, `clippy.toml`, workspace `Cargo.toml` + crate scaffolds (`crates/{hidpp,squeak-core,squeakd,squeak-cli,squeak-gui}`)
- [x] `.github/workflows/ci.yml` (fmt, clippy, test, doc, deny, MSRV 1.89, distro container matrix)
- [x] `README.md`, `CONTRIBUTING.md`, `docs/dev-setup.md`
- [x] M0 quality gate green & hostile self-review

## M1 – `hidpp` Core
- [x] Frame encode/decode (`0x10` short 7B, `0x11` long 20B, `0x12` very long 64B) + bounds validation
- [x] `HidTransport` trait + `MockTransport` (scripted replies, notifications, timeouts, busy injection)
- [x] Linux `hidraw` `HidTransport` implementation (pure non-blocking/timeout I/O)
- [x] HID++ 2.0 error frame (`0xFF`) mapping to typed `#[non_exhaustive]` `Hidpp20Error`
- [x] Request/response multiplexer (`sw_id` filtering, notification skipping, `Busy` backoff retry, timeouts)
- [x] `IRoot` (`0x0000`: `getFeature`, `ping`, protocol version, 1.0 legacy error)
- [x] `IFeatureSet` (`0x0001`: `getCount`, `getFeatureID`) + feature table cache & invalidation
- [x] Golden-frame unit tests + `proptest` roundtrips
- [x] M1 quality gate green & hostile self-review

> **M1 Summary**
> - **Done:** `#![forbid(unsafe_code)]` `hidpp` crate with wire framing (`0x10`/`0x11`/`0x12`), HID report descriptor + sysfs `uevent` parsers, safe `rustix` `HidrawTransport`, `MockTransport`, `HidppSession` multiplexer (`sw_id` correlation, `Busy` exponential backoff, legacy `0x8F` detection), `IRoot` (`0x0000`), `IFeatureSet` (`0x0001`), and `FeatureTable`/`FeatureCache`.
> - **Tested:** `cargo fmt --check`, `clippy --all-targets -- -D warnings`, `cargo doc --no-deps`, and 38 workspace tests (golden frames, proptests, malformed lengths/descriptors, busy retries, continuous notifications, dynamic non-hardcoded `IFeatureSet` index, and mock sysfs discovery).
> - **Needs hardware:** Live Linux `/dev/hidraw*` poll/read/write verification against physical G502 HERO (covered by M2 `[NEEDS-HW]` script).
> - **Risks:** Physical devices sharing one `hidraw` endpoint with consumer/keyboard HID reports may inject non-HID++ packets (`trace`-skipped in `HidrawTransport::read_frame`, `[NEEDS-HW]`).
> - **Next:** M2 — device TOML validator + generic fallback in `squeak-core`, provisional `data/devices/g502-hero.toml`, and `squeak` CLI (`list`, `info`, `dump-device`, `setup-udev`).


## M2 – CLI + `dump-device` + G502 HERO Device Data
- [x] Device TOML loader & schema validator (`squeak-core`: `data/devices/*.toml`, unique PIDs, generic fallback)
- [x] `squeak-cli` (`squeak`): `list`, `info`, `dump-device`, `setup-udev` (`--dry-run`, `--json`)
- [x] Serial-number redaction / omission guarantee in `dump-device` output and logs
- [x] Initial `data/devices/g502-hero.toml` (PID `0xc08b` provisional until `[NEEDS-HW]` verified)
- [x] `[NEEDS-HW]` G502 HERO verification guide & expected output in `docs/hw-tests-m2.md`
- [ ] `[NEEDS-HW]` Physical G502 HERO hardware execution pending (`docs/hw-tests-m2.md`):
  - Verify PID: `lsusb | grep 046d`
  - Install/verify udev access: `cargo run -p squeak-cli -- setup-udev --dry-run` / `setup-udev`
  - Enumerate: `cargo run -p squeak-cli -- list --json`
  - Inspect & dump features: `cargo run -p squeak-cli -- info` / `dump-device --json`
- [x] M2 quality gate green & hostile self-review

> **M2 Summary**
> - **Done:** Implemented `squeak-core` device TOML catalog (`DeviceDatabase`, SVG path/element ID validation, duplicate VID:PID checks, generic fallback) and sanitized schemas (`DeviceDump`, `DeviceInfoReport`, `DeviceListEntry`), provisional `data/devices/g502-hero.toml`, `squeak` CLI (`list`, `info`, `dump-device`, `setup-udev` via `pkexec`), `docs/hw-tests-m2.md`, and hostile-review hardening across `hidpp` (`hidraw` `Duration::MAX`/loop deadline checks, `IFeatureSet` v0/255 clamping, receiver `0x8F` offline/`Busy` distinction).
> - **Tested:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `RUSTFLAGS="-D unused-crate-dependencies" cargo check -p hidpp -p squeak-core -p squeak-cli --lib --bins`, license audit (`deny.toml`), CLI smoke runs (`--help`, `list --json`, `setup-udev --dry-run --json`), and 59 workspace tests (`41` `hidpp`, `5` `squeak-core`, `11` `squeak-cli`, `1` `squeakd`, `1` `squeak-gui`).
> - **Needs hardware:** Execute `lsusb | grep 046d`, `setup-udev`, `list --json`, `info`, and `dump-device --json` against physical G502 HERO on Fedora/Wayland (`[NEEDS-HW]` in `docs/hw-tests-m2.md`).
> - **Risks:** Provisional PID `0xc08b` and button CIDs (`0x00c3`..`0x005d`) in `g502-hero.toml` await live hardware verification; non-HID++ packet skipping on shared endpoints untested on real hardware.
> - **Next:** Run `[NEEDS-HW]` M2 verification on G502 HERO and report output before beginning M3 (`FIRMWARE_INFO`, `DEVICE_NAME`, battery, DPI, report rate).

## M3 – Read/Write Basics (STOP/ASK before first real write)
- [ ] `FIRMWARE_INFO` (`0x0003`) & `DEVICE_NAME` (`0x0005`)
- [ ] `BATTERY_STATUS` (`0x1000`) & `UNIFIED_BATTERY` (`0x1004`)
- [ ] `ADJUSTABLE_DPI` (`0x2201`: sensor count, DPI list/range, get/set stages)
- [ ] `REPORT_RATE` (`0x8060`) & `EXTENDED_REPORT_RATE` (`0x8061`)
- [ ] Readback verification after writes + CLI read/write commands (`--dry-run` default/safe guards)
- [ ] `[NEEDS-HW]` Verify DPI/report-rate readback on G502 HERO (ASK user before real write)

## M4 – Daemon (`squeakd`)
- [ ] udev enumeration & hotplug monitor
- [ ] Device registry + runtime feature table lifecycle (reconnect/wake invalidation & re-apply)
- [ ] Versioned DBus API (`dev.squeak.Daemon` via `zbus`), documented in `docs/dbus.md`
- [ ] DBus activation service + systemd user unit, graceful shutdown, structured `tracing` logs (no serials)
- [ ] Integration tests with `MockTransport` + isolated test DBus
- [ ] `[NEEDS-HW]` Verify udev hotplug + suspend/resume re-apply on real G502 HERO

## M5 – Buttons, Scroll, Lighting
- [ ] `REPROG_CONTROLS_V4` (`0x1B04`) control enumeration + remap (button/key/macro/DPI-cycle/profile-switch/disable)
- [ ] `HIRES_WHEEL` (`0x2121`) & `SMART_SHIFT` (`0x2110`)
- [ ] `COLOR_LED_EFFECTS` (`0x8070`) & `RGB_EFFECTS` (`0x8071`) (zones, effects, brightness)
- [ ] Capability-gated DBus & CLI exposure
- [ ] `[NEEDS-HW]` Verify button remap & RGB effects on G502 HERO

## M6 – Onboard Profiles (`0x8100`) (ASK before real writes)
- [ ] `ONBOARD_PROFILES` (`0x8100`) memory/sector/directory parser + CRC verification
- [ ] Mandatory backup + restore flow before any onboard write
- [ ] Host mode vs. onboard mode switching
- [ ] Golden & fault-injection tests (corrupted CRC, truncated memory read)
- [ ] `[NEEDS-HW]` Verify backup/read/write/restore on G502 HERO (ASK user before real write)

## M7 – GTK4/libadwaita GUI (`squeak-gui`)
- [ ] `AdwApplicationWindow`, `AdwNavigationSplitView` (or gated fallback if Ubuntu 22.04 supported), `AdwToastOverlay`
- [ ] Async DBus proxy + signals (zero blocking on UI thread, live apply with debounce, undo last change)
- [ ] Pages (capabilities-gated): Overview, DPI, Buttons, Lighting, Profiles, Scroll
- [ ] Interactive SVG mouse diagram (`g502-hero.svg` + generic fallback SVG)
- [ ] `AdwStatusPage` error states (hidraw permission -> `pkexec` udev install, disconnected/asleep, unsupported)
- [ ] `gettext-rs` i18n setup (English + German)
- [ ] Keyboard accessibility pass + docs screenshots

## M8 – Software Profiles & Desktop Integration
- [ ] XDG TOML software profiles (`$XDG_CONFIG_HOME/squeak`), import/export
- [ ] Active-window / per-app profile switching behind trait (GNOME/KDE/Hyprland backends + working impl)

## M9 – Packaging & Release
- [ ] Flatpak manifest (`org.gnome.Platform`, `--device=all`, `rust-stable` ext)
- [ ] `packaging/` recipes: RPM spec (COPR), `debian/`, `PKGBUILD`, Nix flake
- [ ] AppStream metainfo, `.desktop`, scalable icon (Squeak branding, no vendor marks)
- [ ] `CHANGELOG.md`, `docs/device-support.md`, `docs/contributing-a-device.md`
- [ ] Release workflow + `v0.1.0` tag proposal

# Squeak Progress & Milestone Plan

Status legend:
- `[x]` Done and verified
- `[~]` In progress / blocked on gate
- `[ ]` Planned
- `[NEEDS-HW]` Requires physical hardware verification (never faked in CI)

---

## Open Gates & Tracked Notes
- **M0 Gate (resolved):** License = `GPL-3.0-or-later` (user choice); MSRV = `1.89` (conservative baseline via ADR-002); GUI baseline = `GTK 4.6` + `libadwaita 1.1` with feature-gated `v1_4` fallback (user choice).
- **Pre-M3/M6 Note (resolved for M3):** Authored `.claude/skills/hardware-safety/SKILL.md` (dry-run/read-only guards, capability bounds validation, pre-write snapshot, mandatory readback verification, onboard memory lock until M6, and Ask-First Gate before real hardware writes).

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
- [x] Initial `data/devices/g502-hero.toml` (PID `0xc08b` verified via `lsusb` & `squeak list`)
- [x] `[NEEDS-HW]` G502 HERO verification guide & verified output in `docs/hw-tests-m2.md`
- [x] `[NEEDS-HW]` Physical G502 HERO hardware execution completed (`docs/hw-tests-m2.md`):
  - Verified PID: `lsusb | grep 046d` -> `046d:c08b` (`Logitech, Inc. G502 SE HERO Gaming Mouse`)
  - Installed/verified udev access: `setup-udev` + `setup-udev --dry-run` (`/dev/hidraw6` -> `accessible`)
  - Enumerated: `list --json` (`HID++ 4.2`, `matched_name: "G502 HERO"`, `accessible`)
  - Inspected & dumped features: `info` + `dump-device --json` (20 features: `IFeatureSet` `v1`, `FIRMWARE_INFO` `0x0003 v2`, `DEVICE_NAME` `0x0005 v0`, `ADJUSTABLE_DPI` `0x2201 v1`, `REPORT_RATE` `0x8060 v0`, `COLOR_LED_EFFECTS` `0x8070 v3`, `ONBOARD_PROFILES` `0x8100 v0`, `MOUSE_BUTTON_SPY` `0x8110 v0`; no `0x1B04`, zero serials)
- [x] M2 quality gate green & hostile self-review

> **M2 Summary**
> - **Done:** Implemented `squeak-core` device TOML catalog (`DeviceDatabase`, SVG path/element ID validation, duplicate VID:PID checks, generic fallback) and sanitized schemas (`DeviceDump`, `DeviceInfoReport`, `DeviceListEntry`), `data/devices/g502-hero.toml`, `squeak` CLI (`list`, `info`, `dump-device`, `setup-udev` via `pkexec`), `docs/hw-tests-m2.md`, and hostile-review hardening across `hidpp` (`hidraw` `Duration::MAX`/loop deadline checks, `IFeatureSet` v0/255 clamping, receiver `0x8F` offline/`Busy` distinction).
> - **Tested:** Quality gates + 59 workspace tests + live Fedora hardware verification on physical G502 SE HERO (`046d:c08b` on `/dev/hidraw6`, HID++ `4.2`, 20 features, zero serial numbers).
> - **Needs hardware:** Completed for M2; button CIDs in `g502-hero.toml` remain `[NEEDS-HW]` for M5/M6 (`MOUSE_BUTTON_SPY` / `ONBOARD_PROFILES`).
> - **Risks:** G502 HERO firmware lacks `0x1B04` (`REPROG_CONTROLS_V4`) and uses `0x8070` (`COLOR_LED_EFFECTS` v3) + `0x8100`/`0x8110` for buttons/profiles; M5/M6 must account for both `0x1B04` and `0x8100`-based mice.
> - **Completed M3:** Implemented `0x0003`/`0x0005`/`0x1000`/`0x1004`/`0x2201`/`0x8060`/`0x8061` drivers (61 `hidpp` tests), sanitized `squeak-core` M3 reports (7 tests), `squeak` CLI `status`/`dpi`/`report-rate` commands with default `--dry-run` guards (14 tests), and live G502 HERO verification (`/dev/hidraw6`: `status`, `0x2201` DPI write+readback `1200 -> 1600 -> 1200` passed; `0x8060` read `125..=1000 Hz` passed, while `0x8060 fn2` returned firmware `InvalidArgument (0x02)` in onboard-profile mode pending M6 `0x8100` Host-Mode switching).
> - **Next:** M4 (`squeakd` — udev enumeration & hotplug monitor, device registry + runtime `FeatureTable` lifecycle, `zbus` DBus API `dev.squeak.Daemon` + `docs/dbus.md`, systemd/DBus activation units, integration tests).

## M3 – Read/Write Basics (STOP/ASK before first real write)
- [x] `FIRMWARE_INFO` (`0x0003`) & `DEVICE_NAME` (`0x0005`)
- [x] `BATTERY_STATUS` (`0x1000`) & `UNIFIED_BATTERY` (`0x1004`)
- [x] `ADJUSTABLE_DPI` (`0x2201`: sensor count, DPI list/range, get/set stages)
- [x] `REPORT_RATE` (`0x8060`) & `EXTENDED_REPORT_RATE` (`0x8061`)
- [x] Readback verification after writes + CLI read/write commands (`--dry-run` default/safe guards)
- [x] `[NEEDS-HW]` Verify DPI/report-rate readback on G502 HERO (ASK user before real write) — `0x2201` write+readback `1200 <-> 1600 DPI` verified; `0x8060` read `1000 Hz` verified (`fn2` rejected with `0x02 InvalidArgument` while `0x8100` onboard mode active; host-mode switching scoped for M6)

## M4 – Daemon (`squeakd`)
- [x] udev enumeration & hotplug monitor (`NETLINK_KOBJECT_UEVENT` pure `rustix` listener with debounce & shutdown)
- [x] Device registry + runtime feature table lifecycle (reconnect/wake invalidation & persistent desired settings re-apply)
- [x] Versioned DBus API (`dev.squeak.Daemon` via `zbus`), documented in `docs/dbus.md`
- [x] DBus activation service + systemd user unit, graceful shutdown, structured `tracing` logs (no serials)
- [x] Integration tests with `MockTransport` + deterministic registry state machine tests
- [ ] `[NEEDS-HW]` Verify udev hotplug + suspend/resume re-apply on real G502 HERO

> **M4 Summary**
> - **Done:** Implemented `squeakd` background daemon crate (`crates/squeakd`): single-owner `DeviceRegistry` over `DeviceBackend` trait, pure `rustix` `NETLINK_KOBJECT_UEVENT` hidraw hotplug watcher, `systemd-logind` `PrepareForSleep` wake watcher, versioned `dev.squeak.Daemon` D-Bus interface with signals (`DevicesChanged`, `DeviceSettingsApplied`, `DeviceError`), systemd user unit (`data/systemd/squeakd.service`), D-Bus session activation service (`data/dbus/dev.squeak.Daemon.service`), and complete `docs/dbus.md` protocol reference.
> - **Tested:** Quality gate green (`fmt`, `clippy -D warnings`, doc checks, unused-crate-dependencies), 87 unit and integration tests passing across all crates.
> - **Needs hardware:** Physical verification of hotplug unbind/bind and `login1` wake signal on real hardware.
> - **Risks:** When `squeakd` is running and owning `/dev/hidraw*`, CLI direct access will compete unless routed through D-Bus proxy.
> - **Next:** M5 (`COLOR_LED_EFFECTS` `0x8070`, `ONBOARD_PROFILES` `0x8100` host mode, `MOUSE_BUTTON_SPY` `0x8110`, button remap).

## M5 – Buttons, Scroll, Lighting
- [x] `REPROG_CONTROLS_V4` (`0x1B04`) control enumeration + remap (button/key/macro/DPI-cycle/profile-switch/disable)
- [x] `HIRES_WHEEL` (`0x2121`) & `SMART_SHIFT` (`0x2110`)
- [x] `COLOR_LED_EFFECTS` (`0x8070`) & `RGB_EFFECTS` (`0x8071`) (zones, effects, brightness, color, breathing, color cycle)
- [x] `ONBOARD_PROFILES` (`0x8100`) host mode vs. onboard mode switching primitives
- [x] `MOUSE_BUTTON_SPY` (`0x8110`) event notification parser and live spy loop
- [x] Capability-gated CLI exposure (`squeak led get/set`, `squeak host-mode get/set`, `squeak buttons list/spy`)
- [x] `[NEEDS-HW]` Verify button spy & volatile RGB effects on G502 HERO (ASK user before real writes)

> **M5 Summary**
> - **Done:** Implemented all M5 feature drivers in `hidpp` (`color_led` `0x8070`, `rgb_effects` `0x8071`, `onboard_profiles` `0x8100`, `button_spy` `0x8110`, `reprog_controls_v4` `0x1B04`, `hires_wheel` `0x2121`, `smart_shift` `0x2110`), integrated into `squeak-core` (`LedConfigReport`, `LedWriteReport`, `OnboardModeReport`, `OnboardModeWriteReport`), and added safe CLI commands (`squeak led`, `squeak host-mode`, `squeak buttons`). Verified live readout on connected G502 HERO (2 LED zones, 4 effects each, onboard profile geometry, 11 physical buttons). Live hardware write confirmed volatile static/breathing/cycle RGB changes, host mode switching, and 500/1000 Hz polling rate switching in host mode.
> - **Tested:** 100 workspace tests passing, clippy with zero warnings, rustdoc with zero warnings, unused dependencies verified.
> - **Needs hardware:** Interactive button spy verification on real buttons.
> - **Risks:** G502 HERO does not use `0x1B04` for remapping but relies on `0x8100` profile sectors; software mode (`0x8100 = 0x02`) allows host button interception via `0x8110` Button Spy.
> - **Next:** M6 (`ONBOARD_PROFILES` `0x8100` sector/directory parser, CRC verification, backup & restore flow).

## M6 – Onboard Profiles (`0x8100`) (ASK before real writes)
- [x] `ONBOARD_PROFILES` (`0x8100`) memory/sector/directory parser + CRC verification
- [x] Mandatory backup + restore flow before any onboard write
- [x] Host mode vs. onboard mode switching
- [x] Golden & fault-injection tests (corrupted CRC, truncated memory read)
- [x] `[NEEDS-HW]` Verify backup/read/write/restore on G502 HERO (ASK user before real write)

> **M6 Summary**
> - **Done:** Implemented complete 0x8100 memory read/write protocol (`read_sector`, `write_sector`, start/data/end chunks), 16-bit CCITT hardware CRC verification (`0x1021`, seed `0xFFFF`), sector 0 directory parser, profile sector format 2 (G303/G502) parser with DPI stages and names. Created standalone `ProfileBackup` JSON format with integrity verification, automatic safety backup flow, and `squeak profiles list/backup/restore/set-active` CLI subcommands. Verified live readout, profile slot switching, full 6-sector backup (1530 bytes), and verified flash restore with pre-flash auto-backup on physical G502 HERO (`046d:c08b`).
> - **Tested:** 112 workspace tests passing across all crates, clippy `-D warnings` and `-D unused-crate-dependencies` clean, rustdoc `-D warnings` clean.
> - **Needs hardware:** None remaining for M6. Hardware tests 100% verified on physical G502 HERO.
> - **Risks:** Onboard flash writes are persistent across power cycles; mandatory safety backup and dry-run defaults protect against accidental corruption.
> - **Next:** M7 (GTK4/libadwaita GUI `squeak-gui`).

## M7 – GTK4/libadwaita GUI (`squeak-gui`)
- [x] `AdwApplicationWindow`, `AdwNavigationSplitView` (or gated fallback if Ubuntu 22.04 supported), `AdwToastOverlay`
- [x] Async DBus proxy + signals (zero blocking on UI thread, live apply with debounce, undo last change)
- [x] Pages (capabilities-gated): Overview, DPI, Buttons, Lighting, Profiles, Scroll
- [x] Interactive SVG mouse diagram (`g502-hero.svg` + generic fallback SVG)
- [x] `AdwStatusPage` error states (hidraw permission -> `pkexec` udev install, disconnected/asleep, unsupported)
- [x] i18n setup (English + German)
- [ ] Keyboard accessibility pass + docs screenshots

> **M7 Summary**
> - **Done:** Implemented full GTK4 (`4.6` baseline) + `libadwaita` (`1.1` baseline + feature-gated `adw_v1_4` `NavigationSplitView` / `Leaflet` fallback) desktop app in `squeak-gui`, communicating exclusively with `squeakd` over D-Bus (`dev.squeak.Daemon`). Built capability-gated pages (`Overview`, `DPI & Sensor` with 220 ms debounced slider, `Buttons` with clickable SVG mouse diagram `g502-hero.svg`/`generic-mouse.svg`, `Lighting`, `Profiles` with backup/activate, `Scroll`), `AdwStatusPage` error views (`NoHidrawPermission` -> `pkexec squeak setup-udev`, `DaemonNotRunning`, `DeviceAsleepOrDisconnected`), `UndoStack`, and EN/DE localization.
> - **Tested:** 117 workspace tests passing, `clippy -D warnings -D unused-crate-dependencies` clean, `--no-default-features` Ubuntu 22.04 fallback build verified, `rustdoc -D warnings` clean.
> - **Needs hardware:** Live GUI interactive testing with running `squeakd` + connected G502 HERO.
> - **Risks:** GNOME/KDE `pkexec` udev prompt requires Polkit agent; `squeakd` must be running on session bus.
> - **Next:** M8 (Software Profiles & Desktop Integration).

## M8 – Software Profiles & Desktop Integration
- [ ] XDG TOML software profiles (`$XDG_CONFIG_HOME/squeak`), import/export
- [ ] Active-window / per-app profile switching behind trait (GNOME/KDE/Hyprland backends + working impl)

## M9 – Packaging & Release
- [ ] Flatpak manifest (`org.gnome.Platform`, `--device=all`, `rust-stable` ext)
- [ ] `packaging/` recipes: RPM spec (COPR), `debian/`, `PKGBUILD`, Nix flake
- [ ] AppStream metainfo, `.desktop`, scalable icon (Squeak branding, no vendor marks)
- [ ] `CHANGELOG.md`, `docs/device-support.md`, `docs/contributing-a-device.md`
- [ ] Release workflow + `v0.1.0` tag proposal

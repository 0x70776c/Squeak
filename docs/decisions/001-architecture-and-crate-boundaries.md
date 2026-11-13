# ADR-001: Workspace Architecture and Crate Boundaries

- **Status:** Accepted
- **Date:** 2026-11-13

## Context
Squeak needs a safe, testable HID++ 2.0 stack usable from a background daemon, CLI, and GTK4 GUI on Linux without requiring root privileges or direct hardware access from unprivileged UI processes.

## Decision
1. **Five-crate workspace:**
   - `hidpp`: Pure HID++ 2.0 protocol framing, error mapping, discovery (`IRoot`/`IFeatureSet`), and feature modules over an abstract `HidTransport` trait (+ Linux `hidraw` and `MockTransport`). Zero GUI, udev, or DBus dependencies.
   - `squeak-core`: Shared domain models, device definition (`data/devices/*.toml`) loader/validator, generic fallback layout, and software profile schemas.
   - `squeakd`: Single owner of `hidraw` devices; handles udev hotplug, runtime feature caching, and exposes a versioned DBus interface (`dev.squeak.Daemon` via `zbus`).
   - `squeak-cli`: CLI client (`squeak`) talking to `squeakd` over DBus, plus local diagnostic/admin helpers (`dump-device`, `setup-udev`).
   - `squeak-gui`: GTK4/libadwaita UI communicating exclusively via DBus with zero protocol logic.
2. **Runtime capability discovery:** Never hardcode feature indices or branch logic on device model; `data/devices/*.toml` supplies UI labels, SVG mapping, and documented firmware quirks only.

## Consequences
- Protocol parsers, retry logic, and daemon state transitions can be tested exhaustively in CI without physical hardware.
- Multi-client contention on `hidraw` is prevented by funneling hardware access through `squeakd`.

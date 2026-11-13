# Squeak

Open-source Linux control panel and daemon for HID++ 2.0 mice and peripherals, built in Rust with a GTK4/libadwaita interface.

> **Status:** Early active development (see [docs/PROGRESS.md](docs/PROGRESS.md)). Hardware verification items are tracked under `[NEEDS-HW]`.

## Architecture

Squeak uses a five-crate Cargo workspace separating protocol handling, device data, hardware ownership, CLI diagnostics, and the desktop UI:

- **`hidpp`** (`crates/hidpp`): Pure HID++ 2.0 protocol framing, runtime feature discovery (`IRoot`/`IFeatureSet`), typed feature decoders, and an abstract `HidTransport` trait (Linux `hidraw` + deterministic mock). Zero GUI or D-Bus dependencies.
- **`squeak-core`** (`crates/squeak-core`): Shared domain types, data-driven `data/devices/*.toml` loader/validator, generic HID++ 2.0 device fallback, and profile schemas.
- **`squeakd`** (`crates/squeakd`): Single unprivileged owner of `hidraw` nodes with udev hotplug and a versioned D-Bus API (`dev.squeak.Daemon` via `zbus`).
- **`squeak-cli`** (`crates/squeak-cli`, binary `squeak`): Command-line interface for device discovery, `dump-device`, udev rule setup, and control.
- **`squeak-gui`** (`crates/squeak-gui`): GTK4/libadwaita control panel (minimum GTK 4.6 / libadwaita 1.1 with feature-gated fallbacks) talking exclusively over D-Bus.

## Design Principles

- **HID++ 2.0+ only:** Legacy HID++ 1.0 devices are out of scope and fail gracefully during discovery.
- **Runtime discovery:** Feature indices and capabilities are always resolved at runtime via `IRoot` (`0x0000`) and `IFeatureSet` (`0x0001`), never hardcoded per model.
- **Data-driven UI & quirks:** `data/devices/*.toml` provides presentation labels, SVG button mappings, and documented firmware quirks; unknown HID++ 2.0 devices run in generic mode.
- **Safety & privacy:** No root daemon (`uaccess` udev rule via `squeak setup-udev`), no serial numbers stored in dumps or logs, and readback verification before/after hardware writes.

## Quick Start

Requires Rust **1.89+** (Edition 2024). See [docs/dev-setup.md](docs/dev-setup.md) for distro package prerequisites.

```bash
cargo test --workspace --all-features
cargo run -p squeak-cli -- --help
cargo run -p squeak-cli -- setup-udev --dry-run
cargo run -p squeak-cli -- list
cargo run -p squeak-cli -- info
cargo run -p squeak-cli -- dump-device --json
```

See [docs/hw-tests-m2.md](docs/hw-tests-m2.md) for `[NEEDS-HW]` hardware verification steps and expected output on physical devices.

## License

Licensed under the [GNU General Public License v3.0 or later](LICENSE) (`GPL-3.0-or-later`).

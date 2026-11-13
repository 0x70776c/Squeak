# Squeak

Open-source Linux control panel and daemon for HID++ 2.0 mice and peripherals, built in Rust with a GTK4/libadwaita interface.

> **Status:** **v0.1.0** — Core milestones **M0–M9** are complete (`hidpp`, `squeak-core`, `squeakd`, `squeak-cli`, `squeak-gui`, XDG software profiles, and packaging). See [CHANGELOG.md](CHANGELOG.md), [docs/device-support.md](docs/device-support.md), and [docs/PROGRESS.md](docs/PROGRESS.md).

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
# Build and run the full workspace test suite
cargo test --workspace --all-features

# Install udev rules for unprivileged hidraw access (preview first, then apply)
cargo run -p squeak-cli -- setup-udev --dry-run
cargo run -p squeak-cli -- setup-udev

# Start the background daemon and launch the GTK4/libadwaita GUI
cargo run -p squeakd &
cargo run -p squeak-gui

# Or use the CLI directly
cargo run -p squeak-cli -- list
cargo run -p squeak-cli -- status
cargo run -p squeak-cli -- sw-profiles list
```

See [docs/device-support.md](docs/device-support.md) for supported hardware and [docs/contributing-a-device.md](docs/contributing-a-device.md) to add new device definitions.

## License

Licensed under the [GNU General Public License v3.0 or later](LICENSE) (`GPL-3.0-or-later`).

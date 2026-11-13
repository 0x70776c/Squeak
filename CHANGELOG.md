# Changelog

All notable changes to Squeak are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-13

### Added
- **HID++ 2.0 Protocol Crate (`hidpp`):** `#![forbid(unsafe_code)]` implementation with descriptor-first hidraw discovery, dynamic `IRoot`/`IFeatureSet` feature table caching, `0x2201` Adjustable DPI, `0x8060`/`0x8061` Report Rate, `0x8070` Color LED Effects, `0x8071` RGB Effects, `0x8100` Onboard Profiles (with CRC-CCITT sector verification), `0x8110`/`0x1B04` button remapping and spy mode, `0x2110`/`0x2121` scroll wheel controls, and `0x1000`/`0x1004` battery reporting.
- **Background Daemon (`squeakd`):** Single-owner `hidraw` manager exposing `dev.squeak.Daemon` over the user session D-Bus, with `NETLINK_KOBJECT_UEVENT` hotplug monitoring, `systemd-logind` `PrepareForSleep` wake re-apply, and active-window per-app software profile auto-switching (GNOME, KDE Plasma, Hyprland).
- **Command-Line Interface (`squeak`):** `list`, `info`, `status`, `dpi`, `report-rate`, `led`, `host-mode`, `buttons`, `profiles`, `sw-profiles`, `dump-device`, and `setup-udev` (all write commands default to `--dry-run`, requiring `--apply` to write hardware).
- **Desktop GUI (`squeak-gui`):** GTK4 + libadwaita application with Overview, DPI & Sensor (up to 6 configurable DPI presets), Buttons (interactive SVG mouse diagram), Lighting (interactive color picker), Profiles (onboard backup & restore), Scroll, EN/DE localization, and Undo/Redo stack.
- **Packaging & Desktop Integration:** `.desktop` entry, AppStream metainfo (`dev.squeak.Squeak.metainfo.xml`), scalable & PNG icons, Flatpak manifest, RPM spec, Debian packaging, Arch `PKGBUILD`, and Nix `flake.nix`.

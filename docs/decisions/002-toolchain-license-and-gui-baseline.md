# ADR-002: License, MSRV, and Minimum GTK4/libadwaita Baseline

- **Status:** Accepted
- **Date:** 2026-11-13

## Context
Milestone 0 requires fixing the project license, Minimum Supported Rust Version (MSRV), and minimum GUI toolkit target across Fedora, Ubuntu LTS, Debian, and Arch.

## Decision
1. **License:** `GPL-3.0-or-later` across the entire workspace (`LICENSE` = GNU GPL v3).
2. **MSRV:** `1.89` (Rust 2024 edition). User skipped MSRV selection; `1.89` is the conservative baseline supporting `zbus` 6.x (`1.89`), `proptest` 1.12 (`1.88`), `tokio` 1.x (`1.85`), and `gtk4` 0.10 (`1.83`), while remaining readily available via Ubuntu 24.04 (`rustc-1.89`), Fedora, Arch, and `rustup` in CI.
3. **GUI Baseline:** Minimum `GTK 4.6` + `libadwaita 1.1` (`gtk4` `v4_6`, `libadwaita` `v1_1` Cargo features) with feature-gated `v1_4` widgets (e.g. `AdwNavigationSplitView` vs `AdwLeaflet` fallback) so native builds remain viable on Ubuntu 22.04 LTS alongside Fedora/Debian/Arch and Flatpak. `squeak-gui` links `gtk4`/`libadwaita` starting in M7 so headless M0–M2 development and tests compile without GUI dev headers.

## Consequences
- `deny.toml` permits GPL-3.0-compatible licenses; CI validates MSRV `1.89` and all target distro containers.

---
name: distro-packaging
description: Use for packaging, CI matrix, udev rules, systemd/DBus activation, Flatpak, and multi-distro compatibility.
---

# Packaging rules
- Primary: Flatpak (`org.gnome.Platform`, `rust-stable` ext). hidraw needs `--device=all`.
  Flatpak can't install udev -> `squeak setup-udev` (pkexec; writes `/etc/udev/rules.d/70-squeak.rules`,
  `udevadm control --reload && udevadm trigger`, verifies access).
- Secondary: RPM spec (COPR), `debian/`, `PKGBUILD`, Nix flake. Keep them in `packaging/`.
- udev rule: `KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="046d", TAG+="uaccess"`
  (consider hid-level match for hidraw nodes of receivers).
- Daemon: DBus-activated (`dev.squeak.Daemon.service`) + optional systemd user unit; works standalone.
- XDG paths only (`$XDG_CONFIG_HOME/squeak`). No distro-specific paths.
- CI matrix (GitHub Actions, containers): `fedora:latest`, `ubuntu:22.04`, `ubuntu:24.04`, `debian:stable`,
  `archlinux` -> build + test. Separate job: Flatpak build. HW tests never in CI.
- Local multi-distro: `distrobox`/`podman` recipes in `docs/dev-setup.md`.
- Fedora dev deps: `dnf install gtk4-devel libadwaita-devel systemd-devel hidapi-devel`
- Release: cargo-dist or GitHub release workflow, signed tags, CHANGELOG (keep-a-changelog).

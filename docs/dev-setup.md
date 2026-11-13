# Development Environment Setup

Squeak targets **Rust 1.89+** (Edition 2024) and Linux desktop distributions with **GTK >= 4.6** and **libadwaita >= 1.1** (GUI crates wired in M7).

## System Packages

### Fedora (primary development target)
```bash
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel systemd-devel hidapi-devel
```

### Ubuntu (22.04 LTS / 24.04 LTS) & Debian (stable)
```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev libhidapi-dev
```

### Arch Linux
```bash
sudo pacman -S --needed base-devel pkgconf gtk4 libadwaita systemd-libs hidapi
```

## Multi-Distro Container Testing (`podman` / `distrobox`)

Reproduce CI builds locally against target distributions:

```bash
# Example: Fedora latest
podman run --rm -it -v "$PWD":/work:Z -w /work fedora:latest bash -lc '
  dnf install -y gcc pkgconf-pkg-config curl gtk4-devel libadwaita-devel systemd-devel hidapi-devel &&
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.89.0 &&
  . "$HOME/.cargo/env" &&
  cargo test --workspace --all-features
'

# Example: Ubuntu 22.04 (verifies older GTK 4.6 / libadwaita 1.1 compatibility)
distrobox create --name squeak-u22 --image ubuntu:22.04
distrobox enter squeak-u22
```

## Hardware Access (`hidraw` udev rule)

Never run `squeakd` or `squeak-gui` as root. To grant the active desktop user unprivileged `hidraw` access:

```bash
cargo run -p squeak-cli -- setup-udev --dry-run
cargo run -p squeak-cli -- setup-udev
```

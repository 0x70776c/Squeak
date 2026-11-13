# Squeak – Linux HID++ 2.0 device control panel (Rust)

## Mission
Open-source G-Hub/Options+-style app for ALL Logitech HID++ 2.0 devices (mice first).
Linux-first, multi-distro (Fedora primary, Ubuntu LTS, Debian, Arch). Clean GTK4/libadwaita UI.

## Non-negotiables
- Workspace: `crates/{hidpp, squeak-core, squeakd, squeak-cli, squeak-gui}`; `data/`; `docs/`
- `hidpp` has no GUI/DBus deps; transport is a trait (`HidTransport`) with real + mock impl.
- Daemon `squeakd` is the single owner of hidraw; GUI/CLI talk via DBus (`dev.squeak.Daemon`, `zbus`).
- Features discovered at runtime (`IRoot`/`IFeatureSet`). NEVER hardcode feature indices or branch on model.
- Device support is data (`data/devices/*.toml`). Unknown HID++ 2.0 device => generic mode.
- HID++ 1.0 legacy: out of scope.
- No "Logitech"/"Logi"/"G HUB" in app name/branding.
- License: decided in milestone 0. No copied GPL code unless license is GPL-3.0-or-later.

## Definition of Done (every milestone)
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` green
- New logic has unit tests (mock transport); protocol code has golden-frame tests
- No `unwrap()/expect()` outside tests; no `unsafe` without `// SAFETY:` comment
- Docs updated (`docs/`), `docs/PROGRESS.md` updated, atomic conventional commits
- Hardware-dependent parts marked `[NEEDS-HW]` in `docs/PROGRESS.md` with exact test commands

## Working rules
- Read relevant skill in `.claude/skills/` BEFORE touching that area.
- Plan in `docs/PROGRESS.md` (checklist). Update it after every task.
- Work autonomously through milestones; stop only at the "Ask first" gates.
- Ask first: license, new heavy dependencies, any write to onboard memory on real hardware,
  architecture changes.
- Prefer small, reviewable commits. Never leave the tree red.
- Test device: G502 HERO (verify PID via `lsusb | grep 046d`), Fedora, Wayland/GNOME.

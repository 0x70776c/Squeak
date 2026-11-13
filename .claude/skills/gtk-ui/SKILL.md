---
name: gtk-ui
description: Use when building or changing the GTK4/libadwaita GUI (squeak-gui): pages, widgets, mouse diagram, error states.
---

# UI rules
- libadwaita only (`AdwApplicationWindow`, `AdwNavigationSplitView`, `AdwPreferencesPage/Group`,
  `AdwToastOverlay`, `AdwStatusPage`). Follow GNOME HIG; light/dark automatic.
- Pages: Overview, DPI, Buttons, Lighting, Profiles, Scroll. Show only capabilities the
  device reports.
- GUI contains no protocol logic; all state via DBus proxy + signals. UI never blocks:
  async calls, spinners, toasts for results.
- Live apply with debounce; explicit "Save to profile". Undo for last change.
- Mouse diagram: SVG with clickable ids from device TOML; fallback generic.
- Error states as `AdwStatusPage` with action buttons:
  - no hidraw permission -> "Install udev rule" (runs `squeak setup-udev` via pkexec)
  - device asleep/disconnected -> retry
  - unsupported feature -> explanatory hint, no crash
- Accessibility: keyboard navigation, accessible labels, no color-only signaling.
- Min versions: pin GTK4/libadwaita minimum in Cargo features; gate newer widgets with fallback
  (Ubuntu 22.04 has older libadwaita).
- i18n-ready from the start (`gettext-rs`), German + English.

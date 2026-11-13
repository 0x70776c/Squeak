//! Active-window detection backends for per-application software profile switching.
//!
//! Provides an [`ActiveWindowProvider`] trait with implementations for
//! Hyprland, KDE Plasma, and GNOME, plus an [`AutoWindowProvider`] that
//! selects the appropriate backend from the current desktop session.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use squeak_core::{SoftwareProfile, match_profile_for_window};

/// Snapshot of the currently focused desktop window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveWindow {
    /// Application identifier / WM class (e.g. `"org.mozilla.firefox"`, `"steam_app_730"`).
    pub app_id: String,
    /// Human-readable window title.
    pub title: String,
}

/// Backend interface for querying the currently focused window.
pub trait ActiveWindowProvider: Send + Sync {
    /// Short backend name for diagnostics and logging.
    fn name(&self) -> &'static str;

    /// Returns the currently focused window, or `None` if unavailable.
    fn active_window(&self) -> Option<ActiveWindow>;
}

/// Hyprland active-window provider using `$HYPRLAND_INSTANCE_SIGNATURE` IPC socket
/// with `hyprctl -j activewindow` fallback.
#[derive(Debug, Default, Clone, Copy)]
pub struct HyprlandWindowProvider;

impl HyprlandWindowProvider {
    /// Returns `true` when running inside a Hyprland session.
    #[must_use]
    pub fn is_available() -> bool {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
    }

    fn socket_path() -> Option<PathBuf> {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        if sig.is_empty() {
            return None;
        }
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let candidate = PathBuf::from(runtime_dir)
                .join("hypr")
                .join(&sig)
                .join(".socket.sock");
            if candidate.exists() {
                return Some(candidate);
            }
        }
        let legacy = PathBuf::from("/tmp/hypr").join(sig).join(".socket.sock");
        if legacy.exists() { Some(legacy) } else { None }
    }

    fn query_via_socket() -> Option<String> {
        let path = Self::socket_path()?;
        let mut stream = UnixStream::connect(path).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
        stream.write_all(b"j/activewindow").ok()?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf).ok()?;
        Some(buf)
    }

    fn query_via_hyprctl() -> Option<String> {
        let output = Command::new("hyprctl")
            .args(["-j", "activewindow"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }
}

impl ActiveWindowProvider for HyprlandWindowProvider {
    fn name(&self) -> &'static str {
        "hyprland"
    }

    fn active_window(&self) -> Option<ActiveWindow> {
        if !Self::is_available() {
            return None;
        }
        let raw = Self::query_via_socket().or_else(Self::query_via_hyprctl)?;
        parse_hyprland_activewindow_json(&raw)
    }
}

/// Parses `hyprctl -j activewindow` / IPC `j/activewindow` JSON output.
#[must_use]
pub fn parse_hyprland_activewindow_json(raw: &str) -> Option<ActiveWindow> {
    let val: Value = serde_json::from_str(raw).ok()?;
    let obj = val.as_object()?;
    let app_id = obj
        .get("class")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| obj.get("initialClass").and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| obj.get("initialTitle").and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_string();

    if app_id.is_empty() && title.is_empty() {
        return None;
    }
    Some(ActiveWindow { app_id, title })
}

/// KDE Plasma active-window provider using `kdotool` (Wayland/X11) or `qdbus6`/`qdbus`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KdeWindowProvider;

impl KdeWindowProvider {
    /// Returns `true` when running inside a KDE Plasma session.
    #[must_use]
    pub fn is_available() -> bool {
        if std::env::var_os("KDE_FULL_SESSION").is_some() {
            return true;
        }
        std::env::var("XDG_CURRENT_DESKTOP")
            .map(|d| d.to_ascii_lowercase().contains("kde"))
            .unwrap_or(false)
    }

    fn query_via_kdotool() -> Option<ActiveWindow> {
        let id_out = Command::new("kdotool")
            .arg("getactivewindow")
            .output()
            .ok()?;
        if !id_out.status.success() {
            return None;
        }
        let win_id = String::from_utf8(id_out.stdout).ok()?.trim().to_string();
        if win_id.is_empty() {
            return None;
        }

        let class_out = Command::new("kdotool")
            .args(["getwindowclassname", &win_id])
            .output()
            .ok();
        let title_out = Command::new("kdotool")
            .args(["getwindowname", &win_id])
            .output()
            .ok();

        let app_id = class_out
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let title = title_out
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if app_id.is_empty() && title.is_empty() {
            None
        } else {
            Some(ActiveWindow { app_id, title })
        }
    }

    fn query_via_qdbus() -> Option<ActiveWindow> {
        for bin in ["qdbus6", "qdbus"] {
            let Ok(out) = Command::new(bin)
                .args(["org.kde.KWin", "/KWin", "queryWindowInfo"])
                .output()
            else {
                continue;
            };
            if !out.status.success() {
                continue;
            }
            if let Ok(text) = String::from_utf8(out.stdout)
                && let Some(win) = parse_kwin_window_info(&text)
            {
                return Some(win);
            }
        }
        None
    }
}

impl ActiveWindowProvider for KdeWindowProvider {
    fn name(&self) -> &'static str {
        "kde"
    }

    fn active_window(&self) -> Option<ActiveWindow> {
        if !Self::is_available() {
            return None;
        }
        Self::query_via_kdotool().or_else(Self::query_via_qdbus)
    }
}

/// Parses `qdbus org.kde.KWin /KWin queryWindowInfo` key/value lines.
#[must_use]
pub fn parse_kwin_window_info(raw: &str) -> Option<ActiveWindow> {
    let mut app_id = String::new();
    let mut title = String::new();
    for line in raw.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        match key {
            "resourceClass" | "desktopFile" if app_id.is_empty() => {
                app_id = val.to_string();
            }
            "caption" if title.is_empty() => {
                title = val.to_string();
            }
            _ => {}
        }
    }
    if app_id.is_empty() && title.is_empty() {
        None
    } else {
        Some(ActiveWindow { app_id, title })
    }
}

/// GNOME active-window provider using `gdbus` (FocusedWindow extension or Shell Eval).
#[derive(Debug, Default, Clone, Copy)]
pub struct GnomeWindowProvider;

impl GnomeWindowProvider {
    /// Returns `true` when running inside a GNOME session.
    #[must_use]
    pub fn is_available() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP")
            .map(|d| d.to_ascii_lowercase().contains("gnome"))
            .unwrap_or(false)
    }

    fn query_focused_window_extension() -> Option<ActiveWindow> {
        let out = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.gnome.Shell",
                "--object-path",
                "/org/gnome/shell/extensions/FocusedWindow",
                "--method",
                "org.gnome.shell.extensions.FocusedWindow.Get",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        parse_gnome_gdbus_json_output(&text)
    }

    fn query_shell_eval() -> Option<ActiveWindow> {
        let out = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.gnome.Shell",
                "--object-path",
                "/org/gnome/Shell",
                "--method",
                "org.gnome.Shell.Eval",
                "global.display.focus_window ? JSON.stringify({wm_class: global.display.focus_window.get_wm_class(), title: global.display.focus_window.get_title()}) : ''",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        parse_gnome_gdbus_json_output(&text)
    }
}

impl ActiveWindowProvider for GnomeWindowProvider {
    fn name(&self) -> &'static str {
        "gnome"
    }

    fn active_window(&self) -> Option<ActiveWindow> {
        if !Self::is_available() {
            return None;
        }
        Self::query_focused_window_extension().or_else(Self::query_shell_eval)
    }
}

/// Parses `gdbus call` output wrapping a JSON object (e.g. `(true, '{"wm_class":"firefox","title":"..."}')`).
#[must_use]
pub fn parse_gnome_gdbus_json_output(raw: &str) -> Option<ActiveWindow> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    let slice = &raw[start..=end];
    // gdbus escapes quotes inside single-quoted GVariant strings in some setups.
    let normalized = slice.replace("\\\"", "\"");
    let val: Value = serde_json::from_str(&normalized).ok()?;
    let obj = val.as_object()?;
    let app_id = obj
        .get("wm_class")
        .or_else(|| obj.get("app_id"))
        .or_else(|| obj.get("wm_class_instance"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    if app_id.is_empty() && title.is_empty() {
        None
    } else {
        Some(ActiveWindow { app_id, title })
    }
}

/// Auto-detecting active-window provider supporting Hyprland, KDE Plasma, and GNOME.
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoWindowProvider;

impl ActiveWindowProvider for AutoWindowProvider {
    fn name(&self) -> &'static str {
        if HyprlandWindowProvider::is_available() {
            "hyprland"
        } else if KdeWindowProvider::is_available() {
            "kde"
        } else if GnomeWindowProvider::is_available() {
            "gnome"
        } else {
            "none"
        }
    }

    fn active_window(&self) -> Option<ActiveWindow> {
        if HyprlandWindowProvider::is_available() {
            return HyprlandWindowProvider.active_window();
        }
        if KdeWindowProvider::is_available() {
            return KdeWindowProvider.active_window();
        }
        if GnomeWindowProvider::is_available() {
            return GnomeWindowProvider.active_window();
        }
        None
    }
}

/// Tracks active-window transitions and deduplicates repeated profile matches.
#[derive(Debug, Default)]
pub struct WindowMatchTracker {
    last_window: Option<ActiveWindow>,
    last_matched_profile: Option<String>,
}

impl WindowMatchTracker {
    /// Creates a new tracker with no prior window or profile state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates a newly observed window against `profiles` for `(vid, pid)`.
    ///
    /// Returns `Some(profile)` only when:
    /// 1. The active window changed since the previous poll, AND
    /// 2. A matching profile exists, AND
    /// 3. That profile differs from the most recently applied profile.
    pub fn evaluate<'a>(
        &mut self,
        window: &ActiveWindow,
        profiles: &'a [SoftwareProfile],
        vid: Option<u16>,
        pid: Option<u16>,
    ) -> Option<&'a SoftwareProfile> {
        if self.last_window.as_ref() == Some(window) {
            return None;
        }
        self.last_window = Some(window.clone());

        let matched = match_profile_for_window(profiles, &window.app_id, &window.title, vid, pid)?;
        if self.last_matched_profile.as_deref() == Some(matched.name.as_str()) {
            return None;
        }
        self.last_matched_profile = Some(matched.name.clone());
        Some(matched)
    }

    /// Resets cached window and profile state (for example after profile list edits).
    pub fn reset(&mut self) {
        self.last_window = None;
        self.last_matched_profile = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squeak_core::SOFTWARE_PROFILE_SCHEMA_V1;

    struct MockWindowProvider {
        window: Option<ActiveWindow>,
    }

    impl ActiveWindowProvider for MockWindowProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn active_window(&self) -> Option<ActiveWindow> {
            self.window.clone()
        }
    }

    #[test]
    fn parses_hyprland_kde_and_gnome_outputs() {
        let hypr = parse_hyprland_activewindow_json(
            r#"{"class":"steam_app_730","initialClass":"cs2","title":"Counter-Strike 2"}"#,
        )
        .expect("hypr json should parse");
        assert_eq!(hypr.app_id, "steam_app_730");
        assert_eq!(hypr.title, "Counter-Strike 2");

        let kde = parse_kwin_window_info("caption: Mozilla Firefox\nresourceClass: firefox\n")
            .expect("kwin info should parse");
        assert_eq!(kde.app_id, "firefox");
        assert_eq!(kde.title, "Mozilla Firefox");

        let gnome = parse_gnome_gdbus_json_output(
            r#"(true, '{"wm_class":"org.gnome.Nautilus","title":"Files"}')"#,
        )
        .expect("gnome output should parse");
        assert_eq!(gnome.app_id, "org.gnome.Nautilus");
        assert_eq!(gnome.title, "Files");
    }

    #[test]
    fn tracker_switches_and_deduplicates_profiles() {
        let profiles = vec![
            SoftwareProfile {
                schema_version: SOFTWARE_PROFILE_SCHEMA_V1,
                name: "fps".into(),
                vendor_id: Some(0x046d),
                product_id: Some(0xc08b),
                dpi: Some(800),
                report_rate_hz: Some(1000),
                onboard_mode: Some("host".into()),
                match_apps: vec!["steam_app_730".into()],
                led: vec![],
            },
            SoftwareProfile {
                schema_version: SOFTWARE_PROFILE_SCHEMA_V1,
                name: "browser".into(),
                vendor_id: None,
                product_id: None,
                dpi: Some(1200),
                report_rate_hz: Some(500),
                onboard_mode: None,
                match_apps: vec!["firefox".into()],
                led: vec![],
            },
        ];

        let mut tracker = WindowMatchTracker::new();
        let game_provider = MockWindowProvider {
            window: Some(ActiveWindow {
                app_id: "steam_app_730".into(),
                title: "CS2".into(),
            }),
        };
        let win1 = game_provider.active_window().expect("mock window");
        let first = tracker.evaluate(&win1, &profiles, Some(0x046d), Some(0xc08b));
        assert_eq!(first.map(|p| p.name.as_str()), Some("fps"));

        // Polling the exact same window again must not re-apply.
        let repeat = tracker.evaluate(&win1, &profiles, Some(0x046d), Some(0xc08b));
        assert!(repeat.is_none());

        // Switching to Firefox applies the browser profile.
        let win2 = ActiveWindow {
            app_id: "org.mozilla.firefox".into(),
            title: "Docs".into(),
        };
        let second = tracker.evaluate(&win2, &profiles, Some(0x046d), Some(0xc08b));
        assert_eq!(second.map(|p| p.name.as_str()), Some("browser"));
    }
}

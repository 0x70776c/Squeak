//! Internationalization (`i18n`) support for `squeak-gui` (English & German).
//!
//! Uses `LC_ALL` / `LC_MESSAGES` / `LANG` detection with a built-in translation table
//! so both uninstalled dev builds and distro-packaged builds render localized strings.

/// Supported UI locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    /// English (default fallback).
    En,
    /// German (`de` / `de_DE` / `de_AT` / `de_CH`).
    De,
}

impl Locale {
    /// Detects the active locale from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, or `LANG`.
    #[must_use]
    pub fn detect() -> Self {
        for var in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                let lower = val.trim().to_ascii_lowercase();
                if lower.starts_with("de") {
                    return Self::De;
                }
                if lower.starts_with("en") || lower == "c" || lower == "posix" {
                    return Self::En;
                }
            }
        }
        Self::En
    }
}

/// Translates `msgid` using the active system locale.
#[must_use]
pub fn tr(msgid: &'static str) -> &'static str {
    tr_for(Locale::detect(), msgid)
}

/// Translates `msgid` for an explicit [`Locale`].
#[must_use]
pub fn tr_for(locale: Locale, msgid: &'static str) -> &'static str {
    if locale == Locale::En {
        return msgid;
    }
    match msgid {
        "Squeak" => "Squeak",
        "Devices" => "Geräte",
        "Overview" => "Übersicht",
        "DPI & Sensor" => "DPI & Sensor",
        "Buttons" => "Tasten",
        "Lighting" => "Beleuchtung",
        "Profiles" => "Profile",
        "Scroll Wheel" => "Mausrad",
        "Refresh" => "Aktualisieren",
        "Undo" => "Rückgängig",
        "Save to profile" => "Im Profil speichern",
        "Apply" => "Anwenden",
        "Backup Now" => "Backup erstellen",
        "Install udev rule" => "udev-Regel installieren",
        "Retry" => "Erneut versuchen",
        "No permission to access HID++ device" => "Keine Berechtigung für HID++-Gerät",
        "Device disconnected or asleep" => "Gerät getrennt oder im Standby",
        "Daemon not running" => "Daemon nicht aktiv",
        "Unsupported feature" => "Nicht unterstützte Funktion",
        "Active Slot" => "Aktiver Slot",
        "Polling Rate" => "Abfragerate",
        "Battery" => "Akku",
        "Firmware" => "Firmware",
        "Operating Mode" => "Betriebsmodus",
        "Host (Software)" => "Host (Software)",
        "Onboard Memory" => "Gerätespeicher",
        "Color" => "Farbe",
        "Effect" => "Effekt",
        "Speed (ms)" => "Geschwindigkeit (ms)",
        "Brightness (%)" => "Helligkeit (%)",
        "Settings applied" => "Einstellungen übernommen",
        "Change undone" => "Änderung rückgängig gemacht",
        "Backup created" => "Backup erstellt",
        _ => msgid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_known_strings_to_german_and_falls_back_to_english() {
        assert_eq!(tr_for(Locale::De, "Overview"), "Übersicht");
        assert_eq!(
            tr_for(Locale::De, "Install udev rule"),
            "udev-Regel installieren"
        );
        assert_eq!(tr_for(Locale::En, "Overview"), "Overview");
        assert_eq!(tr_for(Locale::De, "Untranslated"), "Untranslated");
    }
}

//! Internationalization (`i18n`) support for `squeak-gui` (English & German).
//!
//! Defaults to English (`Locale::En`) on startup and supports live runtime switching
//! between English and German in the GUI.

use std::sync::atomic::{AtomicU8, Ordering};

/// Supported UI locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Locale {
    /// English.
    En = 0,
    /// German (`de` / `de_DE` / `de_AT` / `de_CH`).
    De = 1,
}

static ACTIVE_LOCALE: AtomicU8 = AtomicU8::new(0);

impl Locale {
    /// Returns the default initial locale (`Locale::En`).
    #[must_use]
    pub fn detect() -> Self {
        Self::En
    }
}

/// Returns the currently active UI [`Locale`].
#[must_use]
pub fn active_locale() -> Locale {
    match ACTIVE_LOCALE.load(Ordering::Acquire) {
        1 => Locale::De,
        _ => Locale::En,
    }
}

/// Sets the active UI [`Locale`] at runtime.
pub fn set_locale(locale: Locale) {
    ACTIVE_LOCALE.store(locale as u8, Ordering::Release);
}

/// Translates `msgid` using the active runtime locale.
#[must_use]
pub fn tr(msgid: &'static str) -> &'static str {
    tr_for(active_locale(), msgid)
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
        "DPI Presets" => "DPI-Stufen",
        "Click a preset to apply immediately, or add custom stages (max 6)." => {
            "Preset anklicken zum direkten Übernehmen oder eigene Stufen anlegen (max. 6)."
        }
        "Add Custom DPI" => "Eigene DPI-Stufe",
        "+ Add" => "+ Hinzufügen",
        "Buttons" => "Tasten",
        "Lighting" => "Beleuchtung",
        "Profiles" => "Profile",
        "Scroll Wheel" => "Mausrad",
        "Refresh" => "Aktualisieren",
        "Undo" => "Rückgängig",
        "Save to profile" => "Im Profil speichern",
        "Apply" => "Anwenden",
        "Apply Effect" => "Effekt anwenden",
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
        "Click a button on the mouse diagram" => "Taste im Mausdiagramm anklicken",
        "Highlight" => "Hervorheben",
        "Software Profiles (Auto-Switch)" => "Software-Profile (Auto-Wechsel)",
        "XDG TOML profiles in ~/.config/squeak/profiles/ with per-app window matching" => {
            "XDG-TOML-Profile in ~/.config/squeak/profiles/ mit App-Fenster-Erkennung"
        }
        "Profile Name" => "Profilname",
        "Match Apps" => "Apps zuordnen",
        "Comma-separated WM class or window title patterns" => {
            "Kommagetrennte WM-Class- oder Fenstertitel-Muster"
        }
        "Target DPI" => "Ziel-DPI",
        "Save Software Profile" => "Software-Profil speichern",
        "Save" => "Speichern",
        "Saved Software Profiles" => "Gespeicherte Software-Profile",
        "No software profiles saved yet" => "Noch keine Software-Profile gespeichert",
        "Delete" => "Löschen",
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

    #[test]
    fn runtime_locale_switches_between_en_and_de() {
        set_locale(Locale::En);
        assert_eq!(tr("Overview"), "Overview");
        set_locale(Locale::De);
        assert_eq!(tr("Overview"), "Übersicht");
    }
}

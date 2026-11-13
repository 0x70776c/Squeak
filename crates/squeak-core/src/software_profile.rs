//! XDG TOML software profiles (`$XDG_CONFIG_HOME/squeak/profiles/*.toml`),
//! import/export, validation, and active-window matching.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current schema version for Squeak software profile TOML files.
pub const SOFTWARE_PROFILE_SCHEMA_V1: u32 = 1;

/// Errors returned when loading, validating, importing, or exporting software profiles.
#[derive(Debug, Error)]
pub enum SoftwareProfileError {
    /// Profile name is empty or contains invalid path characters.
    #[error("invalid profile name '{name}': {reason}")]
    InvalidName {
        /// Profile name.
        name: String,
        /// Validation failure reason.
        reason: String,
    },
    /// Unsupported schema version in TOML file.
    #[error(
        "unsupported software profile schema version {0} (expected {SOFTWARE_PROFILE_SCHEMA_V1})"
    )]
    UnsupportedSchema(u32),
    /// Profile configuration contains invalid parameters.
    #[error("invalid profile configuration: {0}")]
    InvalidConfig(String),
    /// Filesystem I/O failure.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Target file or directory path.
        path: PathBuf,
        /// Underlying OS error.
        source: std::io::Error,
    },
    /// TOML parse error.
    #[error("failed to parse profile TOML: {0}")]
    TomlDecode(#[from] toml::de::Error),
    /// TOML serialization error.
    #[error("failed to serialize profile TOML: {0}")]
    TomlEncode(#[from] toml::ser::Error),
}

/// Desired LED zone configuration stored inside a [`SoftwareProfile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareLedZone {
    /// Zero-based LED zone index (`0` = primary/DPI LEDs, `1` = logo, etc.).
    pub zone_index: u8,
    /// Effect name (`"off"`, `"static"`, `"breathing"`, `"cycle"`).
    pub effect: String,
    /// Optional `#RRGGBB` hex color for `static` and `breathing`.
    pub color_hex: Option<String>,
    /// Optional period in milliseconds (`500..=20000`).
    pub period_ms: Option<u16>,
    /// Optional brightness percentage (`1..=100`).
    pub brightness_pct: Option<u8>,
}

/// Host-managed software profile stored as TOML under `$XDG_CONFIG_HOME/squeak/profiles/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareProfile {
    /// Schema version (`1`).
    pub schema_version: u32,
    /// Human-readable profile name (also determines the `.toml` filename).
    pub name: String,
    /// Optional target USB/BT vendor ID filter (e.g. `0x046d`).
    pub vendor_id: Option<u16>,
    /// Optional target USB/BT product ID filter (e.g. `0xc08b`).
    pub product_id: Option<u16>,
    /// Optional case-insensitive app/window-class/title patterns for auto-switching.
    #[serde(default)]
    pub match_apps: Vec<String>,
    /// Optional target DPI for sensor 0.
    pub dpi: Option<u16>,
    /// Optional polling report rate in Hz (e.g. `1000`).
    pub report_rate_hz: Option<u16>,
    /// Optional operating mode (`"host"` or `"onboard"`).
    pub onboard_mode: Option<String>,
    /// Optional per-zone LED effects.
    #[serde(default)]
    pub led: Vec<SoftwareLedZone>,
}

impl SoftwareProfile {
    /// Creates a minimal valid software profile with `schema_version = 1`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema_version: SOFTWARE_PROFILE_SCHEMA_V1,
            name: name.into(),
            vendor_id: None,
            product_id: None,
            match_apps: Vec::new(),
            dpi: None,
            report_rate_hz: None,
            onboard_mode: None,
            led: Vec::new(),
        }
    }

    /// Validates schema version, profile name, and parameter ranges.
    ///
    /// # Errors
    ///
    /// Returns [`SoftwareProfileError`] if any field is invalid.
    pub fn validate(&self) -> Result<(), SoftwareProfileError> {
        if self.schema_version != SOFTWARE_PROFILE_SCHEMA_V1 {
            return Err(SoftwareProfileError::UnsupportedSchema(self.schema_version));
        }
        sanitize_profile_slug(&self.name)?;
        if let Some(dpi) = self.dpi
            && dpi == 0
        {
            return Err(SoftwareProfileError::InvalidConfig(
                "dpi must be greater than 0".to_owned(),
            ));
        }
        if let Some(hz) = self.report_rate_hz
            && hz == 0
        {
            return Err(SoftwareProfileError::InvalidConfig(
                "report_rate_hz must be greater than 0".to_owned(),
            ));
        }
        if let Some(mode) = &self.onboard_mode {
            let lower = mode.trim().to_ascii_lowercase();
            if lower != "host" && lower != "onboard" {
                return Err(SoftwareProfileError::InvalidConfig(format!(
                    "invalid onboard_mode '{mode}' (expected 'host' or 'onboard')"
                )));
            }
        }
        for zone in &self.led {
            crate::ops::parse_led_effect_spec(
                &zone.effect,
                zone.color_hex.as_deref(),
                zone.period_ms,
                zone.brightness_pct,
            )
            .map_err(|e| SoftwareProfileError::InvalidConfig(e.to_string()))?;
        }
        Ok(())
    }

    /// Parses and validates a [`SoftwareProfile`] from TOML text.
    ///
    /// # Errors
    ///
    /// Returns [`SoftwareProfileError`] if TOML parsing or validation fails.
    pub fn from_toml_str(raw: &str) -> Result<Self, SoftwareProfileError> {
        let profile: Self = toml::from_str(raw)?;
        profile.validate()?;
        Ok(profile)
    }

    /// Serializes a validated [`SoftwareProfile`] to pretty TOML text.
    ///
    /// # Errors
    ///
    /// Returns [`SoftwareProfileError`] if validation or TOML serialization fails.
    pub fn to_toml_string(&self) -> Result<String, SoftwareProfileError> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }

    /// Returns `true` if any `match_apps` pattern matches `app_id` or `title` (case-insensitive).
    #[must_use]
    pub fn matches_window(&self, app_id: &str, title: &str) -> bool {
        if self.match_apps.is_empty() {
            return false;
        }
        let app_lower = app_id.trim().to_ascii_lowercase();
        let title_lower = title.trim().to_ascii_lowercase();
        self.match_apps.iter().any(|pat| {
            let clean = pat.trim().to_ascii_lowercase();
            !clean.is_empty() && (app_lower.contains(&clean) || title_lower.contains(&clean))
        })
    }

    /// Serializes this profile to TOML after validation.
    ///
    /// # Errors
    ///
    /// Returns [`SoftwareProfileError`] if validation or serialization fails.
    pub fn to_toml(&self) -> Result<String, SoftwareProfileError> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }

    /// Parses and validates a [`SoftwareProfile`] from TOML.
    ///
    /// # Errors
    ///
    /// Returns [`SoftwareProfileError`] if parsing or validation fails.
    pub fn from_toml(raw: &str) -> Result<Self, SoftwareProfileError> {
        let parsed: Self = toml::from_str(raw)?;
        parsed.validate()?;
        Ok(parsed)
    }
}

/// Returns the default XDG software profiles directory (`$XDG_CONFIG_HOME/squeak/profiles`).
#[must_use]
pub fn xdg_profiles_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    base.join("squeak").join("profiles")
}

/// Converts a profile name into a safe single-segment filename slug.
///
/// # Errors
///
/// Returns [`SoftwareProfileError::InvalidName`] if the name is empty or contains path separators.
pub fn sanitize_profile_slug(name: &str) -> Result<String, SoftwareProfileError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SoftwareProfileError::InvalidName {
            name: name.to_owned(),
            reason: "name must not be empty".to_owned(),
        });
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(SoftwareProfileError::InvalidName {
            name: name.to_owned(),
            reason: "name must not contain path separators or '..'".to_owned(),
        });
    }
    let slug: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let clean = slug.trim_matches('-').to_owned();
    if clean.is_empty() {
        return Err(SoftwareProfileError::InvalidName {
            name: name.to_owned(),
            reason: "name contains no alphanumeric characters".to_owned(),
        });
    }
    Ok(clean)
}

/// Loads all valid `.toml` software profiles from `dir`, sorted by name.
///
/// Returns an empty list if `dir` does not exist yet.
///
/// # Errors
///
/// Returns [`SoftwareProfileError`] if reading `dir` or parsing a `.toml` file fails.
pub fn load_profiles_from_dir(dir: &Path) -> Result<Vec<SoftwareProfile>, SoftwareProfileError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir).map_err(|source| SoftwareProfileError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut profiles = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| SoftwareProfileError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|source| SoftwareProfileError::Io {
            path: path.clone(),
            source,
        })?;
        profiles.push(SoftwareProfile::from_toml(&raw)?);
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

/// Saves `profile` into `dir/<slug>.toml`, creating `dir` if needed.
///
/// # Errors
///
/// Returns [`SoftwareProfileError`] on validation or I/O failure.
pub fn save_profile_to_dir(
    dir: &Path,
    profile: &SoftwareProfile,
) -> Result<PathBuf, SoftwareProfileError> {
    let slug = sanitize_profile_slug(&profile.name)?;
    let toml_text = profile.to_toml()?;
    fs::create_dir_all(dir).map_err(|source| SoftwareProfileError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(format!("{slug}.toml"));
    fs::write(&path, toml_text).map_err(|source| SoftwareProfileError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Deletes the profile matching `name` from `dir`.
///
/// # Errors
///
/// Returns [`SoftwareProfileError`] if the name is invalid or deletion fails.
pub fn delete_profile_from_dir(dir: &Path, name: &str) -> Result<PathBuf, SoftwareProfileError> {
    let slug = sanitize_profile_slug(name)?;
    let path = dir.join(format!("{slug}.toml"));
    fs::remove_file(&path).map_err(|source| SoftwareProfileError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Exports a profile by name from `dir` to `export_path`.
///
/// # Errors
///
/// Returns [`SoftwareProfileError`] if the profile is missing or writing fails.
pub fn export_profile(
    dir: &Path,
    name: &str,
    export_path: &Path,
) -> Result<SoftwareProfile, SoftwareProfileError> {
    let slug = sanitize_profile_slug(name)?;
    let src = dir.join(format!("{slug}.toml"));
    let raw = fs::read_to_string(&src)
        .map_err(|source| SoftwareProfileError::Io { path: src, source })?;
    let profile = SoftwareProfile::from_toml(&raw)?;
    if let Some(parent) = export_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| SoftwareProfileError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(export_path, profile.to_toml()?).map_err(|source| SoftwareProfileError::Io {
        path: export_path.to_path_buf(),
        source,
    })?;
    Ok(profile)
}

/// Imports and validates a `.toml` profile from `import_path` into `dir`.
///
/// # Errors
///
/// Returns [`SoftwareProfileError`] if reading, validation, or saving fails.
pub fn import_profile(
    import_path: &Path,
    dir: &Path,
) -> Result<(SoftwareProfile, PathBuf), SoftwareProfileError> {
    let raw = fs::read_to_string(import_path).map_err(|source| SoftwareProfileError::Io {
        path: import_path.to_path_buf(),
        source,
    })?;
    let profile = SoftwareProfile::from_toml(&raw)?;
    let dest = save_profile_to_dir(dir, &profile)?;
    Ok((profile, dest))
}

/// Finds the first software profile matching `(app_id, title)` and optional device `(vid, pid)`.
#[must_use]
pub fn match_profile_for_window<'a>(
    profiles: &'a [SoftwareProfile],
    app_id: &str,
    title: &str,
    vid: Option<u16>,
    pid: Option<u16>,
) -> Option<&'a SoftwareProfile> {
    profiles.iter().find(|p| {
        let dev_ok = match (p.vendor_id, p.product_id, vid, pid) {
            (Some(pv), Some(pp), Some(dv), Some(dp)) => pv == dv && pp == dp,
            (Some(pv), None, Some(dv), _) => pv == dv,
            (None, None, _, _) => true,
            _ => false,
        };
        dev_ok && p.matches_window(app_id, title)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "squeak-swprofile-{tag}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn roundtrips_saves_loads_exports_and_imports_profile() {
        let dir = TempDir::new("roundtrip");
        let export_dir = TempDir::new("export");

        let mut p = SoftwareProfile::new("Counter Strike 2");
        p.dpi = Some(800);
        p.report_rate_hz = Some(1000);
        p.onboard_mode = Some("host".to_owned());
        p.match_apps = vec!["cs2".to_owned(), "steam_app_730".to_owned()];
        p.led.push(SoftwareLedZone {
            zone_index: 1,
            effect: "static".to_owned(),
            color_hex: Some("#FF5500".to_owned()),
            period_ms: None,
            brightness_pct: None,
        });

        let saved = save_profile_to_dir(&dir.0, &p).unwrap();
        assert!(saved.ends_with("counter-strike-2.toml"));

        let loaded = load_profiles_from_dir(&dir.0).unwrap();
        assert_eq!(loaded, vec![p.clone()]);

        let export_file = export_dir.0.join("cs2-backup.toml");
        export_profile(&dir.0, "Counter Strike 2", &export_file).unwrap();

        delete_profile_from_dir(&dir.0, "Counter Strike 2").unwrap();
        assert!(load_profiles_from_dir(&dir.0).unwrap().is_empty());

        let (imported, _) = import_profile(&export_file, &dir.0).unwrap();
        assert_eq!(imported, p);
    }

    #[test]
    fn rejects_invalid_names_and_matches_active_window() {
        assert!(sanitize_profile_slug("../evil").is_err());
        assert!(sanitize_profile_slug("").is_err());

        let mut p = SoftwareProfile::new("Gaming");
        p.match_apps = vec!["steam_app_730".to_owned()];
        p.vendor_id = Some(0x046D);
        p.product_id = Some(0xC08B);

        let profiles = [p];
        assert!(
            match_profile_for_window(
                &profiles,
                "steam_app_730",
                "Counter-Strike 2",
                Some(0x046D),
                Some(0xC08B)
            )
            .is_some()
        );
        assert!(
            match_profile_for_window(
                &profiles,
                "firefox",
                "Mozilla Firefox",
                Some(0x046D),
                Some(0xC08B)
            )
            .is_none()
        );
    }
}

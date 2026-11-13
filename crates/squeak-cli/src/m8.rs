//! Milestone M8 CLI handlers for `squeak sw-profiles`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use hidpp::{DeviceIndex, FeatureTable, HidTransport, HidppSession};
use serde::Serialize;
use squeak_core::{
    SOFTWARE_PROFILE_SCHEMA_V1, SoftwareLedZone, SoftwareProfile, delete_profile_from_dir,
    execute_dpi_write, execute_host_mode_write, execute_led_write, execute_report_rate_write,
    export_profile, import_profile, load_profiles_from_dir, parse_led_effect_spec,
    sanitize_profile_slug, save_profile_to_dir,
};

use crate::cli::SwProfilesSaveArgs;
use crate::commands::CliError;

/// Summary returned for `sw-profiles save`, `export`, `import`, and `delete`.
#[derive(Debug, Clone, Serialize)]
pub struct SwProfileFileReport {
    /// Action performed (`"saved"`, `"exported"`, `"imported"`, `"deleted"`).
    pub action: String,
    /// Profile name.
    pub name: String,
    /// Target or affected file path.
    pub path: PathBuf,
}

/// Summary returned for `sw-profiles apply`.
#[derive(Debug, Clone, Serialize)]
pub struct SwProfileApplyReport {
    /// Software profile name.
    pub profile_name: String,
    /// Whether this run was a dry-run preview without hardware writes.
    pub dry_run: bool,
    /// Operating mode applied (`"host"` or `"onboard"`), if configured.
    pub onboard_mode: Option<String>,
    /// DPI applied to sensor 0, if configured.
    pub dpi: Option<u16>,
    /// Polling report rate applied in Hz, if configured.
    pub report_rate_hz: Option<u16>,
    /// LED zone indices applied, if configured.
    pub led_zones: Vec<u8>,
}

/// Lists all XDG TOML software profiles in `dir`.
pub fn sw_profiles_list(dir: &Path) -> Result<Vec<SoftwareProfile>, CliError> {
    load_profiles_from_dir(dir).map_err(|e| CliError::Usage(e.to_string()))
}

/// Validates and saves a software profile from CLI flags into `dir`.
pub fn sw_profiles_save(
    dir: &Path,
    args: &SwProfilesSaveArgs,
) -> Result<SwProfileFileReport, CliError> {
    let vendor_id = args.vendor_id.as_deref().map(parse_hex_u16).transpose()?;
    let product_id = args.product_id.as_deref().map(parse_hex_u16).transpose()?;

    let mut led = Vec::new();
    if let Some(ref effect) = args.led_effect {
        parse_led_effect_spec(
            effect,
            args.led_color.as_deref(),
            args.led_period,
            args.led_brightness,
        )
        .map_err(|e| CliError::Usage(e.to_string()))?;
        led.push(SoftwareLedZone {
            zone_index: args.led_zone,
            effect: effect.clone(),
            color_hex: args.led_color.clone(),
            period_ms: args.led_period,
            brightness_pct: args.led_brightness,
        });
    }

    let profile = SoftwareProfile {
        schema_version: SOFTWARE_PROFILE_SCHEMA_V1,
        name: args.name.clone(),
        vendor_id,
        product_id,
        match_apps: args.match_apps.clone(),
        dpi: args.dpi,
        report_rate_hz: args.report_rate,
        onboard_mode: args.onboard_mode.clone(),
        led,
    };

    let path = save_profile_to_dir(dir, &profile).map_err(|e| CliError::Usage(e.to_string()))?;
    Ok(SwProfileFileReport {
        action: "saved".to_owned(),
        name: profile.name,
        path,
    })
}

/// Exports a named profile from `dir` to `out`.
pub fn sw_profiles_export(
    dir: &Path,
    name: &str,
    out: &Path,
) -> Result<SwProfileFileReport, CliError> {
    let profile = export_profile(dir, name, out).map_err(|e| CliError::Usage(e.to_string()))?;
    Ok(SwProfileFileReport {
        action: "exported".to_owned(),
        name: profile.name,
        path: out.to_path_buf(),
    })
}

/// Imports a TOML software profile from `file` into `dir`.
pub fn sw_profiles_import(dir: &Path, file: &Path) -> Result<SwProfileFileReport, CliError> {
    let (profile, path) = import_profile(file, dir).map_err(|e| CliError::Usage(e.to_string()))?;
    Ok(SwProfileFileReport {
        action: "imported".to_owned(),
        name: profile.name,
        path,
    })
}

/// Deletes a named software profile from `dir`.
pub fn sw_profiles_delete(dir: &Path, name: &str) -> Result<SwProfileFileReport, CliError> {
    let path = delete_profile_from_dir(dir, name).map_err(|e| CliError::Usage(e.to_string()))?;
    Ok(SwProfileFileReport {
        action: "deleted".to_owned(),
        name: name.to_owned(),
        path,
    })
}

/// Applies (or dry-runs) a named software profile from `dir` onto `transport`.
pub fn sw_profiles_apply_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    dir: &Path,
    name: &str,
    apply: bool,
) -> Result<SwProfileApplyReport, CliError> {
    let slug = sanitize_profile_slug(name).map_err(|e| CliError::Usage(e.to_string()))?;
    let profiles = load_profiles_from_dir(dir).map_err(|e| CliError::Usage(e.to_string()))?;
    let profile = profiles
        .into_iter()
        .find(|p| sanitize_profile_slug(&p.name).ok().as_deref() == Some(slug.as_str()))
        .ok_or_else(|| CliError::Usage(format!("software profile '{name}' not found")))?;

    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;

    if let Some(ref mode) = profile.onboard_mode {
        let host_mode = match mode.trim().to_ascii_lowercase().as_str() {
            "host" | "software" => true,
            "onboard" | "memory" | "hardware" => false,
            other => {
                return Err(CliError::Usage(format!(
                    "invalid onboard_mode '{other}' in profile '{}'",
                    profile.name
                )));
            }
        };
        execute_host_mode_write(&mut session, &table, host_mode, apply)?;
    }

    if let Some(dpi) = profile.dpi {
        execute_dpi_write(&mut session, &table, 0, dpi, apply)?;
    }

    if let Some(hz) = profile.report_rate_hz {
        execute_report_rate_write(&mut session, &table, hz, apply)?;
    }

    let mut led_zones = Vec::with_capacity(profile.led.len());
    for zone in &profile.led {
        let parsed = parse_led_effect_spec(
            &zone.effect,
            zone.color_hex.as_deref(),
            zone.period_ms,
            zone.brightness_pct,
        )
        .map_err(|e| CliError::Usage(e.to_string()))?;
        execute_led_write(&mut session, &table, zone.zone_index, parsed, apply)?;
        led_zones.push(zone.zone_index);
    }

    Ok(SwProfileApplyReport {
        profile_name: profile.name,
        dry_run: !apply,
        onboard_mode: profile.onboard_mode,
        dpi: profile.dpi,
        report_rate_hz: profile.report_rate_hz,
        led_zones,
    })
}

fn parse_hex_u16(raw: &str) -> Result<u16, CliError> {
    let trimmed = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(trimmed, 16)
        .map_err(|_| CliError::Usage(format!("invalid hex ID '{raw}' (expected e.g. 046d)")))
}

/// Formats `squeak sw-profiles list` output for humans.
#[must_use]
pub fn format_sw_profiles_list_human(profiles: &Vec<SoftwareProfile>) -> String {
    if profiles.is_empty() {
        return "No software profiles found.".to_owned();
    }
    let mut out = String::new();
    for p in profiles {
        let _ = writeln!(
            out,
            "{}: dpi={} rate={}Hz mode={} apps=[{}] leds={}",
            p.name,
            p.dpi.map_or_else(|| "-".to_owned(), |v| v.to_string()),
            p.report_rate_hz
                .map_or_else(|| "-".to_owned(), |v| v.to_string()),
            p.onboard_mode.as_deref().unwrap_or("-"),
            p.match_apps.join(", "),
            p.led.len()
        );
    }
    out.trim_end().to_owned()
}

/// Formats `squeak sw-profiles save/export/import/delete` output for humans.
#[must_use]
pub fn format_sw_profile_file_human(report: &SwProfileFileReport) -> String {
    format!(
        "Software profile '{}' {}: {}",
        report.name,
        report.action,
        report.path.display()
    )
}

/// Formats `squeak sw-profiles apply` output for humans.
#[must_use]
pub fn format_sw_profile_apply_human(report: &SwProfileApplyReport) -> String {
    let prefix = if report.dry_run {
        "[dry-run] Would apply"
    } else {
        "Applied"
    };
    format!(
        "{prefix} software profile '{}' (mode={}, dpi={}, rate={}Hz, led_zones={:?})",
        report.profile_name,
        report.onboard_mode.as_deref().unwrap_or("-"),
        report.dpi.map_or_else(|| "-".to_owned(), |v| v.to_string()),
        report
            .report_rate_hz
            .map_or_else(|| "-".to_owned(), |v| v.to_string()),
        report.led_zones
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("squeak_cli_m8_{label}_{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn sw_profiles_save_list_export_import_delete_roundtrip() {
        let dir = temp_dir("roundtrip");
        let export_dir = temp_dir("export_target");
        let export_path = export_dir.join("exported.toml");
        let import_dir = temp_dir("import_target");

        let save_args = SwProfilesSaveArgs {
            name: "Gaming FPS".to_owned(),
            dpi: Some(800),
            report_rate: Some(1000),
            onboard_mode: Some("host".to_owned()),
            vendor_id: Some("046d".to_owned()),
            product_id: Some("c08b".to_owned()),
            match_apps: vec!["steam_app_730".to_owned()],
            led_zone: 1,
            led_effect: Some("static".to_owned()),
            led_color: Some("#FF3300".to_owned()),
            led_brightness: Some(90),
            led_period: None,
        };

        let saved = sw_profiles_save(&dir, &save_args).expect("save");
        assert_eq!(saved.action, "saved");
        assert!(saved.path.exists());

        let listed = sw_profiles_list(&dir).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].dpi, Some(800));
        assert_eq!(listed[0].vendor_id, Some(0x046d));

        let exported = sw_profiles_export(&dir, "Gaming FPS", &export_path).expect("export");
        assert_eq!(exported.action, "exported");
        assert!(export_path.exists());

        let imported = sw_profiles_import(&import_dir, &export_path).expect("import");
        assert_eq!(imported.action, "imported");
        assert_eq!(sw_profiles_list(&import_dir).expect("list import").len(), 1);

        let deleted = sw_profiles_delete(&dir, "Gaming FPS").expect("delete");
        assert_eq!(deleted.action, "deleted");
        assert!(
            sw_profiles_list(&dir)
                .expect("list after delete")
                .is_empty()
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&export_dir);
        let _ = fs::remove_dir_all(&import_dir);
    }
}

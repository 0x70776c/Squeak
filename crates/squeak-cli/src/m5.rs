//! M5 lighting (`led`), host-mode (`host-mode`), and button inspection (`buttons`) CLI commands.

use std::fmt::Write as _;

use hidpp::{
    DeviceIndex, FeatureId, FeatureTable, HidTransport, HidppSession, LedEffect,
    MouseButtonSpyDriver, ProtocolError, RgbColor, TransportError,
};
use squeak_core::{
    DeviceDatabase, LedConfigReport, LedWriteReport, OnboardModeReport, OnboardModeWriteReport,
    ResolvedDevice, execute_host_mode_write, execute_led_write, read_led_config, read_onboard_mode,
};

use crate::cli::{HostModeSetArgs, LedSetArgs};
use crate::commands::CliError;

/// Reads active and supported LED lighting state from `COLOR_LED_EFFECTS` (`0x8070`).
///
/// # Errors
///
/// Returns [`CliError`] if discovery fails or `0x8070` is not present.
pub fn led_get_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
) -> Result<LedConfigReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    Ok(
        read_led_config(&mut session, &table)?.ok_or(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::COLOR_LED_EFFECTS,
        })?,
    )
}

/// Sets an LED lighting effect via `0x8070` (preview by default; applied when `apply = true`).
///
/// # Errors
///
/// Returns [`CliError`] if arguments are invalid or device communication fails.
pub fn led_set_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    args: &LedSetArgs,
) -> Result<LedWriteReport, CliError> {
    let effect = match args.effect.to_ascii_lowercase().as_str() {
        "off" => LedEffect::Off,
        "static" => {
            let color_str = args.color.as_deref().ok_or_else(|| {
                CliError::Usage(
                    "static effect requires --color <HEX> (e.g. --color '#FF0000')".into(),
                )
            })?;
            let color = parse_hex_color(color_str)?;
            LedEffect::Static { color }
        }
        "breathing" => {
            let color_str = args.color.as_deref().ok_or_else(|| {
                CliError::Usage(
                    "breathing effect requires --color <HEX> (e.g. --color '#0000FF')".into(),
                )
            })?;
            let color = parse_hex_color(color_str)?;
            let period_ms = args.period.unwrap_or(2000);
            let brightness_pct = args.brightness.unwrap_or(100);
            LedEffect::Breathing {
                color,
                period_ms,
                brightness_pct,
            }
        }
        "cycle" | "color-cycle" | "color_cycle" => {
            let period_ms = args.period.unwrap_or(4000);
            let brightness_pct = args.brightness.unwrap_or(100);
            LedEffect::ColorCycle {
                period_ms,
                brightness_pct,
            }
        }
        other => {
            return Err(CliError::Usage(format!(
                "unsupported LED effect '{other}' (expected 'off', 'static', 'breathing', or 'cycle')"
            )));
        }
    };

    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    Ok(execute_led_write(
        &mut session,
        &table,
        args.zone,
        effect,
        args.apply,
    )?)
}

/// Reads operating mode (`host` or `onboard`) and onboard memory description from `0x8100`.
///
/// # Errors
///
/// Returns [`CliError`] if discovery fails or `0x8100` is not present.
pub fn host_mode_get_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
) -> Result<OnboardModeReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    Ok(
        read_onboard_mode(&mut session, &table)?.ok_or(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        })?,
    )
}

/// Sets operating mode (`host` or `onboard`) via `0x8100`.
///
/// # Errors
///
/// Returns [`CliError`] if arguments are invalid or device communication fails.
pub fn host_mode_set_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    args: &HostModeSetArgs,
) -> Result<OnboardModeWriteReport, CliError> {
    let target_host_mode = match args.mode.to_ascii_lowercase().as_str() {
        "host" | "software" => true,
        "onboard" | "memory" | "profile" => false,
        other => {
            return Err(CliError::Usage(format!(
                "invalid mode '{other}' (expected 'host' or 'onboard')"
            )));
        }
    };

    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    Ok(execute_host_mode_write(
        &mut session,
        &table,
        target_host_mode,
        args.apply,
    )?)
}

/// Parses a 24-bit hex color string like `"#FF0000"` or `"FF0000"`.
fn parse_hex_color(s: &str) -> Result<RgbColor, CliError> {
    let clean = s.strip_prefix('#').unwrap_or(s);
    if clean.len() != 6 {
        return Err(CliError::Usage(format!(
            "invalid hex color '{s}' (expected 6 hex characters, e.g. '#FF0000')"
        )));
    }
    let r = u8::from_str_radix(&clean[0..2], 16)
        .map_err(|_| CliError::Usage(format!("invalid red component in hex color '{s}'")))?;
    let g = u8::from_str_radix(&clean[2..4], 16)
        .map_err(|_| CliError::Usage(format!("invalid green component in hex color '{s}'")))?;
    let b = u8::from_str_radix(&clean[4..6], 16)
        .map_err(|_| CliError::Usage(format!("invalid blue component in hex color '{s}'")))?;
    Ok(RgbColor::new(r, g, b))
}

/// Formats a [`LedConfigReport`] into human-readable text.
#[must_use]
pub fn format_led_get_human(report: &LedConfigReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "LED Lighting: COLOR_LED_EFFECTS (0x8070 v{}, write supported: {})",
        report.feature_version, report.write_supported
    );
    for z in &report.zones {
        let _ = writeln!(
            out,
            "  Zone {} [{}]: {} supported effects ({})",
            z.zone_index,
            z.location_name,
            z.effect_count,
            z.supported_effects.join(", ")
        );
    }
    out
}

/// Formats a [`LedWriteReport`] into human-readable text.
#[must_use]
pub fn format_led_set_human(report: &LedWriteReport) -> String {
    let mut out = String::new();
    let status_tag = if report.applied {
        "[APPLIED]"
    } else {
        "[DRY-RUN]"
    };
    let _ = writeln!(
        out,
        "{status_tag} Zone {}: effect '{}'",
        report.zone_index, report.effect
    );
    if let Some(col) = &report.color {
        let _ = writeln!(out, "  Color: {col}");
    }
    if let Some(p) = report.period_ms {
        let _ = writeln!(out, "  Period: {p} ms");
    }
    if let Some(b) = report.brightness_pct {
        let _ = writeln!(out, "  Brightness: {b}%");
    }
    let _ = writeln!(
        out,
        "  Planned wire frame: feat {} fn {} params [{}]",
        report.planned_frame.feature_id,
        report.planned_frame.function_id,
        report.planned_frame.params_hex
    );
    if let Some(note) = &report.note {
        let _ = writeln!(out, "  Notice: {note}");
    }
    out
}

/// Formats a [`OnboardModeReport`] into human-readable text.
#[must_use]
pub fn format_host_mode_get_human(report: &OnboardModeReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Operating Mode: ONBOARD_PROFILES (0x8100 v{})",
        report.feature_version
    );
    let _ = writeln!(out, "  Active mode:     {}", report.mode.to_uppercase());
    if report.mode == "host" {
        let _ = writeln!(
            out,
            "  Profile slot:    None (Host Software Mode overrides onboard flash)"
        );
    } else {
        let _ = writeln!(
            out,
            "  Profile slot:    Slot {} / {}",
            report.current_profile, report.profile_count
        );
    }
    let _ = writeln!(
        out,
        "  Geometry:        {} profile slots, {} buttons, {} memory sectors",
        report.profile_count, report.button_count, report.sector_count
    );
    out
}

/// Formats a [`OnboardModeWriteReport`] into human-readable text.
#[must_use]
pub fn format_host_mode_set_human(report: &OnboardModeWriteReport) -> String {
    let mut out = String::new();
    let status_tag = if report.applied {
        "[APPLIED]"
    } else {
        "[DRY-RUN]"
    };
    let _ = writeln!(
        out,
        "{status_tag} Mode: {} -> {}",
        report.previous_mode, report.requested_mode
    );
    if let Some(rb) = &report.readback_mode {
        let _ = writeln!(
            out,
            "  Verified readback: {rb} (matches: {})",
            report.verified
        );
    }
    let _ = writeln!(
        out,
        "  Planned wire frame: feat {} fn {} params [{}]",
        report.planned_frame.feature_id,
        report.planned_frame.function_id,
        report.planned_frame.params_hex
    );
    if let Some(note) = &report.note {
        let _ = writeln!(out, "  Notice: {note}");
    }
    out
}

/// Button information summary for `squeak buttons list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ButtonsListReport {
    /// Number of physical buttons on the device.
    pub button_count: u8,
    /// Whether `MOUSE_BUTTON_SPY` (`0x8110`) is supported.
    pub spy_supported: bool,
    /// Whether `REPROG_CONTROLS_V4` (`0x1B04`) is supported.
    pub reprog_v4_supported: bool,
    /// Configured button names or labels from TOML database if available.
    pub buttons: Vec<String>,
}

/// Lists physical buttons and remapping capabilities for a device.
///
/// # Errors
///
/// Returns [`CliError`] if discovery fails.
pub fn buttons_list_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    db: &DeviceDatabase,
) -> Result<ButtonsListReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    let dev_info = session.device_info().clone();
    let resolved = db.resolve(dev_info.vendor_id, dev_info.product_id);

    let mut button_count = 0u8;
    let mut buttons = Vec::new();

    if let ResolvedDevice::Known(def) = resolved {
        button_count = def.buttons.len() as u8;
        for b in &def.buttons {
            buttons.push(format!("CID 0x{:04X} ({})", b.cid, b.label));
        }
    }

    let spy_supported = if let Some(spy_entry) = table.get(FeatureId::MOUSE_BUTTON_SPY) {
        if let Ok(spy) = MouseButtonSpyDriver::from_entry(spy_entry)
            && let Ok(cnt) = spy.get_button_count(&mut session, device_index)
            && button_count == 0
        {
            button_count = cnt;
            for i in 0..cnt {
                buttons.push(format!("Button {i}"));
            }
        }
        true
    } else {
        false
    };

    let reprog_v4_supported = table.get(FeatureId::REPROG_CONTROLS_V4).is_some();

    Ok(ButtonsListReport {
        button_count,
        spy_supported,
        reprog_v4_supported,
        buttons,
    })
}

/// Formats a [`ButtonsListReport`] into human-readable text.
#[must_use]
pub fn format_buttons_list_human(report: &ButtonsListReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Buttons: {} physical controls (Spy 0x8110: {}, ReprogV4 0x1B04: {})",
        report.button_count, report.spy_supported, report.reprog_v4_supported
    );
    for b in &report.buttons {
        let _ = writeln!(out, "  - {b}");
    }
    out
}

/// Runs a button spy session, logging events to stdout.
///
/// # Errors
///
/// Returns [`CliError`] if device does not support `0x8110` or communication fails.
pub fn run_buttons_spy<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    count_limit: Option<usize>,
    as_json: bool,
) -> Result<String, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    let entry =
        table
            .get(FeatureId::MOUSE_BUTTON_SPY)
            .ok_or(ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::MOUSE_BUTTON_SPY,
            })?;
    let driver = MouseButtonSpyDriver::from_entry(entry)?;
    let btn_count = driver.get_button_count(&mut session, device_index)?;
    driver.start_spy(&mut session, device_index)?;

    let max_events = count_limit.unwrap_or(20);

    if !as_json {
        eprintln!(
            "Monitoring {} buttons on device (SwId 0x00 notifications; collecting up to {} events)...",
            btn_count, max_events
        );
    }

    let mut event_count = 0usize;
    let mut last_mask = 0u16;

    while event_count < max_events {
        match session
            .transport_mut()
            .read_frame(std::time::Duration::from_millis(500))
        {
            Ok(frame) => {
                if let Ok(Some(ev)) = driver.parse_notification(&frame) {
                    let mask = ev.mask.as_u16();
                    if mask != last_mask {
                        last_mask = mask;
                        event_count += 1;
                        let pressed = ev.mask.pressed_buttons();
                        if as_json {
                            let json = serde_json::json!({
                                "event_index": event_count,
                                "raw_mask": format!("0x{mask:04X}"),
                                "pressed_buttons": pressed,
                            });
                            println!("{json}");
                        } else if pressed.is_empty() {
                            println!("[{event_count}] Released: all buttons up");
                        } else {
                            println!(
                                "[{event_count}] Pressed: buttons {pressed:?} (mask: 0x{mask:04X})"
                            );
                        }
                    }
                }
            }
            Err(TransportError::Timeout) => {
                continue;
            }
            Err(_err) => {
                break;
            }
        }
    }

    let _ = driver.stop_spy(&mut session, device_index);

    if !as_json {
        Ok(format!(
            "Button spy session finished ({event_count} events recorded)."
        ))
    } else {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use hidpp::{
        FeatureIndex, FunctionId, HidDeviceInfo, HidppFrame, MockTransport, ReportId, SwId,
    };

    use super::*;

    fn fn_id(v: u8) -> FunctionId {
        FunctionId::new(v).unwrap()
    }

    fn push_discovery_m5(mock: &mut MockTransport, features: &[(u16, u8)]) {
        // IRoot ping -> fn 1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x02, 0x53],
            )
            .unwrap(),
        );
        // IRoot getFeature(0x0001) -> fn 0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
        // IFeatureSet getCount -> N
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[u8::try_from(features.len()).unwrap(), 0, 0],
            )
            .unwrap(),
        );
        for &(id, ver) in features {
            let [hi, lo] = id.to_be_bytes();
            mock.push_frame(
                HidppFrame::new(
                    ReportId::Long,
                    DeviceIndex::DIRECT,
                    FeatureIndex(1),
                    fn_id(1),
                    SwId::DEFAULT_REQUEST,
                    &[hi, lo, 0, ver],
                )
                .unwrap(),
            );
        }
    }

    const FEATURES: &[(u16, u8)] = &[(0x0001, 0), (0x8070, 3), (0x8100, 0)];

    #[test]
    fn parses_hex_colors_properly() {
        let red = parse_hex_color("#FF0000").unwrap();
        assert_eq!(red, RgbColor::new(0xFF, 0, 0));

        let green = parse_hex_color("00FF00").unwrap();
        assert_eq!(green, RgbColor::new(0, 0xFF, 0));

        assert!(parse_hex_color("#FFF").is_err());
        assert!(parse_hex_color("ZZZZZZ").is_err());
    }

    #[test]
    fn led_get_and_set_dry_run_and_apply() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");

        // 1. led get
        let mut get_mock = MockTransport::new(info.clone());
        push_discovery_m5(&mut get_mock, FEATURES);
        // fn 0 getInfo -> 1 zone
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x01],
            )
            .unwrap(),
        );
        // fn 1 getZoneInfo(0) -> 0x00, loc 0x0001, 1 effect
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x01, 0x01],
            )
            .unwrap(),
        );
        // fn 2 getEffectInfo(0, 0) -> 0x0001 (Static)
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x00, 0x01],
            )
            .unwrap(),
        );

        let report = led_get_from_transport(get_mock, DeviceIndex::DIRECT).unwrap();
        assert_eq!(report.feature_version, 3);
        assert_eq!(report.zones.len(), 1);
        assert_eq!(report.zones[0].location_name, "Logo");
        assert_eq!(report.zones[0].supported_effects, vec!["static"]);

        // 2. led set --dry-run
        let mut dry_mock = MockTransport::new(info.clone());
        push_discovery_m5(&mut dry_mock, FEATURES);
        let dry_args = LedSetArgs {
            zone: 0,
            effect: "static".to_string(),
            color: Some("#00FF00".to_string()),
            period: None,
            brightness: None,
            apply: false,
            dry_run: true,
            target: crate::cli::TargetArgs {
                device: None,
                device_index: "direct".to_string(),
            },
        };
        let dry_res = led_set_from_transport(dry_mock, DeviceIndex::DIRECT, &dry_args).unwrap();
        assert!(!dry_res.applied);
        assert_eq!(dry_res.effect, "static");
        assert_eq!(dry_res.color.as_deref(), Some("#00FF00"));

        // 3. led set --apply
        let mut apply_mock = MockTransport::new(info);
        push_discovery_m5(&mut apply_mock, FEATURES);
        // fn 3 setLedEffect
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(3),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );
        let mut apply_args = dry_args;
        apply_args.apply = true;
        apply_args.dry_run = false;
        let apply_res =
            led_set_from_transport(apply_mock, DeviceIndex::DIRECT, &apply_args).unwrap();
        assert!(apply_res.applied);
        assert!(apply_res.verified);
    }

    #[test]
    fn host_mode_get_and_set_dry_run_and_apply() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");

        // 1. host-mode get
        let mut get_mock = MockTransport::new(info.clone());
        push_discovery_m5(&mut get_mock, FEATURES);
        // fn 0 getDescription
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x02, 0x01, 0x05, 0x0B, 0x10, 0x00],
            )
            .unwrap(),
        );
        // fn 2 getOnboardMode -> Onboard (0x01)
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00],
            )
            .unwrap(),
        );
        // fn 4 getCurrentProfile -> 0x0001
        get_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(4),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01],
            )
            .unwrap(),
        );

        let report = host_mode_get_from_transport(get_mock, DeviceIndex::DIRECT).unwrap();
        assert_eq!(report.mode, "onboard");
        assert_eq!(report.current_profile, 1);
        assert_eq!(report.profile_count, 5);

        // 2. host-mode set host --dry-run
        let mut dry_mock = MockTransport::new(info.clone());
        push_discovery_m5(&mut dry_mock, FEATURES);
        // fn 2 getOnboardMode
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00],
            )
            .unwrap(),
        );
        let dry_args = HostModeSetArgs {
            mode: "host".to_string(),
            apply: false,
            dry_run: true,
            target: crate::cli::TargetArgs {
                device: None,
                device_index: "direct".to_string(),
            },
        };
        let dry_res =
            host_mode_set_from_transport(dry_mock, DeviceIndex::DIRECT, &dry_args).unwrap();
        assert!(!dry_res.applied);
        assert_eq!(dry_res.previous_mode, "onboard");
        assert_eq!(dry_res.requested_mode, "host");
        assert_eq!(dry_res.planned_frame.params_hex, "02");

        // 3. host-mode set host --apply
        let mut apply_mock = MockTransport::new(info);
        push_discovery_m5(&mut apply_mock, FEATURES);
        // previous mode fn 2
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00],
            )
            .unwrap(),
        );
        // write mode fn 1
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );
        // readback mode fn 2 -> 0x02
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00],
            )
            .unwrap(),
        );
        let mut apply_args = dry_args;
        apply_args.apply = true;
        apply_args.dry_run = false;
        let apply_res =
            host_mode_set_from_transport(apply_mock, DeviceIndex::DIRECT, &apply_args).unwrap();
        assert!(apply_res.applied);
        assert!(apply_res.verified);
        assert_eq!(apply_res.readback_mode.as_deref(), Some("host"));
    }
}

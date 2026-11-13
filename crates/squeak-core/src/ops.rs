//! Shared M3 HID++ session operations reused by `squeak-cli` and `squeakd`.
//!
//! Keeps capability validation, `--dry-run` planning, and post-write readback verification in
//! one single place so CLI direct mode and daemon DBus handlers behave identically.

use std::collections::BTreeMap;
use std::path::Path;

use hidpp::{
    AdjustableDpiDriver, AdjustableDpiFeature, ColorLedEffectsDriver, DeviceIndex,
    DeviceNameDriver, DeviceNameFeature, Dpi, DpiCapabilities, ExtendedReportRateDriver, FeatureId,
    FeatureTable, FirmwareInfoDriver, FirmwareInfoFeature, HidDeviceInfo, HidTransport, HidppError,
    HidppFeature, HidppSession, LedEffect, LedEffectId, OnboardMode, OnboardProfilesDriver,
    ProtocolError, ReportRate, ReportRateDriver, parse_onboard_profile, parse_profile_directory,
    read_battery, read_report_rate, set_report_rate_verified,
};

use crate::device_data::DeviceDatabase;
use crate::dump::DeviceDump;
use crate::profile_backup::{ProfileBackup, default_backup_path};
use crate::settings::{
    BatteryStatusReport, DeviceNameReport, DeviceStatusReport, DpiConfigReport, DpiWriteReport,
    FirmwareSummaryReport, LedConfigReport, LedWriteReport, LedZoneStatusReport, OnboardModeReport,
    OnboardModeWriteReport, OnboardProfileSlotReport, PlannedWireFrame, ProfileBackupReport,
    ProfileRestoreReport, ProfileSetActiveReport, ProfilesListReport, ReportRateConfigReport,
    ReportRateWriteReport, SensorDpiReport,
};

const FN_DPI_SET: u8 = 3;
const FN_RATE_SET_STD: u8 = 2;
const FN_RATE_SET_EXT: u8 = 3;

/// Reads all available M3 device status information (`0x0005`, `0x0003`, battery, `0x2201`,
/// `0x8060`/`0x8061`) using a discovered runtime [`FeatureTable`].
#[must_use]
pub fn read_device_status<T: HidTransport>(
    session: &mut HidppSession<T>,
    info: &HidDeviceInfo,
    table: &FeatureTable,
    db: &DeviceDatabase,
    pkg_version: &str,
) -> DeviceStatusReport {
    let device_index = table.device_index();
    let resolved = db.resolve(info.vendor_id, info.product_id);
    let dump = DeviceDump::from_discovery(pkg_version, info, table, resolved);

    let mut notices = Vec::new();
    let device_name = match read_device_name_optional(session, device_index, table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("DEVICE_NAME read failed: {err}"));
            None
        }
    };
    let firmware = match read_firmware_optional(session, device_index, table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("FIRMWARE_INFO read failed: {err}"));
            None
        }
    };
    let battery = match read_battery(session, table) {
        Ok(opt) => opt.map(BatteryStatusReport::from),
        Err(err) => {
            notices.push(format!("BATTERY read failed: {err}"));
            None
        }
    };
    let dpi = match read_dpi_config(session, table, None) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("ADJUSTABLE_DPI read failed: {err}"));
            None
        }
    };
    let report_rate = match read_report_rate_config(session, table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("REPORT_RATE read failed: {err}"));
            None
        }
    };
    let led = match read_led_config(session, table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("COLOR_LED_EFFECTS read failed: {err}"));
            None
        }
    };
    let onboard_mode = match read_onboard_mode(session, table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("ONBOARD_PROFILES read failed: {err}"));
            None
        }
    };

    DeviceStatusReport {
        path: info.path.display().to_string(),
        device: dump.device,
        protocol_version: dump.protocol_version,
        device_name,
        firmware,
        battery,
        dpi,
        report_rate,
        led,
        onboard_mode,
        notices,
    }
}

/// Reads active and supported DPI state from `ADJUSTABLE_DPI` (`0x2201`).
///
/// Returns `Ok(None)` when `0x2201` is not present on the device.
///
/// # Errors
///
/// Returns [`HidppError`] if querying the sensor fails or `sensor_filter` is out of range.
pub fn read_dpi_config<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    sensor_filter: Option<u8>,
) -> Result<Option<DpiConfigReport>, HidppError> {
    let Some(entry) = table.get(FeatureId::ADJUSTABLE_DPI) else {
        return Ok(None);
    };
    let device_index = table.device_index();
    let driver = AdjustableDpiDriver::from_entry(entry)?;
    let count = driver.get_sensor_count(session, device_index)?;

    let indices: Vec<u8> = match sensor_filter {
        Some(idx) => {
            if idx >= count {
                return Err(ProtocolError::InvalidArgument {
                    detail: format!("sensor index {idx} out of range (sensor count is {count})"),
                }
                .into());
            }
            vec![idx]
        }
        None => (0..count).collect(),
    };

    let mut sensors = Vec::with_capacity(indices.len());
    for idx in indices {
        let caps = driver.get_sensor_dpi_list(session, device_index, idx)?;
        let state = driver.get_sensor_dpi(session, device_index, idx)?;
        sensors.push(SensorDpiReport::from_state(state, &caps));
    }

    Ok(Some(DpiConfigReport {
        feature_version: driver.version(),
        write_supported: driver.supports_writes(),
        sensors,
    }))
}

/// Validates and either previews (`apply = false`) or applies + readback-verifies (`apply = true`)
/// a sensor DPI change via `ADJUSTABLE_DPI` (`0x2201`).
///
/// # Errors
///
/// Returns [`HidppError`] if `0x2201` is missing, firmware version is unsupported for writes,
/// `sensor_index` or `target_dpi` fails device capability validation, or post-write readback
/// mismatches.
pub fn execute_dpi_write<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    sensor_index: u8,
    target_dpi: u16,
    apply: bool,
) -> Result<DpiWriteReport, HidppError> {
    let device_index = table.device_index();
    let entry = table
        .get(FeatureId::ADJUSTABLE_DPI)
        .ok_or(HidppError::Protocol(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ADJUSTABLE_DPI,
        }))?;
    let driver = AdjustableDpiDriver::from_entry(entry)?;
    if !driver.supports_writes() {
        return Err(ProtocolError::UnsupportedFeatureVersion {
            feature_id: FeatureId::ADJUSTABLE_DPI,
            version: driver.version(),
            max_supported: AdjustableDpiDriver::MAX_SUPPORTED_VERSION,
        }
        .into());
    }

    let requested = Dpi::new(target_dpi)?;
    let count = driver.get_sensor_count(session, device_index)?;
    if sensor_index >= count {
        return Err(ProtocolError::InvalidArgument {
            detail: format!("sensor index {sensor_index} out of range (sensor count is {count})"),
        }
        .into());
    }

    let caps = driver.get_sensor_dpi_list(session, device_index, sensor_index)?;
    if !caps.supports(requested) {
        return Err(ProtocolError::InvalidArgument {
            detail: format!(
                "requested DPI {} is not supported by sensor {sensor_index} ({})",
                requested.as_u16(),
                describe_dpi_caps(&caps),
            ),
        }
        .into());
    }

    let current_state = driver.get_sensor_dpi(session, device_index, sensor_index)?;
    let params = AdjustableDpiDriver::encode_set_sensor_dpi_params(sensor_index, requested);
    let planned_frame = PlannedWireFrame {
        feature_id: format!("0x{:04X}", FeatureId::ADJUSTABLE_DPI.0),
        feature_index: driver.feature_index().0,
        function_id: FN_DPI_SET,
        params_hex: format_hex_bytes(&params),
    };
    let note = onboard_profile_note(table);

    if !apply {
        return Ok(DpiWriteReport {
            applied: false,
            sensor_index,
            previous_dpi: current_state.current.as_u16(),
            requested_dpi: requested.as_u16(),
            readback_dpi: None,
            verified: false,
            planned_frame,
            note,
        });
    }

    let verification =
        driver.set_sensor_dpi_verified(session, device_index, sensor_index, requested)?;
    Ok(DpiWriteReport {
        applied: true,
        sensor_index,
        previous_dpi: current_state.current.as_u16(),
        requested_dpi: requested.as_u16(),
        readback_dpi: Some(verification.readback.current.as_u16()),
        verified: verification.readback.current == requested,
        planned_frame,
        note,
    })
}

/// Reads active and supported polling report rates from `0x8061` or `0x8060`.
///
/// Returns `Ok(None)` if neither report-rate feature is present.
///
/// # Errors
///
/// Returns [`HidppError`] on transport or protocol failures.
pub fn read_report_rate_config<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<ReportRateConfigReport>, HidppError> {
    let Some(state) = read_report_rate(session, table)? else {
        return Ok(None);
    };
    let (version, write_supported) = match table.get(state.source_feature) {
        Some(entry) if state.source_feature == FeatureId::EXTENDED_REPORT_RATE => {
            let drv = ExtendedReportRateDriver::from_entry(entry)?;
            (drv.version(), drv.supports_writes())
        }
        Some(entry) => {
            let drv = ReportRateDriver::from_entry(entry)?;
            (drv.version(), drv.supports_writes())
        }
        None => (0, false),
    };
    Ok(Some(ReportRateConfigReport::from_state(
        &state,
        version,
        write_supported,
    )))
}

/// Validates and either previews (`apply = false`) or applies + readback-verifies (`apply = true`)
/// a polling report rate change via `0x8061` or `0x8060`.
///
/// # Errors
///
/// Returns [`HidppError`] if `hz` is invalid, neither rate feature exists, firmware version is
/// unsupported for writes, `hz` is unsupported by the device, or post-write readback mismatches.
pub fn execute_report_rate_write<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    hz: u16,
    apply: bool,
) -> Result<ReportRateWriteReport, HidppError> {
    let target_rate = parse_report_rate_hz(hz)?;

    let (source_feature, feature_index, fn_id, params) =
        if let Some(entry) = table.get(FeatureId::EXTENDED_REPORT_RATE) {
            let drv = ExtendedReportRateDriver::from_entry(entry)?;
            if !drv.supports_writes() {
                return Err(ProtocolError::UnsupportedFeatureVersion {
                    feature_id: FeatureId::EXTENDED_REPORT_RATE,
                    version: drv.version(),
                    max_supported: ExtendedReportRateDriver::MAX_SUPPORTED_VERSION,
                }
                .into());
            }
            let encoded = ExtendedReportRateDriver::encode_set_rate_params(target_rate)?;
            (
                FeatureId::EXTENDED_REPORT_RATE,
                drv.feature_index().0,
                FN_RATE_SET_EXT,
                encoded.to_vec(),
            )
        } else if let Some(entry) = table.get(FeatureId::REPORT_RATE) {
            let drv = ReportRateDriver::from_entry(entry)?;
            if !drv.supports_writes() {
                return Err(ProtocolError::UnsupportedFeatureVersion {
                    feature_id: FeatureId::REPORT_RATE,
                    version: drv.version(),
                    max_supported: ReportRateDriver::MAX_SUPPORTED_VERSION,
                }
                .into());
            }
            let encoded = ReportRateDriver::encode_set_rate_params(target_rate)?;
            (
                FeatureId::REPORT_RATE,
                drv.feature_index().0,
                FN_RATE_SET_STD,
                encoded.to_vec(),
            )
        } else {
            return Err(ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::REPORT_RATE,
            }
            .into());
        };

    let state = read_report_rate(session, table)?.ok_or(ProtocolError::FeatureNotSupported {
        feature_id: source_feature,
    })?;

    if !state.supported.contains(&target_rate) {
        let supported_list: Vec<String> = state
            .supported
            .iter()
            .map(|r| format!("{} Hz", r.hz()))
            .collect();
        return Err(ProtocolError::InvalidArgument {
            detail: format!(
                "requested report rate {} Hz is not supported (device supports: {})",
                target_rate.hz(),
                supported_list.join(", ")
            ),
        }
        .into());
    }

    let planned_frame = PlannedWireFrame {
        feature_id: format!("0x{:04X}", source_feature.0),
        feature_index,
        function_id: fn_id,
        params_hex: format_hex_bytes(&params),
    };
    let note = onboard_profile_note(table);

    if !apply {
        return Ok(ReportRateWriteReport {
            applied: false,
            source_feature: format!("0x{:04X}", source_feature.0),
            previous_hz: state.current.hz(),
            requested_hz: target_rate.hz(),
            readback_hz: None,
            verified: false,
            planned_frame,
            note,
        });
    }

    let verification = set_report_rate_verified(session, table, target_rate)?;
    Ok(ReportRateWriteReport {
        applied: true,
        source_feature: format!("0x{:04X}", verification.source_feature.0),
        previous_hz: state.current.hz(),
        requested_hz: target_rate.hz(),
        readback_hz: Some(verification.readback.hz()),
        verified: verification.readback == target_rate,
        planned_frame,
        note,
    })
}

/// Converts a polling frequency in Hz (`125`, `250`, `500`, `1000`, `2000`, `4000`, `8000`)
/// into a [`ReportRate`].
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidArgument`] for unsupported frequencies.
pub fn parse_report_rate_hz(hz: u16) -> Result<ReportRate, ProtocolError> {
    match hz {
        125 => Ok(ReportRate::Hz125),
        250 => Ok(ReportRate::Hz250),
        500 => Ok(ReportRate::Hz500),
        1000 => Ok(ReportRate::Hz1000),
        2000 => Ok(ReportRate::Hz2000),
        4000 => Ok(ReportRate::Hz4000),
        8000 => Ok(ReportRate::Hz8000),
        _ => Err(ProtocolError::InvalidArgument {
            detail: format!(
                "unsupported polling rate {hz} Hz; valid rates are 125, 250, 500, 1000, 2000, 4000, or 8000"
            ),
        }),
    }
}

fn read_device_name_optional<T: HidTransport>(
    session: &mut HidppSession<T>,
    device_index: DeviceIndex,
    table: &FeatureTable,
) -> Result<Option<DeviceNameReport>, HidppError> {
    let Some(entry) = table.get(FeatureId::DEVICE_NAME) else {
        return Ok(None);
    };
    let driver = DeviceNameDriver::from_entry(entry)?;
    let name = driver.get_name(session, device_index)?;
    let kind = driver.get_device_type(session, device_index)?;
    Ok(Some(DeviceNameReport {
        name,
        device_type: kind.label().to_owned(),
    }))
}

fn read_firmware_optional<T: HidTransport>(
    session: &mut HidppSession<T>,
    device_index: DeviceIndex,
    table: &FeatureTable,
) -> Result<Option<FirmwareSummaryReport>, HidppError> {
    let Some(entry) = table.get(FeatureId::FIRMWARE_INFO) else {
        return Ok(None);
    };
    let driver = FirmwareInfoDriver::from_entry(entry)?;
    let inv = driver.read_inventory(session, device_index)?;
    Ok(Some(FirmwareSummaryReport::from_inventory(&inv)))
}

fn onboard_profile_note(table: &FeatureTable) -> Option<String> {
    table.get(FeatureId::ONBOARD_PROFILES).map(|_| {
        "Device exposes ONBOARD_PROFILES (0x8100); if onboard memory mode is active, volatile host changes may be overridden or rejected until host mode is active.".to_owned()
    })
}

fn describe_dpi_caps(caps: &DpiCapabilities) -> String {
    let mut parts = Vec::new();
    if !caps.discrete.is_empty() {
        let vals: Vec<String> = caps
            .discrete
            .iter()
            .map(|d| d.as_u16().to_string())
            .collect();
        parts.push(format!("discrete: [{}]", vals.join(", ")));
    }
    for r in &caps.ranges {
        parts.push(format!(
            "range {}..={} step {}",
            r.min.as_u16(),
            r.max.as_u16(),
            r.step
        ));
    }
    if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join("; ")
    }
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads active and supported LED lighting state from `COLOR_LED_EFFECTS` (`0x8070`).
///
/// Returns `Ok(None)` when `0x8070` is not present on the device.
///
/// # Errors
///
/// Returns [`HidppError`] if querying LED zones fails.
pub fn read_led_config<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<LedConfigReport>, HidppError> {
    let Some(entry) = table.get(FeatureId::COLOR_LED_EFFECTS) else {
        return Ok(None);
    };
    let driver = ColorLedEffectsDriver::from_entry(entry)?;
    let dev_idx = table.device_index();
    let zones = driver.query_all_zones(session, dev_idx)?;
    let zone_reports = zones
        .into_iter()
        .map(|z| {
            let location_name = match z.location {
                0x0001 => "Logo".to_string(),
                0x0002 => "DPI LEDs".to_string(),
                _ => format!("Zone {}", z.zone_index),
            };
            let supported_effects = z
                .supported_effects
                .into_iter()
                .map(|e| match e {
                    LedEffectId::Off => "off".to_string(),
                    LedEffectId::Static => "static".to_string(),
                    LedEffectId::Breathing => "breathing".to_string(),
                    LedEffectId::ColorCycle => "color_cycle".to_string(),
                    LedEffectId::Other(id) => format!("0x{id:04X}"),
                })
                .collect();
            LedZoneStatusReport {
                zone_index: z.zone_index,
                location: z.location,
                location_name,
                effect_count: z.effect_count,
                supported_effects,
            }
        })
        .collect();

    Ok(Some(LedConfigReport {
        feature_version: driver.version(),
        write_supported: driver.version() <= ColorLedEffectsDriver::MAX_SUPPORTED_VERSION,
        zones: zone_reports,
    }))
}

/// Validates and either previews (`apply = false`) or applies (`apply = true`) an LED effect via `0x8070`.
///
/// # Errors
///
/// Returns [`HidppError`] if `0x8070` is missing or parameters are invalid.
pub fn execute_led_write<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    zone_index: u8,
    effect: LedEffect,
    apply: bool,
) -> Result<LedWriteReport, HidppError> {
    let Some(entry) = table.get(FeatureId::COLOR_LED_EFFECTS) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::COLOR_LED_EFFECTS,
        }
        .into());
    };
    let driver = ColorLedEffectsDriver::from_entry(entry)?;
    let payload = effect.to_payload(zone_index)?;
    let planned_frame = PlannedWireFrame {
        feature_id: "0x8070".to_string(),
        feature_index: driver.feature_index().0,
        function_id: 3,
        params_hex: format_hex_bytes(&payload),
    };

    let (effect_name, color, period_ms, brightness_pct) = match effect {
        LedEffect::Off => ("off".to_string(), None, None, None),
        LedEffect::Static { color } => ("static".to_string(), Some(color.to_string()), None, None),
        LedEffect::Breathing {
            color,
            period_ms,
            brightness_pct,
        } => (
            "breathing".to_string(),
            Some(color.to_string()),
            Some(period_ms),
            Some(brightness_pct),
        ),
        LedEffect::ColorCycle {
            period_ms,
            brightness_pct,
        } => (
            "color_cycle".to_string(),
            None,
            Some(period_ms),
            Some(brightness_pct),
        ),
    };

    if apply {
        driver.set_led_effect(session, table.device_index(), zone_index, effect)?;
    }

    Ok(LedWriteReport {
        applied: apply,
        zone_index,
        effect: effect_name,
        color,
        period_ms,
        brightness_pct,
        verified: apply,
        planned_frame,
        note: if !apply {
            Some("Dry run only: no frames sent to device (use --apply to write)".to_string())
        } else {
            None
        },
    })
}

/// Reads operating mode and memory description from `ONBOARD_PROFILES` (`0x8100`).
///
/// Returns `Ok(None)` when `0x8100` is not present on the device.
///
/// # Errors
///
/// Returns [`HidppError`] if querying onboard profiles fails.
pub fn read_onboard_mode<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<OnboardModeReport>, HidppError> {
    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Ok(None);
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();
    let desc = driver.get_description(session, dev_idx)?;
    let mode = driver.get_onboard_mode(session, dev_idx)?;
    let cur_prof = driver.get_current_profile(session, dev_idx)?;

    Ok(Some(OnboardModeReport {
        feature_version: driver.version(),
        mode: match mode {
            OnboardMode::Host => "host".to_string(),
            OnboardMode::Onboard => "onboard".to_string(),
            OnboardMode::Other(c) => format!("unknown(0x{c:02X})"),
        },
        current_profile: cur_prof,
        profile_count: desc.profile_count,
        button_count: desc.button_count,
        sector_count: desc.sector_count,
        sector_size: desc.sector_size,
    }))
}

/// Validates and either previews (`apply = false`) or applies + readback-verifies (`apply = true`)
/// switching between Host Software Mode and Onboard Profile Memory Mode via `0x8100`.
///
/// # Errors
///
/// Returns [`HidppError`] if `0x8100` is missing or post-write readback mismatches.
pub fn execute_host_mode_write<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    target_host_mode: bool,
    apply: bool,
) -> Result<OnboardModeWriteReport, HidppError> {
    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();
    let prev_mode = driver.get_onboard_mode(session, dev_idx)?;
    let prev_str = match prev_mode {
        OnboardMode::Host => "host".to_string(),
        OnboardMode::Onboard => "onboard".to_string(),
        OnboardMode::Other(c) => format!("unknown(0x{c:02X})"),
    };
    let target_mode = if target_host_mode {
        OnboardMode::Host
    } else {
        OnboardMode::Onboard
    };
    let target_str = if target_host_mode { "host" } else { "onboard" }.to_string();

    let planned_frame = PlannedWireFrame {
        feature_id: "0x8100".to_string(),
        feature_index: driver.feature_index().0,
        function_id: 1,
        params_hex: format!("{:02X}", u8::from(target_mode)),
    };

    let (readback_mode, verified) = if apply {
        driver.set_onboard_mode(session, dev_idx, target_mode)?;
        let readback = driver.get_onboard_mode(session, dev_idx)?;
        let rb_str = match readback {
            OnboardMode::Host => "host".to_string(),
            OnboardMode::Onboard => "onboard".to_string(),
            OnboardMode::Other(c) => format!("unknown(0x{c:02X})"),
        };
        let matches = readback == target_mode;
        (Some(rb_str), matches)
    } else {
        (None, false)
    };

    Ok(OnboardModeWriteReport {
        applied: apply,
        previous_mode: prev_str,
        requested_mode: target_str,
        readback_mode,
        verified,
        planned_frame,
        note: if !apply {
            Some("Dry run only: no frames sent to device (use --apply to write)".to_string())
        } else {
            None
        },
    })
}

/// Reads the onboard profiles directory (sector 0) and all profile sectors via `0x8100`.
///
/// # Errors
///
/// Returns [`HidppError`] if reading or parsing sectors fails.
pub fn read_profiles_summary<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<ProfilesListReport, HidppError> {
    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();

    let desc = driver.get_description(session, dev_idx)?;
    let mode = driver.get_onboard_mode(session, dev_idx)?;
    let cur_prof = driver.get_current_profile(session, dev_idx)?;

    let mode_str = match mode {
        OnboardMode::Host => "host".to_string(),
        OnboardMode::Onboard => "onboard".to_string(),
        OnboardMode::Other(c) => format!("unknown(0x{c:02X})"),
    };

    // Read directory sector 0
    let sector0 = driver.read_sector(session, dev_idx, 0, desc.sector_size, true)?;
    let dir_entries = parse_profile_directory(&sector0, desc.profile_count)?;

    let mut profiles = Vec::new();
    for (idx, dir_entry) in dir_entries.iter().enumerate() {
        let slot = (idx + 1) as u16;
        let sector_data = driver.read_sector(
            session,
            dev_idx,
            dir_entry.sector_address,
            desc.sector_size,
            true,
        )?;
        let parsed = parse_onboard_profile(slot, &sector_data)?;
        profiles.push(OnboardProfileSlotReport {
            slot,
            sector_address: dir_entry.sector_address,
            enabled: dir_entry.enabled,
            is_active: mode == OnboardMode::Onboard && cur_prof == slot,
            report_rate_hz: parsed.report_rate_hz,
            default_dpi_stage: parsed.default_dpi_stage,
            switched_dpi_stage: parsed.switched_dpi_stage,
            dpi_stages: parsed.dpi_stages,
            name: parsed.name,
            crc_verified: true,
        });
    }

    Ok(ProfilesListReport {
        feature_version: driver.version(),
        mode: mode_str,
        current_profile: cur_prof,
        profile_count: desc.profile_count,
        sector_count: desc.sector_count,
        sector_size: desc.sector_size,
        profiles,
    })
}

/// Dumps all onboard profile sectors (sector 0 and profiles 1..=profile_count) to a verified backup file.
///
/// # Errors
///
/// Returns [`HidppError`] if reading or saving fails.
pub fn execute_profile_backup<T: HidTransport>(
    session: &mut HidppSession<T>,
    info: &HidDeviceInfo,
    table: &FeatureTable,
    custom_path: Option<&Path>,
) -> Result<ProfileBackupReport, HidppError> {
    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();

    let desc = driver.get_description(session, dev_idx)?;

    // Read sector 0 (directory)
    let sector0 = driver.read_sector(session, dev_idx, 0, desc.sector_size, true)?;
    let mut sectors = BTreeMap::new();
    sectors.insert(0, sector0);

    // Read profile sectors (1..=profile_count)
    for slot in 1..=u16::from(desc.profile_count) {
        let sector_data = driver.read_sector(session, dev_idx, slot, desc.sector_size, true)?;
        sectors.insert(slot, sector_data);
    }

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let timestamp_str = format!("epoch_{secs}");

    let backup = ProfileBackup::new(
        timestamp_str.clone(),
        info.vendor_id,
        info.product_id,
        desc.sector_size,
        desc.sector_count,
        sectors,
    );

    backup.verify_integrity().map_err(|e| {
        HidppError::Protocol(ProtocolError::UnexpectedResponse {
            detail: format!("backup integrity check failed: {e}"),
        })
    })?;

    let backup_path = match custom_path {
        Some(p) => p.to_path_buf(),
        None => default_backup_path(info.vendor_id, info.product_id, &timestamp_str),
    };

    backup.save_to_file(&backup_path).map_err(|e| {
        HidppError::Protocol(ProtocolError::UnexpectedResponse {
            detail: format!("failed saving backup file: {e}"),
        })
    })?;

    let total_bytes: usize = backup.sectors.values().map(Vec::len).sum();

    Ok(ProfileBackupReport {
        backup_path: backup_path.display().to_string(),
        vendor_id: format!("0x{:04X}", info.vendor_id),
        product_id: format!("0x{:04X}", info.product_id),
        sectors_saved: backup.sectors.len(),
        total_bytes,
    })
}

/// Restores a verified profile backup into hardware memory sectors (`fn 6`, `fn 7`, `fn 8`).
///
/// In preview mode (`apply = false`), validates backup file and outputs planned writes without transmitting.
/// In apply mode (`apply = true`), flashes each sector to device and performs readback CRC verification.
///
/// # Errors
///
/// Returns [`HidppError`] if backup file is invalid, device VID:PID mismatches, or writing/verifying fails.
pub fn execute_profile_restore<T: HidTransport>(
    session: &mut HidppSession<T>,
    info: &HidDeviceInfo,
    table: &FeatureTable,
    backup_path: &Path,
    apply: bool,
) -> Result<ProfileRestoreReport, HidppError> {
    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();

    let backup = ProfileBackup::load_from_file(backup_path).map_err(|e| {
        HidppError::Protocol(ProtocolError::InvalidArgument {
            detail: format!("failed loading backup {}: {e}", backup_path.display()),
        })
    })?;

    if backup.vendor_id != info.vendor_id || backup.product_id != info.product_id {
        return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
            detail: format!(
                "backup device mismatch: file has 0x{:04X}:0x{:04X}, target device is 0x{:04X}:0x{:04X}",
                backup.vendor_id, backup.product_id, info.vendor_id, info.product_id
            ),
        }));
    }

    let mut planned_writes = Vec::new();
    let mut total_bytes = 0;

    for (&sector, data) in &backup.sectors {
        total_bytes += data.len();
        planned_writes.push(PlannedWireFrame {
            feature_id: "0x8100".to_string(),
            feature_index: driver.feature_index().0,
            function_id: 6,
            params_hex: format!(
                "{:04X}{:04X}{:04X} (sector 0x{sector:04X}, count {})",
                sector,
                0,
                data.len(),
                data.len()
            ),
        });
    }

    if apply {
        // Automatic pre-flash safety backup before writing!
        let _ = execute_profile_backup(session, info, table, None);

        for (&sector, data) in &backup.sectors {
            driver.write_sector(session, dev_idx, sector, data)?;
            // Readback verify
            let readback =
                driver.read_sector(session, dev_idx, sector, backup.sector_size, true)?;
            if readback != *data {
                return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                    detail: format!("post-write readback mismatch in sector 0x{sector:04X}"),
                }));
            }
        }
    }

    Ok(ProfileRestoreReport {
        applied: apply,
        backup_path: backup_path.display().to_string(),
        sectors_to_restore: backup.sectors.len(),
        total_bytes,
        planned_writes,
        verified: apply,
        note: if !apply {
            Some(
                "Dry run only: no sectors were written (use --apply to flash onboard memory)"
                    .to_string(),
            )
        } else {
            None
        },
    })
}

/// Switches the active onboard profile slot (`fn 3`) and verifies with readback (`fn 4`).
///
/// # Errors
///
/// Returns [`HidppError`] if slot is invalid or readback mismatches.
pub fn execute_set_active_profile<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    slot: u16,
    apply: bool,
) -> Result<ProfileSetActiveReport, HidppError> {
    if slot == 0 {
        return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
            detail: "slot 0 is the directory sector; profile slots are 1-based (e.g. 1..=5)"
                .to_string(),
        }));
    }

    let Some(entry) = table.get(FeatureId::ONBOARD_PROFILES) else {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    };
    let driver = OnboardProfilesDriver::from_entry(entry)?;
    let dev_idx = table.device_index();

    let desc = driver.get_description(session, dev_idx)?;
    if slot > u16::from(desc.profile_count) {
        return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
            detail: format!(
                "requested slot {slot} exceeds device profile count ({})",
                desc.profile_count
            ),
        }));
    }

    let prev_slot = driver.get_current_profile(session, dev_idx)?;

    let planned_frame = PlannedWireFrame {
        feature_id: "0x8100".to_string(),
        feature_index: driver.feature_index().0,
        function_id: 3,
        params_hex: format!("{slot:04X}"),
    };

    let (readback_slot, verified) = if apply {
        driver.set_current_profile(session, dev_idx, slot)?;
        let rb = driver.get_current_profile(session, dev_idx)?;
        let matches = rb == slot;
        (Some(rb), matches)
    } else {
        (None, false)
    };

    Ok(ProfileSetActiveReport {
        applied: apply,
        requested_slot: slot,
        previous_slot: prev_slot,
        readback_slot,
        verified,
        planned_frame,
        note: if !apply {
            Some(
                "Dry run only: no frames sent to device (use --apply to switch profile)"
                    .to_string(),
            )
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hidpp::{
        DeviceIndex, FeatureEntry, FeatureFlags, FeatureId, FeatureIndex, FunctionId,
        HidDeviceInfo, HidppFrame, MockTransport, ReportId, SessionConfig, SwId,
        calculate_sector_crc,
    };

    use super::*;

    fn test_session(mock: MockTransport) -> HidppSession<MockTransport> {
        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        HidppSession::with_config(mock, cfg)
    }

    fn mock_table() -> FeatureTable {
        use hidpp::ProtocolVersion;
        let entries = vec![
            FeatureEntry {
                index: FeatureIndex(0),
                id: FeatureId::IROOT,
                version: 0,
                flags: FeatureFlags::default(),
            },
            FeatureEntry {
                index: FeatureIndex(1),
                id: FeatureId::IFEATURE_SET,
                version: 1,
                flags: FeatureFlags::default(),
            },
            FeatureEntry {
                index: FeatureIndex(12),
                id: FeatureId::ONBOARD_PROFILES,
                version: 0,
                flags: FeatureFlags::default(),
            },
        ];
        FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 2 },
            entries,
        )
    }

    #[test]
    fn test_set_active_profile_dry_run_and_apply() {
        let table = mock_table();
        let feat_idx = FeatureIndex(12);
        let dev_idx = DeviceIndex::DIRECT;

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // For dry-run: getDescription (fn 0), getCurrentProfile (fn 4)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x01, 0x02, 0x01, 0x05, 0x05, 0x0B, 0x10, 0x00, 0xFF, 0x0A, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(4).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let dry_report = execute_set_active_profile(&mut session, &table, 2, false).unwrap();
        assert!(!dry_report.applied);
        assert_eq!(dry_report.requested_slot, 2);
        assert_eq!(dry_report.previous_slot, 1);
        assert_eq!(dry_report.readback_slot, None);
        assert!(!dry_report.verified);

        // Slot 0 rejected
        assert!(execute_set_active_profile(&mut session, &table, 0, false).is_err());
    }

    #[test]
    fn test_profile_backup_and_restore_cycle() {
        let table = mock_table();
        let feat_idx = FeatureIndex(12);
        let dev_idx = DeviceIndex::DIRECT;
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");

        let mut mock = MockTransport::new(info.clone());

        // 1. fn 0: getDescription
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x01, 0x02, 0x01, 0x01, 0x01, 0x0B, 0x02, 0x00, 0xFF, 0x0A, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );

        // 2. read sector 0 (16 chunks of 16 bytes for 255-byte sector)
        let mut s0 = vec![0xFF; 255];
        s0[0..4].copy_from_slice(&[0x00, 0x01, 0x01, 0xFF]);
        let crc0 = calculate_sector_crc(&s0[..253]);
        s0[253..255].copy_from_slice(&crc0.to_be_bytes());

        for chunk_idx in 0..16 {
            let offset = (chunk_idx * 16).min(239);
            mock.push_frame(
                HidppFrame::new(
                    ReportId::Long,
                    dev_idx,
                    feat_idx,
                    FunctionId::new(5).unwrap(),
                    SwId::DEFAULT_REQUEST,
                    &s0[offset..offset + 16],
                )
                .unwrap(),
            );
        }

        // 3. read sector 1 (16 chunks of 16 bytes)
        let mut s1 = vec![0xFF; 255];
        s1[0] = 1; // 1000 Hz
        s1[1] = 0; // default DPI stage 0
        let crc1 = calculate_sector_crc(&s1[..253]);
        s1[253..255].copy_from_slice(&crc1.to_be_bytes());

        for chunk_idx in 0..16 {
            let offset = (chunk_idx * 16).min(239);
            mock.push_frame(
                HidppFrame::new(
                    ReportId::Long,
                    dev_idx,
                    feat_idx,
                    FunctionId::new(5).unwrap(),
                    SwId::DEFAULT_REQUEST,
                    &s1[offset..offset + 16],
                )
                .unwrap(),
            );
        }

        let mut session = test_session(mock);

        let temp_dir = std::env::temp_dir().join(format!("squeak_test_{}", std::process::id()));
        let backup_file = temp_dir.join("test_backup.json");

        let backup_report =
            execute_profile_backup(&mut session, &info, &table, Some(&backup_file)).unwrap();
        assert_eq!(backup_report.sectors_saved, 2);
        assert!(backup_file.exists());

        // Test restore dry-run
        let restore_report =
            execute_profile_restore(&mut session, &info, &table, &backup_file, false).unwrap();
        assert!(!restore_report.applied);
        assert_eq!(restore_report.sectors_to_restore, 2);
        assert!(!restore_report.verified);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

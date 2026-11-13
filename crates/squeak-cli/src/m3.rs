//! M3 read/write subcommands (`status`, `dpi`, `report-rate`) and `--dry-run` safety guards.

use std::fmt::Write as _;

use hidpp::{
    AdjustableDpiDriver, AdjustableDpiFeature, DeviceIndex, DeviceNameDriver, DeviceNameFeature,
    Dpi, ExtendedReportRateDriver, FeatureId, FeatureTable, FirmwareInfoDriver,
    FirmwareInfoFeature, HidTransport, HidppFeature, HidppSession, ProtocolError, ReportRate,
    ReportRateDriver, read_battery, read_report_rate, set_report_rate_verified,
};
use squeak_core::{
    BatteryStatusReport, DeviceDatabase, DeviceDump, DeviceNameReport, DeviceStatusReport,
    DpiConfigReport, DpiWriteReport, FirmwareSummaryReport, PlannedWireFrame,
    ReportRateConfigReport, ReportRateWriteReport, SensorDpiReport,
};

use crate::commands::CliError;

const FN_DPI_SET: u8 = 3;
const FN_RATE_SET_STD: u8 = 2;
const FN_RATE_SET_EXT: u8 = 3;

/// Reads all available M3 device status information (`0x0005`, `0x0003`, battery, `0x2201`,
/// `0x8060`/`0x8061`) using runtime-discovered feature indices.
///
/// # Errors
///
/// Returns [`CliError`] if `IRoot` / `IFeatureSet` discovery itself fails. Non-fatal failures
/// reading individual optional features are recorded in [`DeviceStatusReport::notices`].
pub fn status_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    db: &DeviceDatabase,
) -> Result<DeviceStatusReport, CliError> {
    let path = transport.device_info().path.display().to_string();
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    let info = session.device_info().clone();
    let resolved = db.resolve(info.vendor_id, info.product_id);
    let dump = DeviceDump::from_discovery(env!("CARGO_PKG_VERSION"), &info, &table, resolved);

    let mut notices = Vec::new();
    let device_name = match read_device_name_optional(&mut session, device_index, &table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("DEVICE_NAME read failed: {err}"));
            None
        }
    };
    let firmware = match read_firmware_optional(&mut session, device_index, &table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("FIRMWARE_INFO read failed: {err}"));
            None
        }
    };
    let battery = match read_battery(&mut session, &table) {
        Ok(opt) => opt.map(BatteryStatusReport::from),
        Err(err) => {
            notices.push(format!("BATTERY read failed: {err}"));
            None
        }
    };
    let dpi = match read_dpi_optional(&mut session, device_index, &table, None) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("ADJUSTABLE_DPI read failed: {err}"));
            None
        }
    };
    let report_rate = match read_report_rate_optional(&mut session, &table) {
        Ok(val) => val,
        Err(err) => {
            notices.push(format!("REPORT_RATE read failed: {err}"));
            None
        }
    };

    Ok(DeviceStatusReport {
        path,
        device: dump.device,
        protocol_version: dump.protocol_version,
        device_name,
        firmware,
        battery,
        dpi,
        report_rate,
        notices,
    })
}

/// Reads active and supported DPI state from `ADJUSTABLE_DPI` (`0x2201`).
///
/// # Errors
///
/// Returns [`CliError`] if discovery fails, `0x2201` is absent, or `sensor_filter` is out of range.
pub fn dpi_get_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    sensor_filter: Option<u8>,
) -> Result<DpiConfigReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    read_dpi_optional(&mut session, device_index, &table, sensor_filter)?.ok_or_else(|| {
        CliError::Hidpp(
            ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::ADJUSTABLE_DPI,
            }
            .into(),
        )
    })
}

/// Validates and either previews (`apply = false`) or applies + readback-verifies (`apply = true`)
/// a sensor DPI change via `ADJUSTABLE_DPI` (`0x2201`).
///
/// # Errors
///
/// Returns [`CliError`] if `0x2201` is missing, firmware version is unsupported for writes,
/// `sensor_index` or `target_dpi` fails device capability validation, or post-write readback mismatches.
pub fn dpi_set_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    sensor_index: u8,
    target_dpi: u16,
    apply: bool,
) -> Result<DpiWriteReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;

    let entry = table.get(FeatureId::ADJUSTABLE_DPI).ok_or_else(|| {
        CliError::Hidpp(
            ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::ADJUSTABLE_DPI,
            }
            .into(),
        )
    })?;
    let driver = AdjustableDpiDriver::from_entry(entry)?;
    if !driver.supports_writes() {
        return Err(ProtocolError::UnsupportedFeatureVersion {
            feature_id: FeatureId::ADJUSTABLE_DPI,
            version: driver.version(),
            max_supported: AdjustableDpiDriver::MAX_SUPPORTED_VERSION,
        }
        .into());
    }

    let requested = Dpi::new(target_dpi).map_err(hidpp::HidppError::Protocol)?;
    let count = driver.get_sensor_count(&mut session, device_index)?;
    if sensor_index >= count {
        return Err(ProtocolError::InvalidArgument {
            detail: format!("sensor index {sensor_index} out of range (sensor count is {count})"),
        }
        .into());
    }

    let caps = driver.get_sensor_dpi_list(&mut session, device_index, sensor_index)?;
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

    let current_state = driver.get_sensor_dpi(&mut session, device_index, sensor_index)?;
    let params = AdjustableDpiDriver::encode_set_sensor_dpi_params(sensor_index, requested);
    let planned_frame = PlannedWireFrame {
        feature_id: format!("0x{:04X}", FeatureId::ADJUSTABLE_DPI.0),
        feature_index: driver.feature_index().0,
        function_id: FN_DPI_SET,
        params_hex: format_hex_bytes(&params),
    };

    let note = onboard_profile_note(&table);

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
        driver.set_sensor_dpi_verified(&mut session, device_index, sensor_index, requested)?;

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
/// # Errors
///
/// Returns [`CliError`] if discovery fails or neither report-rate feature is supported.
pub fn report_rate_get_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
) -> Result<ReportRateConfigReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    read_report_rate_optional(&mut session, &table)?.ok_or_else(|| {
        CliError::Hidpp(
            ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::REPORT_RATE,
            }
            .into(),
        )
    })
}

/// Validates and either previews (`apply = false`) or applies + readback-verifies (`apply = true`)
/// a polling report rate change via `0x8061` or `0x8060`.
///
/// # Errors
///
/// Returns [`CliError`] if `hz` is invalid, neither rate feature exists, firmware version is
/// unsupported for writes, `hz` is unsupported by the device, or post-write readback mismatches.
pub fn report_rate_set_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    hz: u16,
    apply: bool,
) -> Result<ReportRateWriteReport, CliError> {
    let target_rate = parse_report_rate_hz(hz)?;
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;

    let (source_feature, feature_index, version, supports_writes, fn_id, params) =
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
                drv.version(),
                drv.supports_writes(),
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
                drv.version(),
                drv.supports_writes(),
                FN_RATE_SET_STD,
                encoded.to_vec(),
            )
        } else {
            return Err(ProtocolError::FeatureNotSupported {
                feature_id: FeatureId::REPORT_RATE,
            }
            .into());
        };

    let _ = (version, supports_writes);
    let state = read_report_rate(&mut session, &table)?.ok_or_else(|| {
        CliError::Hidpp(
            ProtocolError::FeatureNotSupported {
                feature_id: source_feature,
            }
            .into(),
        )
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
    let note = onboard_profile_note(&table);

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

    let verification = set_report_rate_verified(&mut session, &table, target_rate)?;
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

/// Converts a CLI Hz argument (`125`, `250`, `500`, `1000`, `2000`, `4000`, `8000`) into [`ReportRate`].
///
/// # Errors
///
/// Returns [`CliError::Usage`] for unsupported frequencies.
pub fn parse_report_rate_hz(hz: u16) -> Result<ReportRate, CliError> {
    match hz {
        125 => Ok(ReportRate::Hz125),
        250 => Ok(ReportRate::Hz250),
        500 => Ok(ReportRate::Hz500),
        1000 => Ok(ReportRate::Hz1000),
        2000 => Ok(ReportRate::Hz2000),
        4000 => Ok(ReportRate::Hz4000),
        8000 => Ok(ReportRate::Hz8000),
        _ => Err(CliError::Usage(format!(
            "unsupported polling rate {hz} Hz; valid rates are 125, 250, 500, 1000, 2000, 4000, or 8000"
        ))),
    }
}

/// Formats `squeak status` output for humans.
#[must_use]
pub fn format_status_human(report: &DeviceStatusReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Device:           {} (0x{:04x}:0x{:04x}, {}, HID++ {})",
        report.device.matched_name,
        report.device.vendor_id,
        report.device.product_id,
        report.device.bus,
        report.protocol_version,
    );
    let _ = writeln!(out, "Node:             {}", report.path);

    if let Some(name) = &report.device_name {
        let _ = writeln!(
            out,
            "Device name:      {} ({})",
            name.name, name.device_type
        );
    }

    if let Some(fw) = &report.firmware {
        let tr = if fw.transports.is_empty() {
            "-".to_owned()
        } else {
            fw.transports.join(", ")
        };
        let _ = writeln!(
            out,
            "Firmware:         {} entities (transports: {tr})",
            fw.entity_count
        );
        for ent in &fw.entities {
            let active_tag = if ent.active { " [active]" } else { "" };
            let _ = writeln!(
                out,
                "  - [{}] {}: {}{}",
                ent.index, ent.kind, ent.version, active_tag
            );
        }
    }

    if let Some(bat) = &report.battery {
        let pct = bat
            .percentage
            .map(|p| format!("{p}%"))
            .or_else(|| bat.approximate_percent.map(|p| format!("~{p}%")))
            .unwrap_or_else(|| "unknown".to_owned());
        let _ = writeln!(
            out,
            "Battery:          {pct}, {} ({})",
            bat.charging_status, bat.source_feature
        );
    }

    if let Some(dpi) = &report.dpi {
        let _ = writeln!(
            out,
            "{}",
            format_dpi_get_human_lines("DPI:             ", dpi)
        );
    }

    if let Some(rr) = &report.report_rate {
        let _ = writeln!(
            out,
            "Report rate:      {} Hz ({} us, supported: {}) [{} v{}]",
            rr.current_hz,
            rr.current_interval_us,
            join_u16(&rr.supported_hz, " Hz"),
            rr.source_feature,
            rr.feature_version
        );
    }

    if !report.notices.is_empty() {
        let _ = writeln!(out, "Notices:");
        for notice in &report.notices {
            let _ = writeln!(out, "  - {notice}");
        }
    }

    out.trim_end().to_owned()
}

/// Formats `squeak dpi get` output for humans.
#[must_use]
pub fn format_dpi_get_human(report: &DpiConfigReport) -> String {
    format_dpi_get_human_lines("ADJUSTABLE_DPI (0x2201):", report)
}

fn format_dpi_get_human_lines(header: &str, report: &DpiConfigReport) -> String {
    let mut out = String::new();
    let rw = if report.write_supported {
        "read/write"
    } else {
        "read-only (unsupported version)"
    };
    let _ = writeln!(out, "{header} v{} ({rw})", report.feature_version);
    for sensor in &report.sensors {
        let def_str = sensor
            .default_dpi
            .map(|d| format!(", default {d}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  - sensor {}: current {} DPI{def_str} ({})",
            sensor.sensor_index,
            sensor.current_dpi,
            describe_caps_report(&sensor.capabilities)
        );
    }
    out.trim_end().to_owned()
}

/// Formats `squeak dpi set` output for humans.
#[must_use]
pub fn format_dpi_set_human(report: &DpiWriteReport) -> String {
    let mut out = String::new();
    if report.applied {
        let _ = writeln!(
            out,
            "DPI updated (sensor {}): {} -> {} DPI (verified readback: {} DPI)",
            report.sensor_index,
            report.previous_dpi,
            report.requested_dpi,
            report.readback_dpi.unwrap_or(0)
        );
    } else {
        let _ = writeln!(
            out,
            "DRY RUN (sensor {}): validated {} -> {} DPI (no write sent; pass `--apply` to commit)",
            report.sensor_index, report.previous_dpi, report.requested_dpi
        );
    }
    let _ = writeln!(
        out,
        "Wire frame: {} idx={} fn={} params=[{}]",
        report.planned_frame.feature_id,
        report.planned_frame.feature_index,
        report.planned_frame.function_id,
        report.planned_frame.params_hex
    );
    if let Some(note) = &report.note {
        let _ = writeln!(out, "Note: {note}");
    }
    out.trim_end().to_owned()
}

/// Formats `squeak report-rate get` output for humans.
#[must_use]
pub fn format_report_rate_get_human(report: &ReportRateConfigReport) -> String {
    let rw = if report.write_supported {
        "read/write"
    } else {
        "read-only (unsupported version)"
    };
    format!(
        "Report rate ({} v{}, {rw}): current {} Hz ({} us); supported: {}",
        report.source_feature,
        report.feature_version,
        report.current_hz,
        report.current_interval_us,
        join_u16(&report.supported_hz, " Hz")
    )
}

/// Formats `squeak report-rate set` output for humans.
#[must_use]
pub fn format_report_rate_set_human(report: &ReportRateWriteReport) -> String {
    let mut out = String::new();
    if report.applied {
        let _ = writeln!(
            out,
            "Report rate updated ({}): {} Hz -> {} Hz (verified readback: {} Hz)",
            report.source_feature,
            report.previous_hz,
            report.requested_hz,
            report.readback_hz.unwrap_or(0)
        );
    } else {
        let _ = writeln!(
            out,
            "DRY RUN ({}): validated {} Hz -> {} Hz (no write sent; pass `--apply` to commit)",
            report.source_feature, report.previous_hz, report.requested_hz
        );
    }
    let _ = writeln!(
        out,
        "Wire frame: {} idx={} fn={} params=[{}]",
        report.planned_frame.feature_id,
        report.planned_frame.feature_index,
        report.planned_frame.function_id,
        report.planned_frame.params_hex
    );
    if let Some(note) = &report.note {
        let _ = writeln!(out, "Note: {note}");
    }
    out.trim_end().to_owned()
}

fn read_device_name_optional<T: HidTransport>(
    session: &mut HidppSession<T>,
    device_index: DeviceIndex,
    table: &FeatureTable,
) -> Result<Option<DeviceNameReport>, hidpp::HidppError> {
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
) -> Result<Option<FirmwareSummaryReport>, hidpp::HidppError> {
    let Some(entry) = table.get(FeatureId::FIRMWARE_INFO) else {
        return Ok(None);
    };
    let driver = FirmwareInfoDriver::from_entry(entry)?;
    let inv = driver.read_inventory(session, device_index)?;
    Ok(Some(FirmwareSummaryReport::from_inventory(&inv)))
}

fn read_dpi_optional<T: HidTransport>(
    session: &mut HidppSession<T>,
    device_index: DeviceIndex,
    table: &FeatureTable,
    sensor_filter: Option<u8>,
) -> Result<Option<DpiConfigReport>, hidpp::HidppError> {
    let Some(entry) = table.get(FeatureId::ADJUSTABLE_DPI) else {
        return Ok(None);
    };
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

fn read_report_rate_optional<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<ReportRateConfigReport>, hidpp::HidppError> {
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

fn onboard_profile_note(table: &FeatureTable) -> Option<String> {
    table.get(FeatureId::ONBOARD_PROFILES).map(|_| {
        "Device exposes ONBOARD_PROFILES (0x8100); if onboard memory mode is active, volatile host changes may be overridden by the active onboard profile.".to_owned()
    })
}

fn describe_dpi_caps(caps: &hidpp::DpiCapabilities) -> String {
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

fn describe_caps_report(caps: &squeak_core::DpiCapabilitiesReport) -> String {
    let mut parts = Vec::new();
    if !caps.discrete.is_empty() {
        parts.push(format!("discrete: {}", join_u16(&caps.discrete, "")));
    }
    for r in &caps.ranges {
        parts.push(format!("{}..={} step {}", r.min, r.max, r.step));
    }
    if parts.is_empty() {
        "none".to_owned()
    } else {
        parts.join(", ")
    }
}

fn join_u16(values: &[u16], suffix: &str) -> String {
    values
        .iter()
        .map(|v| format!("{v}{suffix}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use hidpp::{
        FeatureIndex, FunctionId, HidDeviceInfo, HidppFrame, MockTransport, ReportId, SwId,
    };

    use super::*;

    fn fn_id(id: u8) -> FunctionId {
        FunctionId::new(id).unwrap()
    }

    fn push_discovery_g502_m3(mock: &mut MockTransport, features: &[(u16, u8)]) {
        // ping -> HID++ 4.2
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
        // getFeature(0x0001) -> idx 1, v0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00, 0x00],
            )
            .unwrap(),
        );
        // getCount() -> N
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
                    &[hi, lo, 0x00, ver],
                )
                .unwrap(),
            );
        }
    }

    #[test]
    fn dry_run_dpi_set_sends_no_write_frames_and_apply_verifies_readback() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        // Features: 1=0x0001 v0, 2=0x2201 v1, 3=0x8100 v0
        let features = &[(0x0001, 0), (0x2201, 1), (0x8100, 0)];

        // 1. Dry-run test: zero fn3 write frames allowed!
        let mut dry_mock = MockTransport::new(info.clone());
        push_discovery_g502_m3(&mut dry_mock, features);
        // 0x2201 fn0 getSensorCount -> 1
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        // 0x2201 fn1 getSensorDpiList(0) -> range 100..25600 step 50
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        // 0x2201 fn2 getSensorDpi(0) -> current 800, default 800
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, 0x03, 0x20, 0x03, 0x20],
            )
            .unwrap(),
        );

        let dry_report =
            dpi_set_from_transport(dry_mock, DeviceIndex::DIRECT, 0, 1600, false).unwrap();
        assert!(!dry_report.applied);
        assert!(!dry_report.verified);
        assert_eq!(dry_report.previous_dpi, 800);
        assert_eq!(dry_report.requested_dpi, 1600);
        assert_eq!(dry_report.readback_dpi, None);
        assert_eq!(dry_report.planned_frame.params_hex, "00 06 40");
        assert!(dry_report.note.as_deref().unwrap().contains("0x8100"));

        // 2. Apply test: sends fn3 write + fn2 readback
        let mut apply_mock = MockTransport::new(info);
        push_discovery_g502_m3(&mut apply_mock, features);
        // initial count, list, get
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, 0x03, 0x20, 0x03, 0x20],
            )
            .unwrap(),
        );
        // set_sensor_dpi_verified internal checks: count, list, set echo, readback
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(3),
                SwId::DEFAULT_REQUEST,
                &[0, 0x06, 0x40],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, 0x06, 0x40, 0x03, 0x20],
            )
            .unwrap(),
        );

        let apply_report =
            dpi_set_from_transport(apply_mock, DeviceIndex::DIRECT, 0, 1600, true).unwrap();
        assert!(apply_report.applied);
        assert!(apply_report.verified);
        assert_eq!(apply_report.previous_dpi, 800);
        assert_eq!(apply_report.readback_dpi, Some(1600));
    }

    #[test]
    fn report_rate_dry_run_and_apply_with_validation() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let features = &[(0x0001, 0), (0x8060, 0)];

        // Unsupported Hz rejected by parser before touching transport
        let empty_mock = MockTransport::new(info.clone());
        let bad_hz = report_rate_set_from_transport(empty_mock, DeviceIndex::DIRECT, 333, false);
        assert!(matches!(bad_hz, Err(CliError::Usage(_))));

        // Dry run validates against supported mask without writing
        let mut dry_mock = MockTransport::new(info.clone());
        push_discovery_g502_m3(&mut dry_mock, features);
        // read_report_rate -> list (0x8B = 1,2,4,8 ms), current (1 ms = 1000 Hz)
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x8B],
            )
            .unwrap(),
        );
        dry_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        let dry_rr =
            report_rate_set_from_transport(dry_mock, DeviceIndex::DIRECT, 500, false).unwrap();
        assert!(!dry_rr.applied);
        assert_eq!(dry_rr.previous_hz, 1000);
        assert_eq!(dry_rr.requested_hz, 500);
        assert_eq!(dry_rr.planned_frame.params_hex, "02");

        // Unsupported 2000 Hz on 0x8060 fails cleanly
        let mut unsupported_mock = MockTransport::new(info.clone());
        push_discovery_g502_m3(&mut unsupported_mock, features);
        let res_2k =
            report_rate_set_from_transport(unsupported_mock, DeviceIndex::DIRECT, 2000, true);
        assert!(matches!(res_2k, Err(CliError::Hidpp(_))));

        // Apply 500 Hz + verified readback
        let mut apply_mock = MockTransport::new(info);
        push_discovery_g502_m3(&mut apply_mock, features);
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x8B],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        // set_report_rate_verified internal list + fn2 write + fn1 readback
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x8B],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[],
            )
            .unwrap(),
        );
        apply_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[2],
            )
            .unwrap(),
        );
        let applied_rr =
            report_rate_set_from_transport(apply_mock, DeviceIndex::DIRECT, 500, true).unwrap();
        assert!(applied_rr.applied);
        assert!(applied_rr.verified);
        assert_eq!(applied_rr.readback_hz, Some(500));
    }

    #[test]
    fn status_reads_all_present_m3_features_without_failing_on_missing_battery() {
        let db = DeviceDatabase::builtin().unwrap();
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        // Features: 1=0x0001, 2=0x0003 v2, 3=0x0005 v0, 4=0x2201 v1, 5=0x8060 v0
        let features = &[
            (0x0001, 0),
            (0x0003, 2),
            (0x0005, 0),
            (0x2201, 1),
            (0x8060, 0),
        ];
        let mut mock = MockTransport::new(info);
        push_discovery_g502_m3(&mut mock, features);

        // 0x0005 name: count=9, chunk="G502 HERO", type=0x03 (Mouse)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[9],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                b"G502 HERO",
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(3),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0x03],
            )
            .unwrap(),
        );

        // 0x0003 firmware: entityCount=1, USB, PID C08B -> entity 0 MainApp MPM 12.03.B0019
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[
                    1, 0xAA, 0xBB, 0xCC, 0xDD, 0x08, 0xC0, 0x8B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
                ],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[
                    0x00, b'M', b'P', b'M', 0x12, 0x03, 0x00, 0x19, 0x01, 0xC0, 0x8B,
                ],
            )
            .unwrap(),
        );

        // 0x2201 DPI: count=1, range 100..25600 step 50, current=800 default=800
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(4),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(4),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(4),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, 0x03, 0x20, 0x03, 0x20],
            )
            .unwrap(),
        );

        // 0x8060 report rate: mask 0x8B, current 1 ms
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(5),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x8B],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(5),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );

        let report = status_from_transport(mock, DeviceIndex::DIRECT, &db).unwrap();
        assert_eq!(report.device_name.as_ref().unwrap().name, "G502 HERO");
        assert_eq!(
            report.firmware.as_ref().unwrap().entities[0].version,
            "MPM 12.03.B0019"
        );
        assert!(report.battery.is_none());
        assert_eq!(report.dpi.as_ref().unwrap().sensors[0].current_dpi, 800);
        assert_eq!(report.report_rate.as_ref().unwrap().current_hz, 1000);
        assert!(report.notices.is_empty());
    }
}

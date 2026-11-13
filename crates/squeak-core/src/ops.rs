//! Shared M3 HID++ session operations reused by `squeak-cli` and `squeakd`.
//!
//! Keeps capability validation, `--dry-run` planning, and post-write readback verification in
//! one single place so CLI direct mode and daemon DBus handlers behave identically.

use hidpp::{
    AdjustableDpiDriver, AdjustableDpiFeature, DeviceIndex, DeviceNameDriver, DeviceNameFeature,
    Dpi, DpiCapabilities, ExtendedReportRateDriver, FeatureId, FeatureTable, FirmwareInfoDriver,
    FirmwareInfoFeature, HidDeviceInfo, HidTransport, HidppError, HidppFeature, HidppSession,
    ProtocolError, ReportRate, ReportRateDriver, read_battery, read_report_rate,
    set_report_rate_verified,
};

use crate::device_data::DeviceDatabase;
use crate::dump::DeviceDump;
use crate::settings::{
    BatteryStatusReport, DeviceNameReport, DeviceStatusReport, DpiConfigReport, DpiWriteReport,
    FirmwareSummaryReport, PlannedWireFrame, ReportRateConfigReport, ReportRateWriteReport,
    SensorDpiReport,
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

    DeviceStatusReport {
        path: info.path.display().to_string(),
        device: dump.device,
        protocol_version: dump.protocol_version,
        device_name,
        firmware,
        battery,
        dpi,
        report_rate,
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

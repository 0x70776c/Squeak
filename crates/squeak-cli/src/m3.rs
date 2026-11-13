//! M3 read/write subcommands (`status`, `dpi`, `report-rate`) and `--dry-run` safety guards.

use std::fmt::Write as _;

use hidpp::{DeviceIndex, FeatureId, FeatureTable, HidTransport, HidppSession, ProtocolError};
use squeak_core::{
    DeviceDatabase, DeviceStatusReport, DpiCapabilitiesReport, DpiConfigReport, DpiWriteReport,
    ReportRateConfigReport, ReportRateWriteReport, execute_dpi_write, execute_report_rate_write,
    read_device_status, read_dpi_config, read_report_rate_config,
};

use crate::commands::CliError;

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
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    let info = session.device_info().clone();
    Ok(read_device_status(
        &mut session,
        &info,
        &table,
        db,
        env!("CARGO_PKG_VERSION"),
    ))
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
    Ok(read_dpi_config(&mut session, &table, sensor_filter)?.ok_or(
        ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ADJUSTABLE_DPI,
        },
    )?)
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
    Ok(execute_dpi_write(
        &mut session,
        &table,
        sensor_index,
        target_dpi,
        apply,
    )?)
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
    Ok(read_report_rate_config(&mut session, &table)?.ok_or(
        ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::REPORT_RATE,
        },
    )?)
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
    if let Err(err) = squeak_core::parse_report_rate_hz(hz) {
        return Err(CliError::Usage(err.to_string()));
    }
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    Ok(execute_report_rate_write(&mut session, &table, hz, apply)?)
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

fn describe_caps_report(caps: &DpiCapabilitiesReport) -> String {
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

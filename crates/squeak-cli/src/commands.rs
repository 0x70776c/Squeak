//! Subcommand implementations (`list`, `info`, `dump-device`, `setup-udev`) and formatters.

use std::fmt::Write as _;
use std::path::Path;

use hidpp::{
    DeviceIndex, FeatureTable, HidDeviceInfo, HidTransport, HidppError, HidppSession,
    HidrawTransport, IRoot, ProtocolError, TransportError, discover_hidpp_devices,
};
use squeak_core::{
    DefinitionMatchKind, DeviceDataError, DeviceDatabase, DeviceDump, DeviceInfoReport,
    DeviceListEntry, ProbeStatus,
};
use thiserror::Error;

use crate::cli::{
    ButtonsCommand, Cli, Command, DpiCommand, HostModeCommand, LedCommand, ListArgs,
    ReportRateCommand, TargetArgs,
};
use crate::m3::{
    dpi_get_from_transport, dpi_set_from_transport, format_dpi_get_human, format_dpi_set_human,
    format_report_rate_get_human, format_report_rate_set_human, format_status_human,
    report_rate_get_from_transport, report_rate_set_from_transport, status_from_transport,
};
use crate::m5::{
    buttons_list_from_transport, format_buttons_list_human, format_host_mode_get_human,
    format_host_mode_set_human, format_led_get_human, format_led_set_human,
    host_mode_get_from_transport, host_mode_set_from_transport, led_get_from_transport,
    led_set_from_transport, run_buttons_spy,
};
use crate::udev::{UdevSetupError, format_setup_udev_human, run_setup_udev};

const LIST_PING_DATA: u8 = 0x53;

/// Top-level CLI execution errors.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CliError {
    /// Invalid CLI flag or selector value.
    #[error("{0}")]
    Usage(String),

    /// Built-in device TOML database failed validation.
    #[error("failed loading device database: {0}")]
    DeviceDatabase(#[from] DeviceDataError),

    /// Transport-level failure discovering or opening `hidraw`.
    #[error("{0}")]
    Transport(#[from] TransportError),

    /// HID++ 2.0 protocol error during probing or discovery.
    #[error("{0}")]
    Hidpp(#[from] HidppError),

    /// Error installing or verifying udev rules.
    #[error("{0}")]
    Udev(#[from] UdevSetupError),

    /// JSON serialization error.
    #[error("failed serializing JSON output: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<ProtocolError> for CliError {
    fn from(err: ProtocolError) -> Self {
        Self::Hidpp(HidppError::Protocol(err))
    }
}

/// Runs the parsed CLI command and prints output to stdout cleanly.
///
/// # Errors
///
/// Returns [`CliError`] if discovery, device communication, or serialization fails.
pub fn execute(cli: Cli) -> Result<String, CliError> {
    let db = DeviceDatabase::builtin()?;

    match cli.command {
        Command::List(args) => run_list(&args, &db, cli.json),
        Command::Info(args) => run_info(&args, &db, cli.json),
        Command::Status(args) => run_status(&args, &db, cli.json),
        Command::Dpi(args) => match args.action {
            DpiCommand::Get(get_args) => {
                let (device_index, transport) = open_selected_transport(&get_args.target)?;
                let report = dpi_get_from_transport(transport, device_index, get_args.sensor)?;
                format_output(&report, cli.json, format_dpi_get_human)
            }
            DpiCommand::Set(set_args) => {
                let (device_index, transport) = open_selected_transport(&set_args.target)?;
                let report = dpi_set_from_transport(
                    transport,
                    device_index,
                    set_args.sensor,
                    set_args.dpi,
                    set_args.apply,
                )?;
                format_output(&report, cli.json, format_dpi_set_human)
            }
        },
        Command::ReportRate(args) => match args.action {
            ReportRateCommand::Get(target) => {
                let (device_index, transport) = open_selected_transport(&target)?;
                let report = report_rate_get_from_transport(transport, device_index)?;
                format_output(&report, cli.json, format_report_rate_get_human)
            }
            ReportRateCommand::Set(set_args) => {
                let (device_index, transport) = open_selected_transport(&set_args.target)?;
                let report = report_rate_set_from_transport(
                    transport,
                    device_index,
                    set_args.hz,
                    set_args.apply,
                )?;
                format_output(&report, cli.json, format_report_rate_set_human)
            }
        },
        Command::Led(args) => match args.action {
            LedCommand::Get(target) => {
                let (device_index, transport) = open_selected_transport(&target)?;
                let report = led_get_from_transport(transport, device_index)?;
                format_output(&report, cli.json, format_led_get_human)
            }
            LedCommand::Set(set_args) => {
                let (device_index, transport) = open_selected_transport(&set_args.target)?;
                let report = led_set_from_transport(transport, device_index, &set_args)?;
                format_output(&report, cli.json, format_led_set_human)
            }
        },
        Command::HostMode(args) => match args.action {
            HostModeCommand::Get(target) => {
                let (device_index, transport) = open_selected_transport(&target)?;
                let report = host_mode_get_from_transport(transport, device_index)?;
                format_output(&report, cli.json, format_host_mode_get_human)
            }
            HostModeCommand::Set(set_args) => {
                let (device_index, transport) = open_selected_transport(&set_args.target)?;
                let report = host_mode_set_from_transport(transport, device_index, &set_args)?;
                format_output(&report, cli.json, format_host_mode_set_human)
            }
        },
        Command::Buttons(args) => match args.action {
            ButtonsCommand::List(target) => {
                let (device_index, transport) = open_selected_transport(&target)?;
                let report = buttons_list_from_transport(transport, device_index, &db)?;
                format_output(&report, cli.json, format_buttons_list_human)
            }
            ButtonsCommand::Spy(spy_args) => {
                let (device_index, transport) = open_selected_transport(&spy_args.target)?;
                run_buttons_spy(transport, device_index, spy_args.count, cli.json)
            }
        },
        Command::DumpDevice(args) => run_dump_device(&args, &db, cli.json),
        Command::SetupUdev(args) => {
            let report = run_setup_udev(args.dry_run)?;
            format_output(&report, cli.json, format_setup_udev_human)
        }
    }
}

fn format_output<T: serde::Serialize>(
    value: &T,
    as_json: bool,
    human: fn(&T) -> String,
) -> Result<String, CliError> {
    if as_json {
        Ok(serde_json::to_string_pretty(value)?)
    } else {
        Ok(human(value))
    }
}

fn open_selected_transport(args: &TargetArgs) -> Result<(DeviceIndex, HidrawTransport), CliError> {
    let device_index = parse_device_index(&args.device_index)?;
    let discovered = discover_hidpp_devices()?;
    let target = select_device(&discovered, args.device.as_deref())?.clone();
    let transport = HidrawTransport::open(target)?;
    Ok((device_index, transport))
}

fn run_list(args: &ListArgs, db: &DeviceDatabase, as_json: bool) -> Result<String, CliError> {
    let device_index = parse_device_index(&args.device_index)?;
    let discovered = discover_hidpp_devices()?;

    let mut entries = Vec::with_capacity(discovered.len());
    for info in discovered {
        let opened = HidrawTransport::open(info.clone());
        entries.push(probe_entry_from_transport(&info, opened, device_index, db));
    }

    if as_json {
        Ok(serde_json::to_string_pretty(&entries)?)
    } else {
        Ok(format_list_human(&entries))
    }
}

fn run_info(args: &TargetArgs, db: &DeviceDatabase, as_json: bool) -> Result<String, CliError> {
    let (device_index, transport) = open_selected_transport(args)?;
    let (info_report, _dump) = discover_from_transport(transport, device_index, db)?;
    format_output(&info_report, as_json, format_info_human)
}

fn run_status(args: &TargetArgs, db: &DeviceDatabase, as_json: bool) -> Result<String, CliError> {
    let (device_index, transport) = open_selected_transport(args)?;
    let report = status_from_transport(transport, device_index, db)?;
    format_output(&report, as_json, format_status_human)
}

fn run_dump_device(
    args: &TargetArgs,
    db: &DeviceDatabase,
    as_json: bool,
) -> Result<String, CliError> {
    let (device_index, transport) = open_selected_transport(args)?;
    let (_info_report, dump) = discover_from_transport(transport, device_index, db)?;
    format_output(&dump, as_json, format_dump_human)
}

/// Parses `--device-index` accepting `"direct"`, `"0xff"`, `"255"`, or receiver slots `"1"..="6"`.
///
/// # Errors
///
/// Returns [`CliError::Usage`] on malformed values or out-of-range indices.
pub fn parse_device_index(spec: &str) -> Result<DeviceIndex, CliError> {
    let trimmed = spec.trim();
    if trimmed.eq_ignore_ascii_case("direct") {
        return Ok(DeviceIndex::DIRECT);
    }

    let parsed: u8 = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u8::from_str_radix(hex, 16)
            .map_err(|_| CliError::Usage(format!("invalid hex device index {spec:?}")))?
    } else {
        trimmed
            .parse::<u8>()
            .map_err(|_| CliError::Usage(format!("invalid device index {spec:?}")))?
    };

    if parsed == 0xFF {
        Ok(DeviceIndex::DIRECT)
    } else if (1..=6).contains(&parsed) {
        DeviceIndex::receiver(parsed).map_err(|err| CliError::Usage(err.to_string()))
    } else {
        Err(CliError::Usage(format!(
            "unsupported device index {spec:?}: expected \"direct\" (0xFF) or receiver slot 1..=6"
        )))
    }
}

/// Resolves `--device` selector against discovered HID++ devices.
///
/// # Errors
///
/// Returns [`CliError::Usage`] when no device is present, the selector matches nothing, or
/// multiple devices match and disambiguation via `--device` is required.
pub fn select_device<'a>(
    devices: &'a [HidDeviceInfo],
    selector: Option<&str>,
) -> Result<&'a HidDeviceInfo, CliError> {
    if devices.is_empty() {
        return Err(CliError::Usage(
            "no HID++ 2.0 devices found under /sys/class/hidraw; check cable or receiver"
                .to_owned(),
        ));
    }

    let Some(raw_sel) = selector.map(str::trim).filter(|s| !s.is_empty()) else {
        if devices.len() == 1 {
            return Ok(&devices[0]);
        }
        let available = format_available_list(devices);
        return Err(CliError::Usage(format!(
            "multiple HID++ endpoints found; pass `--device <selector>`:\n{available}"
        )));
    };

    let mut matches: Vec<&HidDeviceInfo> = Vec::new();
    if let Some((vid, pid)) = parse_vid_pid(raw_sel) {
        for dev in devices {
            if dev.vendor_id == vid && dev.product_id == pid {
                matches.push(dev);
            }
        }
    } else {
        for dev in devices {
            let basename = dev.path.file_name().and_then(|n| n.to_str());
            if dev.path == Path::new(raw_sel) || basename == Some(raw_sel) {
                matches.push(dev);
            }
        }
    }

    match matches.len() {
        0 => {
            let available = format_available_list(devices);
            Err(CliError::Usage(format!(
                "no HID++ device matching {raw_sel:?}. Connected endpoints:\n{available}"
            )))
        }
        1 => Ok(matches[0]),
        _ => {
            let candidates = matches
                .iter()
                .map(|d| {
                    format!(
                        "  - {} ({:04x}:{:04x})",
                        d.path.display(),
                        d.vendor_id,
                        d.product_id
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Err(CliError::Usage(format!(
                "selector {raw_sel:?} matched multiple endpoints; pass an exact node path:\n{candidates}"
            )))
        }
    }
}

fn parse_vid_pid(spec: &str) -> Option<(u16, u16)> {
    let (v, p) = spec.split_once(':')?;
    let parse_part = |part: &str| -> Option<u16> {
        let clean = part
            .trim()
            .strip_prefix("0x")
            .or_else(|| part.trim().strip_prefix("0X"))
            .unwrap_or(part.trim());
        u16::from_str_radix(clean, 16).ok()
    };
    Some((parse_part(v)?, parse_part(p)?))
}

fn format_available_list(devices: &[HidDeviceInfo]) -> String {
    devices
        .iter()
        .map(|d| {
            let name = d.name.as_deref().unwrap_or("Unknown");
            format!(
                "  - {} ({:04x}:{:04x}, {}, {})",
                d.path.display(),
                d.vendor_id,
                d.product_id,
                d.bus,
                name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Probes a single discovered HID++ endpoint via `IRoot::ping` and builds a [`DeviceListEntry`].
#[must_use]
pub fn probe_entry_from_transport<T: HidTransport>(
    info: &HidDeviceInfo,
    transport_res: Result<T, TransportError>,
    device_index: DeviceIndex,
    db: &DeviceDatabase,
) -> DeviceListEntry {
    let resolved = db.resolve(info.vendor_id, info.product_id);
    let definition_match = if resolved.is_known() {
        DefinitionMatchKind::Known
    } else {
        DefinitionMatchKind::Generic
    };
    let matched_name = resolved.display_name(info.name.as_deref());

    let (protocol_version, probe_status, hint) = match transport_res {
        Ok(transport) => {
            let mut session = HidppSession::new(transport);
            match IRoot::ping(&mut session, device_index, LIST_PING_DATA) {
                Ok(ver) => (Some(ver.to_string()), ProbeStatus::Accessible, None),
                Err(HidppError::Protocol(ProtocolError::LegacyDeviceNotSupported { .. })) => (
                    None,
                    ProbeStatus::LegacyHidpp10,
                    Some("Legacy HID++ 1.0 hardware is out of scope.".to_owned()),
                ),
                Err(HidppError::Transport(TransportError::Timeout))
                | Err(HidppError::Protocol(ProtocolError::ReceiverDeviceOffline { .. })) => (
                    None,
                    ProbeStatus::TimeoutOrAsleep,
                    Some(
                        "Timed out or receiver slot offline; wake wireless device or try `--device-index 1..=6` on a receiver."
                            .to_owned(),
                    ),
                ),
                Err(err) => (
                    None,
                    ProbeStatus::Error {
                        message: err.to_string(),
                    },
                    None,
                ),
            }
        }
        Err(TransportError::PermissionDenied { .. }) => (
            None,
            ProbeStatus::PermissionDenied,
            Some(
                "Permission denied; run `squeak setup-udev` to install the uaccess rule."
                    .to_owned(),
            ),
        ),
        Err(err) => (
            None,
            ProbeStatus::Error {
                message: err.to_string(),
            },
            None,
        ),
    };

    DeviceListEntry {
        path: info.path.display().to_string(),
        bus: info.bus.to_string(),
        vendor_id: info.vendor_id,
        product_id: info.product_id,
        hid_name: info.name.clone(),
        definition_match,
        matched_name,
        protocol_version,
        probe_status,
        hint,
    }
}

/// Discovers the runtime [`FeatureTable`] over `transport` and produces sanitized [`DeviceInfoReport`]
/// and [`DeviceDump`] outputs.
///
/// # Errors
///
/// Returns [`CliError`] if `IRoot` / `IFeatureSet` discovery fails.
pub fn discover_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    db: &DeviceDatabase,
) -> Result<(DeviceInfoReport, DeviceDump), CliError> {
    let path = transport.device_info().path.display().to_string();
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    let info = session.device_info();
    let resolved = db.resolve(info.vendor_id, info.product_id);
    let dump = DeviceDump::from_discovery(env!("CARGO_PKG_VERSION"), info, &table, resolved);
    let summary = DeviceInfoReport::from_dump(path, &dump, &table);
    Ok((summary, dump))
}

/// Formats `squeak list` rows into an aligned human table.
#[must_use]
pub fn format_list_human(entries: &[DeviceListEntry]) -> String {
    if entries.is_empty() {
        return "No HID++ endpoints detected under /sys/class/hidraw.".to_owned();
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<16} {:<11} {:<10} {:<8} {:<24} STATUS",
        "NODE", "VID:PID", "BUS", "PROTO", "DEVICE"
    );
    for entry in entries {
        let mode_tag = match entry.definition_match {
            DefinitionMatchKind::Known => "",
            DefinitionMatchKind::Generic => " [generic]",
        };
        let proto = entry.protocol_version.as_deref().unwrap_or("-");
        let status = match &entry.probe_status {
            ProbeStatus::Accessible => "accessible".to_owned(),
            ProbeStatus::PermissionDenied => "permission denied".to_owned(),
            ProbeStatus::LegacyHidpp10 => "legacy HID++ 1.0 (unsupported)".to_owned(),
            ProbeStatus::TimeoutOrAsleep => "timeout / asleep".to_owned(),
            ProbeStatus::Error { message } => format!("error: {message}"),
            _ => "unknown status".to_owned(),
        };
        let _ = writeln!(
            out,
            "{:<16} {:04x}:{:04x}  {:<10} {:<8} {:<24} {}",
            entry.path,
            entry.vendor_id,
            entry.product_id,
            entry.bus,
            proto,
            format!("{}{}", entry.matched_name, mode_tag),
            status
        );
        if let Some(hint) = &entry.hint {
            let _ = writeln!(out, "  hint: {hint}");
        }
    }
    out.trim_end().to_owned()
}

/// Formats `squeak info` summary for human output.
#[must_use]
pub fn format_info_human(report: &DeviceInfoReport) -> String {
    let mut out = String::new();
    let match_label = match report.device.definition_match {
        DefinitionMatchKind::Known => "known TOML definition",
        DefinitionMatchKind::Generic => "generic HID++ 2.0 fallback",
    };

    let _ = writeln!(out, "Device:           {}", report.device.matched_name);
    let _ = writeln!(
        out,
        "Definition:       {match_label} ({})",
        report.device.kind
    );
    let _ = writeln!(out, "Node:             {}", report.path);
    let _ = writeln!(
        out,
        "VID:PID / Bus:    0x{:04x}:0x{:04x} ({})",
        report.device.vendor_id, report.device.product_id, report.device.bus
    );
    let _ = writeln!(out, "Device index:     {}", report.device.device_index);
    let _ = writeln!(out, "HID++ version:    {}", report.protocol_version);
    let _ = writeln!(
        out,
        "Report IDs:       {}",
        report.device.supported_reports.join(", ")
    );
    let _ = writeln!(out, "Feature count:    {}", report.feature_count);

    if report.detected_capabilities.is_empty() {
        let _ = writeln!(out, "Capabilities:     (basic IRoot/IFeatureSet only)");
    } else {
        let _ = writeln!(out, "Capabilities:");
        for cap in &report.detected_capabilities {
            let _ = writeln!(out, "  - {cap}");
        }
    }
    out.trim_end().to_owned()
}

/// Formats `squeak dump-device` runtime feature table for human output.
#[must_use]
pub fn format_dump_human(dump: &DeviceDump) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} (0x{:04x}:0x{:04x}, {}, HID++ {}, index {}, mode: {:?})",
        dump.device.matched_name,
        dump.device.vendor_id,
        dump.device.product_id,
        dump.device.bus,
        dump.protocol_version,
        dump.device.device_index,
        dump.device.definition_match,
    );
    let _ = writeln!(
        out,
        "Supported reports: {}",
        dump.device.supported_reports.join(", ")
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<5} {:<8} {:<30} {:<5} FLAGS",
        "IDX", "ID", "FEATURE", "VER"
    );
    for feat in &dump.features {
        let mut flag_parts = Vec::new();
        if feat.flags.obsolete {
            flag_parts.push("obsolete");
        }
        if feat.flags.hidden {
            flag_parts.push("hidden");
        }
        if feat.flags.engineering {
            flag_parts.push("eng");
        }
        let flags_str = if flag_parts.is_empty() {
            "-".to_owned()
        } else {
            flag_parts.join(",")
        };
        let _ = writeln!(
            out,
            "{:<5} 0x{:04X}   {:<30} {:<5} {}",
            feat.index, feat.feature_id, feat.name, feat.version, flags_str
        );
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hidpp::{
        FeatureIndex, FunctionId, HidppFrame, MockTransport, ReportId, SupportedReports, SwId,
    };

    use super::*;

    fn fn_id(id: u8) -> FunctionId {
        FunctionId::new(id).unwrap()
    }

    fn queue_g502_discovery(mock: &mut MockTransport) {
        // ping(0x53) -> 4.2
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
        // getFeature(0x0001) -> index 1, flags 0, version 1
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
        // getCount() -> 4 features
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[4, 0, 0],
            )
            .unwrap(),
        );
        // 1: IFeatureSet (0x0001)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
        // 2: ADJUSTABLE_DPI (0x2201) v1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x22, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
        // 3: REPROG_CONTROLS_V4 (0x1B04) v3
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x1B, 0x04, 0x00, 0x03],
            )
            .unwrap(),
        );
        // 4: RGB_EFFECTS (0x8071) v0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x80, 0x71, 0x00, 0x00],
            )
            .unwrap(),
        );
    }

    #[test]
    fn parses_device_index_inputs_strictly() {
        assert_eq!(parse_device_index("direct").unwrap(), DeviceIndex::DIRECT);
        assert_eq!(parse_device_index("DIRECT").unwrap(), DeviceIndex::DIRECT);
        assert_eq!(parse_device_index("0xff").unwrap(), DeviceIndex::DIRECT);
        assert_eq!(parse_device_index("255").unwrap(), DeviceIndex::DIRECT);
        assert_eq!(
            parse_device_index("1").unwrap(),
            DeviceIndex::receiver(1).unwrap()
        );
        assert_eq!(
            parse_device_index("0x06").unwrap(),
            DeviceIndex::receiver(6).unwrap()
        );

        assert!(parse_device_index("0").is_err());
        assert!(parse_device_index("7").is_err());
        assert!(parse_device_index("abc").is_err());
    }

    #[test]
    fn selects_devices_by_auto_path_basename_and_vid_pid() {
        let mut dev1 = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        dev1.path = PathBuf::from("/dev/hidraw1");
        let mut dev2 = HidDeviceInfo::mock(0x046D, 0xB034, "Wireless Mouse");
        dev2.path = PathBuf::from("/dev/hidraw4");

        // Auto-select single device
        assert_eq!(
            select_device(std::slice::from_ref(&dev1), None)
                .unwrap()
                .path,
            dev1.path
        );
        // Require disambiguation when multiple connected
        let two = [dev1.clone(), dev2.clone()];
        assert!(select_device(&two, None).is_err());

        assert_eq!(
            select_device(&two, Some("hidraw4")).unwrap().path,
            dev2.path
        );
        assert_eq!(
            select_device(&two, Some("/dev/hidraw1")).unwrap().path,
            dev1.path
        );
        assert_eq!(
            select_device(&two, Some("046d:b034")).unwrap().path,
            dev2.path
        );
        assert!(select_device(&[], None).is_err());
        assert!(select_device(&two, Some("046d:9999")).is_err());
    }

    #[test]
    fn list_probe_covers_accessible_permission_denied_and_legacy10() {
        let db = DeviceDatabase::builtin().unwrap();

        let mut g502_info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        g502_info.path = PathBuf::from("/dev/hidraw1");
        let mut ok_mock = MockTransport::new(g502_info.clone());
        ok_mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x02, LIST_PING_DATA],
            )
            .unwrap(),
        );
        let ok_entry =
            probe_entry_from_transport(&g502_info, Ok(ok_mock), DeviceIndex::DIRECT, &db);
        assert_eq!(ok_entry.probe_status, ProbeStatus::Accessible);
        assert_eq!(ok_entry.protocol_version.as_deref(), Some("4.2"));
        assert_eq!(ok_entry.definition_match, DefinitionMatchKind::Known);

        // Permission denied case
        let denied_entry = probe_entry_from_transport::<MockTransport>(
            &g502_info,
            Err(TransportError::PermissionDenied {
                path: g502_info.path.clone(),
            }),
            DeviceIndex::DIRECT,
            &db,
        );
        assert_eq!(denied_entry.probe_status, ProbeStatus::PermissionDenied);
        assert!(denied_entry.hint.as_deref().unwrap().contains("setup-udev"));

        // Legacy 1.0 receiver case
        let mut legacy_info = HidDeviceInfo::mock(0x046D, 0xC52B, "Unifying Receiver");
        legacy_info.path = PathBuf::from("/dev/hidraw2");
        let mut legacy_mock = MockTransport::new(legacy_info.clone());
        legacy_mock
            .push_frame(HidppFrame::decode(&[0x10, 0xFF, 0x8F, 0x00, 0x1A, 0x01, 0x00]).unwrap());
        let legacy_entry =
            probe_entry_from_transport(&legacy_info, Ok(legacy_mock), DeviceIndex::DIRECT, &db);
        assert_eq!(legacy_entry.probe_status, ProbeStatus::LegacyHidpp10);
        assert_eq!(legacy_entry.definition_match, DefinitionMatchKind::Generic);

        let table = format_list_human(&[ok_entry, denied_entry, legacy_entry]);
        assert!(table.contains("G502 HERO"));
        assert!(table.contains("permission denied"));
        assert!(table.contains("legacy HID++ 1.0"));
    }

    #[test]
    fn info_and_dump_device_work_for_known_and_generic_devices() {
        let db = DeviceDatabase::builtin().unwrap();

        // Known G502 HERO
        let mut known_info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO Gaming Mouse");
        known_info.supported_reports = SupportedReports::SHORT_AND_LONG;
        let mut mock = MockTransport::new(known_info);
        queue_g502_discovery(&mut mock);

        let (summary, dump) = discover_from_transport(mock, DeviceIndex::DIRECT, &db).unwrap();
        assert_eq!(dump.device.definition_match, DefinitionMatchKind::Known);
        assert_eq!(dump.device.matched_name, "G502 HERO");
        assert_eq!(dump.features.len(), 5);
        assert!(
            summary
                .detected_capabilities
                .iter()
                .any(|c| c.contains("Adjustable DPI"))
        );

        let json = serde_json::to_string_pretty(&dump).unwrap();
        assert!(json.contains("\"feature_id\": \"0x2201\""));
        assert!(!json.to_lowercase().contains("serial"));
        assert!(format_dump_human(&dump).contains("ADJUSTABLE_DPI"));
        assert!(format_info_human(&summary).contains("known TOML definition"));

        // Unknown device -> Generic HID++ 2.0 mode
        let generic_info = HidDeviceInfo::mock(0x046D, 0xDEAD, "Experimental HID++ Trackball");
        let mut generic_mock = MockTransport::new(generic_info);
        queue_g502_discovery(&mut generic_mock);
        let (generic_summary, generic_dump) =
            discover_from_transport(generic_mock, DeviceIndex::DIRECT, &db).unwrap();
        assert_eq!(
            generic_dump.device.definition_match,
            DefinitionMatchKind::Generic
        );
        assert_eq!(
            generic_dump.device.matched_name,
            "Experimental HID++ Trackball"
        );
        assert!(format_info_human(&generic_summary).contains("generic HID++ 2.0 fallback"));
    }

    #[test]
    fn handles_ambiguous_vid_pid_timeout_probe_and_receiver_slot() {
        let mut dev_a = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        dev_a.path = PathBuf::from("/dev/hidraw1");
        let mut dev_b = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        dev_b.path = PathBuf::from("/dev/hidraw3");
        let pair = [dev_a.clone(), dev_b];

        // Ambiguous VID:PID requires explicit hidraw path
        assert!(select_device(&pair, Some("046d:c08b")).is_err());
        assert_eq!(
            select_device(&pair, Some("/dev/hidraw1")).unwrap().path,
            dev_a.path
        );

        // Empty MockTransport triggers TimeoutOrAsleep probe status
        let db = DeviceDatabase::builtin().unwrap();
        let receiver_info = HidDeviceInfo::mock(0x046D, 0xC548, "Bolt Receiver");
        let asleep_mock = MockTransport::new(receiver_info.clone());
        let slot2 = DeviceIndex::receiver(2).unwrap();
        let entry = probe_entry_from_transport(&receiver_info, Ok(asleep_mock), slot2, &db);
        assert_eq!(entry.probe_status, ProbeStatus::TimeoutOrAsleep);
        assert!(entry.hint.as_deref().unwrap().contains("receiver"));

        // Receiver 0x8F ResourceError on an offline paired slot also maps to TimeoutOrAsleep
        let mut offline_mock = MockTransport::new(receiver_info.clone());
        offline_mock
            .push_frame(HidppFrame::decode(&[0x10, 0x02, 0x8F, 0x00, 0x1A, 0x09, 0x00]).unwrap());
        let offline_entry =
            probe_entry_from_transport(&receiver_info, Ok(offline_mock), slot2, &db);
        assert_eq!(offline_entry.probe_status, ProbeStatus::TimeoutOrAsleep);
        assert!(offline_entry.hint.as_deref().unwrap().contains("offline"));

        // Receiver-slot discovery records non-direct device_index accurately
        let mut slot_mock = MockTransport::new(receiver_info);
        slot_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                slot2,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x05, 0x53],
            )
            .unwrap(),
        );
        slot_mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                slot2,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        let (_summary, dump) = discover_from_transport(slot_mock, slot2, &db).unwrap();
        assert_eq!(dump.device.device_index, "0x02");
        assert_eq!(dump.protocol_version, "4.5");
    }
}

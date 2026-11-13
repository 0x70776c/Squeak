//! M6 onboard profile memory management (`profiles`) CLI commands.

use std::fmt::Write as _;
use std::path::Path;

use hidpp::{DeviceIndex, FeatureId, FeatureTable, HidTransport, HidppSession, ProtocolError};
use squeak_core::{
    ProfileBackupReport, ProfileRestoreReport, ProfileSetActiveReport, ProfilesListReport,
    execute_profile_backup, execute_profile_restore, execute_set_active_profile,
    read_profiles_summary,
};

use crate::commands::CliError;

/// Reads onboard profiles summary from `ONBOARD_PROFILES` (`0x8100`).
///
/// # Errors
///
/// Returns [`CliError`] if discovery fails or `0x8100` is missing.
pub fn profiles_list_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
) -> Result<ProfilesListReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    if !table.contains(FeatureId::ONBOARD_PROFILES) {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    }
    Ok(read_profiles_summary(&mut session, &table)?)
}

/// Backs up all onboard profile memory sectors to a verified JSON backup file.
///
/// # Errors
///
/// Returns [`CliError`] if discovery or reading fails.
pub fn profiles_backup_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    out: Option<&Path>,
) -> Result<ProfileBackupReport, CliError> {
    let info = transport.device_info().clone();
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    if !table.contains(FeatureId::ONBOARD_PROFILES) {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    }
    Ok(execute_profile_backup(&mut session, &info, &table, out)?)
}

/// Restores onboard profile memory sectors from a verified backup file.
///
/// # Errors
///
/// Returns [`CliError`] if reading, flashing, or verification fails.
pub fn profiles_restore_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    file: &Path,
    apply: bool,
) -> Result<ProfileRestoreReport, CliError> {
    let info = transport.device_info().clone();
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    if !table.contains(FeatureId::ONBOARD_PROFILES) {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    }
    Ok(execute_profile_restore(
        &mut session,
        &info,
        &table,
        file,
        apply,
    )?)
}

/// Switches the active onboard profile slot.
///
/// # Errors
///
/// Returns [`CliError`] if slot is invalid or communication fails.
pub fn profiles_set_active_from_transport<T: HidTransport>(
    transport: T,
    device_index: DeviceIndex,
    slot: u16,
    apply: bool,
) -> Result<ProfileSetActiveReport, CliError> {
    let mut session = HidppSession::new(transport);
    let table = FeatureTable::discover(&mut session, device_index)?;
    if !table.contains(FeatureId::ONBOARD_PROFILES) {
        return Err(ProtocolError::FeatureNotSupported {
            feature_id: FeatureId::ONBOARD_PROFILES,
        }
        .into());
    }
    Ok(execute_set_active_profile(
        &mut session,
        &table,
        slot,
        apply,
    )?)
}

/// Formats a [`ProfilesListReport`] into human-readable text.
#[must_use]
pub fn format_profiles_list_human(report: &ProfilesListReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Onboard Profiles (0x8100 v{}, {} mode):",
        report.feature_version,
        report.mode.to_uppercase()
    );
    if report.mode == "host" {
        let _ = writeln!(
            out,
            "  Active Slot: None (Host Software Mode overrides onboard memory)"
        );
    } else {
        let _ = writeln!(
            out,
            "  Active Slot: Slot {} / {}",
            report.current_profile, report.profile_count
        );
    }
    let _ = writeln!(
        out,
        "  Geometry:    {} profile slots, {} memory sectors ({} bytes/sector)",
        report.profile_count, report.sector_count, report.sector_size
    );
    let _ = writeln!(out);

    for p in &report.profiles {
        let status = if p.is_active {
            "[ACTIVE]"
        } else if p.enabled {
            "[ENABLED]"
        } else {
            "[DISABLED]"
        };

        let name_display = if p.name.is_empty() {
            "Default".to_string()
        } else {
            format!("\"{}\"", p.name)
        };

        let stages_str = p
            .dpi_stages
            .iter()
            .enumerate()
            .map(|(idx, &dpi)| {
                if idx == usize::from(p.default_dpi_stage) {
                    format!("{dpi}*")
                } else if idx == usize::from(p.switched_dpi_stage) {
                    format!("{dpi}#")
                } else {
                    dpi.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let _ = writeln!(out, "  Slot {} {status} - {name_display}:", p.slot);
        let _ = writeln!(out, "    Polling Rate: {} Hz", p.report_rate_hz);
        let _ = writeln!(
            out,
            "    DPI Stages:   [{stages_str}] (* default, # sniper)"
        );
        let _ = writeln!(
            out,
            "    Sector:       0x{:04X} (CRC verified)",
            p.sector_address
        );
    }

    out
}

/// Formats a [`ProfileBackupReport`] into human-readable text.
#[must_use]
pub fn format_profiles_backup_human(report: &ProfileBackupReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "[BACKUP COMPLETE]");
    let _ = writeln!(
        out,
        "  Device:        {}:{}",
        report.vendor_id, report.product_id
    );
    let _ = writeln!(out, "  Saved to:      {}", report.backup_path);
    let _ = writeln!(out, "  Sectors saved: {}", report.sectors_saved);
    let _ = writeln!(out, "  Total payload: {} bytes", report.total_bytes);
    out
}

/// Formats a [`ProfileRestoreReport`] into human-readable text.
#[must_use]
pub fn format_profiles_restore_human(report: &ProfileRestoreReport) -> String {
    let status_tag = if report.applied {
        "[RESTORE APPLIED]"
    } else {
        "[DRY-RUN]"
    };
    let mut out = String::new();
    let _ = writeln!(out, "{status_tag} Restore from {}", report.backup_path);
    let _ = writeln!(
        out,
        "  Sectors:       {} sectors ({} total bytes)",
        report.sectors_to_restore, report.total_bytes
    );
    let _ = writeln!(out, "  Planned wire operations:");
    for w in &report.planned_writes {
        let _ = writeln!(
            out,
            "    - Feature {} fn {} [{}]",
            w.feature_id, w.function_id, w.params_hex
        );
    }
    if let Some(note) = &report.note {
        let _ = writeln!(out, "  Notice: {note}");
    }
    out
}

/// Formats a [`ProfileSetActiveReport`] into human-readable text.
#[must_use]
pub fn format_profiles_set_active_human(report: &ProfileSetActiveReport) -> String {
    let status_tag = if report.applied {
        "[APPLIED]"
    } else {
        "[DRY-RUN]"
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{status_tag} Active profile switch: Slot {} -> Slot {}",
        report.previous_slot, report.requested_slot
    );
    if let Some(rb) = report.readback_slot {
        let _ = writeln!(out, "  Readback: Slot {rb} (verified: {})", report.verified);
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

#[cfg(test)]
mod tests {
    use hidpp::{
        DeviceIndex, FeatureIndex, FunctionId, HidDeviceInfo, HidppFrame, MockTransport, ReportId,
        SwId,
    };
    use squeak_core::PlannedWireFrame;

    use super::*;

    fn push_discovery_m6(mock: &mut MockTransport) {
        let dev_idx = DeviceIndex::DIRECT;
        let root = FeatureIndex(0);
        let fset = FeatureIndex(1);

        // 1. Root ping (HID++ 4.2)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                dev_idx,
                root,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x02, 0x53],
            )
            .unwrap(),
        );

        // 2. IRoot getFeature(0x0001) -> FeatureSet at index 1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                dev_idx,
                root,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00, 0x01],
            )
            .unwrap(),
        );

        // 3. IFeatureSet getCount -> 2 features
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                dev_idx,
                fset,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00, 0x00],
            )
            .unwrap(),
        );

        // 4. getFeatureId(1) -> 0x0001 v1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                fset,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );

        // 5. getFeatureId(2) -> 0x8100 v0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                fset,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x81, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
    }

    #[test]
    fn test_format_profiles_list_human() {
        let report = ProfilesListReport {
            feature_version: 0,
            mode: "onboard".to_string(),
            current_profile: 1,
            profile_count: 5,
            sector_count: 16,
            sector_size: 255,
            profiles: vec![squeak_core::OnboardProfileSlotReport {
                slot: 1,
                sector_address: 1,
                enabled: true,
                is_active: true,
                report_rate_hz: 1000,
                default_dpi_stage: 1,
                switched_dpi_stage: 0,
                dpi_stages: vec![1200, 2400, 3200],
                name: "Gaming".to_string(),
                crc_verified: true,
            }],
        };
        let formatted = format_profiles_list_human(&report);
        assert!(formatted.contains("Onboard Profiles"));
        assert!(formatted.contains("Slot 1 [ACTIVE] - \"Gaming\""));
        assert!(formatted.contains("1000 Hz"));
        assert!(formatted.contains("1200#, 2400*, 3200"));
    }

    #[test]
    fn test_format_profiles_backup_human() {
        let report = ProfileBackupReport {
            backup_path: "/tmp/backup.json".to_string(),
            vendor_id: "0x046D".to_string(),
            product_id: "0xC08B".to_string(),
            sectors_saved: 6,
            total_bytes: 1530,
        };
        let formatted = format_profiles_backup_human(&report);
        assert!(formatted.contains("[BACKUP COMPLETE]"));
        assert!(formatted.contains("/tmp/backup.json"));
        assert!(formatted.contains("1530 bytes"));
    }

    #[test]
    fn test_format_profiles_restore_human() {
        let report = ProfileRestoreReport {
            applied: false,
            backup_path: "/tmp/backup.json".to_string(),
            sectors_to_restore: 6,
            total_bytes: 1530,
            planned_writes: vec![PlannedWireFrame {
                feature_id: "0x8100".to_string(),
                feature_index: 2,
                function_id: 6,
                params_hex: "0000000000FF".to_string(),
            }],
            verified: false,
            note: Some("Dry run only".to_string()),
        };
        let formatted = format_profiles_restore_human(&report);
        assert!(formatted.contains("[DRY-RUN]"));
        assert!(formatted.contains("Dry run only"));
    }

    #[test]
    fn test_format_profiles_set_active_human() {
        let report = ProfileSetActiveReport {
            applied: true,
            requested_slot: 2,
            previous_slot: 1,
            readback_slot: Some(2),
            verified: true,
            planned_frame: PlannedWireFrame {
                feature_id: "0x8100".to_string(),
                feature_index: 2,
                function_id: 3,
                params_hex: "0002".to_string(),
            },
            note: None,
        };
        let formatted = format_profiles_set_active_human(&report);
        assert!(formatted.contains("[APPLIED]"));
        assert!(formatted.contains("Slot 1 -> Slot 2"));
        assert!(formatted.contains("verified: true"));
    }

    #[test]
    fn test_profiles_set_active_from_transport() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        push_discovery_m6(&mut mock);

        let dev_idx = DeviceIndex::DIRECT;
        let feat_idx = FeatureIndex(2);

        // 1. fn 0: getDescription (profile_count = 5)
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

        // 2. fn 4: getCurrentProfile -> currently slot 1
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

        // Dry run
        let report = profiles_set_active_from_transport(mock, dev_idx, 3, false).unwrap();
        assert!(!report.applied);
        assert_eq!(report.requested_slot, 3);
        assert_eq!(report.previous_slot, 1);
        assert!(!report.verified);
    }
}

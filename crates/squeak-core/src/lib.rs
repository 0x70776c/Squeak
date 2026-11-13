//! Shared domain models, data-driven device definitions (`data/devices/*.toml`),
//! and sanitized dump schemas for Squeak.

#![forbid(unsafe_code)]

pub mod device_data;
pub mod dump;
pub mod ops;
pub mod profile_backup;
pub mod settings;

pub use device_data::{
    BUILTIN_G502_HERO_SVG, BUILTIN_GENERIC_MOUSE_SVG, ButtonDefinition, DeviceDataError,
    DeviceDatabase, DeviceDefinition, DeviceKind, DeviceQuirks, GenericDeviceProfile,
    ResolvedDevice, button_label_map, extract_svg_ids, resolve_builtin_svg,
};
pub use dump::{
    DUMP_SCHEMA_VERSION, DefinitionMatchKind, DeviceDump, DeviceIdentityDump, DeviceInfoReport,
    DeviceListEntry, FeatureDumpEntry, FeatureFlagSummary, ProbeStatus,
};
pub use ops::{
    execute_dpi_write, execute_host_mode_write, execute_led_write, execute_profile_backup,
    execute_profile_restore, execute_report_rate_write, execute_set_active_profile,
    parse_led_effect_spec, parse_report_rate_hz, read_buttons_report, read_device_status,
    read_dpi_config, read_led_config, read_onboard_mode, read_profiles_summary,
    read_report_rate_config, read_scroll_config,
};
pub use profile_backup::{BACKUP_MAGIC_V1, ProfileBackup, ProfileBackupError, default_backup_path};
pub use settings::{
    BatteryStatusReport, ButtonEntryReport, ButtonsReport, DaemonDeviceSummary, DeviceNameReport,
    DeviceStatusReport, DpiCapabilitiesReport, DpiConfigReport, DpiRangeReport, DpiWriteReport,
    FirmwareEntityReport, FirmwareSummaryReport, LedConfigReport, LedWriteReport,
    LedZoneStatusReport, OnboardModeReport, OnboardModeWriteReport, OnboardProfileSlotReport,
    PlannedWireFrame, ProfileBackupReport, ProfileRestoreReport, ProfileSetActiveReport,
    ProfilesListReport, ReportRateConfigReport, ReportRateWriteReport, ScrollConfigReport,
    SensorDpiReport,
};

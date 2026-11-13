//! Shared domain models, data-driven device definitions (`data/devices/*.toml`),
//! and sanitized dump schemas for Squeak.

#![forbid(unsafe_code)]

pub mod device_data;
pub mod dump;
pub mod ops;
pub mod settings;

pub use device_data::{
    ButtonDefinition, DeviceDataError, DeviceDatabase, DeviceDefinition, DeviceKind, DeviceQuirks,
    GenericDeviceProfile, ResolvedDevice, button_label_map, extract_svg_ids,
};
pub use dump::{
    DUMP_SCHEMA_VERSION, DefinitionMatchKind, DeviceDump, DeviceIdentityDump, DeviceInfoReport,
    DeviceListEntry, FeatureDumpEntry, FeatureFlagSummary, ProbeStatus,
};
pub use ops::{
    execute_dpi_write, execute_report_rate_write, parse_report_rate_hz, read_device_status,
    read_dpi_config, read_report_rate_config,
};
pub use settings::{
    BatteryStatusReport, DaemonDeviceSummary, DeviceNameReport, DeviceStatusReport,
    DpiCapabilitiesReport, DpiConfigReport, DpiRangeReport, DpiWriteReport, FirmwareEntityReport,
    FirmwareSummaryReport, PlannedWireFrame, ReportRateConfigReport, ReportRateWriteReport,
    SensorDpiReport,
};

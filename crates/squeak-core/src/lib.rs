//! Shared domain models, data-driven device definitions (`data/devices/*.toml`),
//! and sanitized dump schemas for Squeak.

#![forbid(unsafe_code)]

pub mod device_data;
pub mod dump;
pub mod settings;

pub use device_data::{
    ButtonDefinition, DeviceDataError, DeviceDatabase, DeviceDefinition, DeviceKind, DeviceQuirks,
    GenericDeviceProfile, ResolvedDevice, button_label_map, extract_svg_ids,
};
pub use dump::{
    DUMP_SCHEMA_VERSION, DefinitionMatchKind, DeviceDump, DeviceIdentityDump, DeviceInfoReport,
    DeviceListEntry, FeatureDumpEntry, FeatureFlagSummary, ProbeStatus,
};
pub use settings::{
    BatteryStatusReport, DeviceNameReport, DeviceStatusReport, DpiCapabilitiesReport,
    DpiConfigReport, DpiRangeReport, DpiWriteReport, FirmwareEntityReport, FirmwareSummaryReport,
    PlannedWireFrame, ReportRateConfigReport, ReportRateWriteReport, SensorDpiReport,
};

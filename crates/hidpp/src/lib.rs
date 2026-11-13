//! Pure HID++ 2.0 protocol library for Squeak.
//!
//! This crate intentionally has no GUI, `libudev`, or D-Bus dependencies. All hardware
//! I/O passes through the [`HidTransport`] trait implemented by Linux `hidraw`
//! ([`HidrawTransport`]) and in-memory test mocks ([`MockTransport`]).
//!
//! # Key Guarantees
//!
//! - Safe Rust only (`#![forbid(unsafe_code)]`).
//! - Runtime feature discovery via [`IRoot`] (`0x0000`) and [`IFeatureSet`] (`0x0001`);
//!   feature indices are never hardcoded and devices never branch on model.
//! - Serial numbers (`HID_UNIQ`) are never collected, retained, or logged.

#![forbid(unsafe_code)]

pub mod descriptor;
pub mod error;
pub mod feature;
pub mod frame;
pub mod hidraw;
pub mod session;
pub mod transport;

pub use error::{
    Hidpp10Error, Hidpp10ErrorCode, Hidpp20Error, Hidpp20ErrorCode, HidppError, ProtocolError,
    TransportError,
};
pub use feature::{
    AdjustableDpiDriver, AdjustableDpiFeature, BatteryCapability, BatteryChargingStatus,
    BatteryFeature, BatteryLevelBucket, BatteryPercent, BatteryReading, BatteryStatusDriver,
    BatteryStatusReply, DeviceFirmwareSummary, DeviceNameDriver, DeviceNameFeature, Dpi,
    DpiCapabilities, DpiRange, DpiWriteVerification, ExtendedReportRateDriver, FeatureCache,
    FeatureEntry, FeatureFlags, FeatureId, FeatureTable, FirmwareEntity, FirmwareEntityKind,
    FirmwareInfoDriver, FirmwareInfoFeature, FirmwareInventory, HidppDeviceType, HidppFeature,
    IFeatureSet, IRoot, MAX_PROTOCOL_DPI, ProtocolVersion, ReportRate, ReportRateDriver,
    ReportRateFeature, ReportRateState, ReportRateWriteVerification, ResolvedFeature,
    SensorDpiState, TransportCapabilities, UnifiedBatteryCapabilities, UnifiedBatteryDriver,
    UnifiedBatteryStatus, read_battery, read_report_rate, set_report_rate_verified,
};
pub use frame::{
    DeviceIndex, FeatureIndex, FunctionId, HidppFrame, LONG_PARAM_LEN, LONG_REPORT_LEN,
    MAX_REPORT_LEN, ReportId, SHORT_PARAM_LEN, SHORT_REPORT_LEN, SupportedReports, SwId,
    VERY_LONG_PARAM_LEN, VERY_LONG_REPORT_LEN,
};
pub use hidraw::{HIDPP_VENDOR_ID, HidrawTransport, discover_hidpp_devices};
pub use session::{HidppSession, SessionConfig};
pub use transport::{BusType, HidDeviceInfo, HidTransport, MockTransport};

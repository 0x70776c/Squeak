//! Sanitized M3 capability, readout, dry-run plan, and verified-write schemas.
//!
//! All types in this module exclude per-unit hardware serial numbers (`HID_UNIQ`
//! and `0x0003` `unitId`).

use hidpp::{BatteryReading, DpiCapabilities, FirmwareInventory, ReportRateState, SensorDpiState};
use serde::{Deserialize, Serialize};

use crate::device_data::DeviceKind;
use crate::dump::{DefinitionMatchKind, DeviceIdentityDump, ProbeStatus};

/// Sanitized summary of `DEVICE_NAME` (`0x0005`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceNameReport {
    /// UTF-8 product/device name returned by `0x0005`.
    pub name: String,
    /// Device classification label (e.g., `"mouse"`).
    pub device_type: String,
}

/// One firmware/hardware entry from `FIRMWARE_INFO` (`0x0003`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareEntityReport {
    /// Zero-based entity index.
    pub index: u8,
    /// Entity classification label (`"main-application"`, `"bootloader"`, `"hardware"`, ...).
    pub kind: String,
    /// Formatted version string (e.g., `"MPM 12.03.B0019"` or `"rev 4"`).
    pub version: String,
    /// Whether this entity is the active responder.
    pub active: bool,
    /// Transport PID formatted as uppercase hex when non-zero.
    pub transport_pid: Option<String>,
}

/// Sanitized firmware inventory derived from `FIRMWARE_INFO` (`0x0003`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareSummaryReport {
    /// Total number of firmware/hardware entities reported by firmware.
    pub entity_count: u8,
    /// Advertised transport names (e.g., `["usb"]`).
    pub transports: Vec<String>,
    /// Per-transport model PIDs (`0xXXXX`).
    pub model_pids: Vec<String>,
    /// Firmware/hardware entities (`0..entity_count`).
    pub entities: Vec<FirmwareEntityReport>,
}

impl FirmwareSummaryReport {
    /// Maps a [`FirmwareInventory`] into a sanitized [`FirmwareSummaryReport`].
    #[must_use]
    pub fn from_inventory(inventory: &FirmwareInventory) -> Self {
        let mut transports = Vec::new();
        if let Some(tr) = inventory.summary.transports {
            if tr.usb() {
                transports.push("usb".to_owned());
            }
            if tr.equad() {
                transports.push("equad".to_owned());
            }
            if tr.ble() {
                transports.push("ble".to_owned());
            }
            if tr.bluetooth() {
                transports.push("bluetooth".to_owned());
            }
        }

        let model_pids = inventory
            .summary
            .model_pids
            .iter()
            .map(|pid| format!("0x{pid:04X}"))
            .collect();

        let entities = inventory
            .entities
            .iter()
            .map(|entity| FirmwareEntityReport {
                index: entity.index,
                kind: entity.kind.label().to_owned(),
                version: entity.display_version(),
                active: entity.active,
                transport_pid: (entity.transport_pid != 0)
                    .then(|| format!("0x{:04X}", entity.transport_pid)),
            })
            .collect();

        Self {
            entity_count: inventory.summary.entity_count,
            transports,
            model_pids,
            entities,
        }
    }
}

/// Sanitized battery readout from `UNIFIED_BATTERY` (`0x1004`) or `BATTERY_STATUS` (`0x1000`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryStatusReport {
    /// Feature ID that produced this reading (`"0x1004"` or `"0x1000"`).
    pub source_feature: String,
    /// Exact state-of-charge percentage (`0..=100`), when reported.
    pub percentage: Option<u8>,
    /// Best available percentage (exact or bucket approximation).
    pub approximate_percent: Option<u8>,
    /// Coarse level bucket (`"full"`, `"good"`, `"low"`, `"critical"`, ...).
    pub level_bucket: Option<String>,
    /// Charging status label (`"discharging"`, `"recharging"`, `"full"`, ...).
    pub charging_status: String,
    /// External power byte if reported (`UNIFIED_BATTERY` v1+).
    pub external_power: Option<u8>,
}

impl From<BatteryReading> for BatteryStatusReport {
    fn from(reading: BatteryReading) -> Self {
        Self {
            source_feature: format!("0x{:04X}", reading.source_feature.0),
            percentage: reading.percentage.map(|p| p.as_u8()),
            approximate_percent: reading.best_percent(),
            level_bucket: reading.level_bucket.map(|b| b.to_string()),
            charging_status: reading.charging_status.label().to_owned(),
            external_power: reading.external_power,
        }
    }
}

/// Stepped DPI range advertised by `ADJUSTABLE_DPI` (`0x2201`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiRangeReport {
    /// Minimum DPI in range.
    pub min: u16,
    /// Increment step in DPI.
    pub step: u16,
    /// Maximum DPI in range.
    pub max: u16,
}

/// Supported DPI capabilities for one sensor (`0x2201`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiCapabilitiesReport {
    /// Discrete selectable DPI values.
    pub discrete: Vec<u16>,
    /// Continuous stepped DPI ranges.
    pub ranges: Vec<DpiRangeReport>,
    /// Overall minimum supported DPI.
    pub min_dpi: Option<u16>,
    /// Overall maximum supported DPI.
    pub max_dpi: Option<u16>,
}

impl From<&DpiCapabilities> for DpiCapabilitiesReport {
    fn from(caps: &DpiCapabilities) -> Self {
        Self {
            discrete: caps.discrete.iter().map(|d| d.as_u16()).collect(),
            ranges: caps
                .ranges
                .iter()
                .map(|r| DpiRangeReport {
                    min: r.min.as_u16(),
                    step: r.step,
                    max: r.max.as_u16(),
                })
                .collect(),
            min_dpi: caps.min_dpi().map(|d| d.as_u16()),
            max_dpi: caps.max_dpi().map(|d| d.as_u16()),
        }
    }
}

/// Active DPI state and capabilities for a single sensor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorDpiReport {
    /// Sensor index (`0` for single-sensor mice).
    pub sensor_index: u8,
    /// Active sensor DPI.
    pub current_dpi: u16,
    /// Firmware default DPI when reported (`0x2201` v1+).
    pub default_dpi: Option<u16>,
    /// Supported DPI values/ranges for this sensor.
    pub capabilities: DpiCapabilitiesReport,
}

impl SensorDpiReport {
    /// Constructs a [`SensorDpiReport`] from sensor state and capability list.
    #[must_use]
    pub fn from_state(state: SensorDpiState, capabilities: &DpiCapabilities) -> Self {
        Self {
            sensor_index: state.sensor_index,
            current_dpi: state.current.as_u16(),
            default_dpi: state.default_dpi.map(|d| d.as_u16()),
            capabilities: DpiCapabilitiesReport::from(capabilities),
        }
    }
}

/// Complete `ADJUSTABLE_DPI` (`0x2201`) snapshot across all sensors on a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiConfigReport {
    /// Feature version reported by firmware (`0x2201`).
    pub feature_version: u8,
    /// Whether this driver version permits mutating writes (`version <= MAX_SUPPORTED_VERSION`).
    pub write_supported: bool,
    /// Per-sensor state and capabilities.
    pub sensors: Vec<SensorDpiReport>,
}

/// Complete polling report-rate snapshot from `0x8060` or `0x8061`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRateConfigReport {
    /// Feature ID used (`"0x8060"` or `"0x8061"`).
    pub source_feature: String,
    /// Feature version reported by firmware.
    pub feature_version: u8,
    /// Whether this driver version permits mutating writes.
    pub write_supported: bool,
    /// Active polling rate in Hz.
    pub current_hz: u16,
    /// Active polling interval in microseconds.
    pub current_interval_us: u16,
    /// Supported polling rates in Hz (sorted descending or device-mask order).
    pub supported_hz: Vec<u16>,
}

impl ReportRateConfigReport {
    /// Builds a [`ReportRateConfigReport`] from a [`ReportRateState`].
    #[must_use]
    pub fn from_state(state: &ReportRateState, feature_version: u8, write_supported: bool) -> Self {
        Self {
            source_feature: format!("0x{:04X}", state.source_feature.0),
            feature_version,
            write_supported,
            current_hz: state.current.hz(),
            current_interval_us: state.current.interval_us(),
            supported_hz: state.supported.iter().map(|r| r.hz()).collect(),
        }
    }
}

/// Aggregated read-only M3 readout returned by `squeak status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatusReport {
    /// Hidraw device node path.
    pub path: String,
    /// Identity summary (no serial numbers).
    pub device: DeviceIdentityDump,
    /// Negotiated HID++ version (e.g. `"4.2"`).
    pub protocol_version: String,
    /// `DEVICE_NAME` (`0x0005`) readout if present on the device.
    pub device_name: Option<DeviceNameReport>,
    /// `FIRMWARE_INFO` (`0x0003`) inventory if present on the device.
    pub firmware: Option<FirmwareSummaryReport>,
    /// Battery status from `0x1004` or `0x1000` if present on the device.
    pub battery: Option<BatteryStatusReport>,
    /// DPI configuration from `0x2201` if present on the device.
    pub dpi: Option<DpiConfigReport>,
    /// Polling report rate from `0x8061` or `0x8060` if present on the device.
    pub report_rate: Option<ReportRateConfigReport>,
    /// Non-fatal warnings/notices encountered while reading optional features.
    pub notices: Vec<String>,
}

/// Wire frame preview used for `--dry-run` and audit output of M3 write commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedWireFrame {
    /// HID++ feature identifier (`"0x2201"`, `"0x8060"`, `"0x8061"`).
    pub feature_id: String,
    /// Runtime-discovered feature index.
    pub feature_index: u8,
    /// Function ID (`0..15`).
    pub function_id: u8,
    /// Parameter bytes formatted as hex pairs.
    pub params_hex: String,
}

/// Output of `squeak dpi set` (both `--dry-run` preview and `--apply` verified write).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiWriteReport {
    /// `true` when the write frame was actually transmitted (`--apply`), `false` in `--dry-run`.
    pub applied: bool,
    /// Target sensor index.
    pub sensor_index: u8,
    /// Sensor DPI read prior to applying (or during dry-run validation).
    pub previous_dpi: u16,
    /// Requested target DPI.
    pub requested_dpi: u16,
    /// Post-write readback DPI (`Some` when `--apply` ran, `None` in `--dry-run`).
    pub readback_dpi: Option<u16>,
    /// `true` only when `--apply` succeeded and readback matched `requested_dpi`.
    pub verified: bool,
    /// Wire command details that were (or would be) sent.
    pub planned_frame: PlannedWireFrame,
    /// Optional human/safety notice (e.g., onboard-profile note).
    pub note: Option<String>,
}

/// Output of `squeak report-rate set` (both `--dry-run` preview and `--apply` verified write).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRateWriteReport {
    /// `true` when the write frame was actually transmitted (`--apply`), `false` in `--dry-run`.
    pub applied: bool,
    /// Feature ID used (`"0x8060"` or `"0x8061"`).
    pub source_feature: String,
    /// Report rate read prior to applying (in Hz).
    pub previous_hz: u16,
    /// Requested target polling rate (in Hz).
    pub requested_hz: u16,
    /// Post-write readback rate in Hz (`Some` when `--apply` ran, `None` in `--dry-run`).
    pub readback_hz: Option<u16>,
    /// `true` only when `--apply` succeeded and readback matched `requested_hz`.
    pub verified: bool,
    /// Wire command details that were (or would be) sent.
    pub planned_frame: PlannedWireFrame,
    /// Optional human/safety notice.
    pub note: Option<String>,
}

/// Sanitized device summary exposed by `squeakd` (`dev.squeak.Daemon::ListDevices`).
///
/// Never includes `HID_UNIQ` or per-unit serial numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonDeviceSummary {
    /// Sanitized runtime device key (e.g. `"hidraw6-046d-c08b-ff"`).
    pub device_key: String,
    /// Current `/dev/hidraw*` node path.
    pub path: String,
    /// Bus (`"USB"`, `"Bluetooth"`, ...).
    pub bus: String,
    /// Vendor ID formatted as lowercase hex (`"0x046d"`).
    pub vendor_id: String,
    /// Product ID formatted as lowercase hex (`"0xc08b"`).
    pub product_id: String,
    /// Kernel-reported HID product string, if any.
    pub hid_name: Option<String>,
    /// Target HID++ device index (`"0xFF"` or `"0x01".."0x06"`).
    pub device_index: String,
    /// Whether the VID:PID matched a device definition TOML or uses generic mode.
    pub definition_match: DefinitionMatchKind,
    /// Display name from definition TOML or kernel HID fallback.
    pub matched_name: String,
    /// Device kind (`mouse`, `keyboard`, ...).
    pub kind: DeviceKind,
    /// Negotiated HID++ protocol version (e.g. `"4.2"`), if accessible.
    pub protocol_version: Option<String>,
    /// Probe/accessibility status.
    pub probe_status: ProbeStatus,
    /// Number of runtime-discovered HID++ 2.0 features (`0` if unreachable/asleep).
    pub feature_count: usize,
    /// Cached desired sensor-0 DPI target stored by the daemon for reconnect/wake re-apply.
    pub desired_dpi: Option<u16>,
    /// Cached desired report rate (Hz) stored by the daemon for reconnect/wake re-apply.
    pub desired_report_rate_hz: Option<u16>,
    /// Optional status/troubleshooting hint.
    pub hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use hidpp::{
        BatteryChargingStatus, BatteryLevelBucket, BatteryPercent, DeviceFirmwareSummary, Dpi,
        DpiRange, FeatureId, FirmwareEntity, FirmwareEntityKind, ReportRate, TransportCapabilities,
    };

    use super::*;

    #[test]
    fn maps_sanitized_firmware_and_battery_without_serials() {
        let inventory = FirmwareInventory {
            summary: DeviceFirmwareSummary {
                entity_count: 2,
                transports: Some(TransportCapabilities::from_raw(TransportCapabilities::USB)),
                model_pids: vec![0xC08B],
                capabilities: Some(0x01),
            },
            entities: vec![
                FirmwareEntity {
                    index: 0,
                    kind: FirmwareEntityKind::MainApplication,
                    prefix: "U1".to_owned(),
                    fw_number: 0x12,
                    revision: 0x03,
                    build: 0x0019,
                    active: true,
                    transport_pid: 0xC08B,
                    hardware_revision: None,
                },
                FirmwareEntity {
                    index: 1,
                    kind: FirmwareEntityKind::Hardware,
                    prefix: String::new(),
                    fw_number: 0,
                    revision: 0,
                    build: 0,
                    active: false,
                    transport_pid: 0,
                    hardware_revision: Some(2),
                },
            ],
        };

        let fw_report = FirmwareSummaryReport::from_inventory(&inventory);
        assert_eq!(fw_report.entity_count, 2);
        assert_eq!(fw_report.transports, vec!["usb"]);
        assert_eq!(fw_report.model_pids, vec!["0xC08B"]);
        assert_eq!(fw_report.entities[0].version, "U1 12.03.B0019");
        assert_eq!(
            fw_report.entities[0].transport_pid.as_deref(),
            Some("0xC08B")
        );
        assert_eq!(fw_report.entities[1].version, "rev 2");
        assert_eq!(fw_report.entities[1].transport_pid, None);

        let fw_json = serde_json::to_string(&fw_report).unwrap();
        assert!(!fw_json.contains("unit_id"));
        assert!(!fw_json.contains("unitId"));

        let reading = BatteryReading {
            source_feature: FeatureId::UNIFIED_BATTERY,
            percentage: Some(BatteryPercent::from_wire(85).unwrap()),
            level_bucket: Some(BatteryLevelBucket::Full),
            next_level: None,
            charging_status: BatteryChargingStatus::Discharging,
            external_power: Some(0),
        };
        let bat_report = BatteryStatusReport::from(reading);
        assert_eq!(bat_report.source_feature, "0x1004");
        assert_eq!(bat_report.percentage, Some(85));
        assert_eq!(bat_report.approximate_percent, Some(85));
        assert_eq!(bat_report.charging_status, "discharging");
    }

    #[test]
    fn maps_dpi_and_report_rate_reports() {
        let caps = DpiCapabilities {
            sensor_index: 0,
            discrete: vec![],
            ranges: vec![DpiRange {
                min: Dpi::new(100).unwrap(),
                step: 50,
                max: Dpi::new(25600).unwrap(),
            }],
        };
        let sensor = SensorDpiReport::from_state(
            SensorDpiState {
                sensor_index: 0,
                current: Dpi::new(1600).unwrap(),
                default_dpi: Some(Dpi::new(800).unwrap()),
            },
            &caps,
        );
        assert_eq!(sensor.current_dpi, 1600);
        assert_eq!(sensor.default_dpi, Some(800));
        assert_eq!(sensor.capabilities.min_dpi, Some(100));
        assert_eq!(sensor.capabilities.max_dpi, Some(25600));

        let rr = ReportRateConfigReport::from_state(
            &ReportRateState {
                source_feature: FeatureId::REPORT_RATE,
                current: ReportRate::Hz1000,
                supported: vec![
                    ReportRate::Hz1000,
                    ReportRate::Hz500,
                    ReportRate::Hz250,
                    ReportRate::Hz125,
                ],
            },
            0,
            true,
        );
        assert_eq!(rr.current_hz, 1000);
        assert_eq!(rr.current_interval_us, 1000);
        assert_eq!(rr.supported_hz, vec![1000, 500, 250, 125]);
    }
}

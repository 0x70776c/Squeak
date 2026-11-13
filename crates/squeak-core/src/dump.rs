//! Sanitized device inventory and `dump-device` schemas for CLI/PR reporting.
//!
//! Serial numbers (`HID_UNIQ`) and host-private identifiers are structurally absent from
//! all types in this module.

use hidpp::{FeatureEntry, FeatureId, FeatureTable, HidDeviceInfo};
use serde::{Deserialize, Serialize};

use crate::device_data::{DeviceKind, ResolvedDevice};

/// Current JSON schema version for [`DeviceDump`].
pub const DUMP_SCHEMA_VERSION: u32 = 1;

mod hex_u16_lower {
    use serde::{Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(val: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("0x{val:04x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        struct HexOrInt;
        impl de::Visitor<'_> for HexOrInt {
            type Value = u16;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a u16 integer or hex string like 0x046d")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<u16, E> {
                u16::try_from(v).map_err(|_| de::Error::custom("u16 overflow"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<u16, E> {
                let s = v.trim();
                if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    u16::from_str_radix(hex, 16).map_err(de::Error::custom)
                } else {
                    s.parse::<u16>().map_err(de::Error::custom)
                }
            }
        }
        d.deserialize_any(HexOrInt)
    }
}

mod hex_u16_upper {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("0x{val:04X}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        super::hex_u16_lower::deserialize(d)
    }
}

/// Whether a discovered device matched a TOML file in `data/devices/` or uses generic mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefinitionMatchKind {
    /// Matched an explicit `(vendor_id, product_id)` definition file.
    Known,
    /// Unrecognized PID running in generic HID++ 2.0 mode.
    Generic,
}

/// Result of probing an HID++ `hidraw` endpoint during `squeak list`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ProbeStatus {
    /// Successfully pinged over HID++ 2.0+.
    Accessible,
    /// `/dev/hidraw*` permission denied (`EACCES`/`EPERM`); run `squeak setup-udev`.
    PermissionDenied,
    /// Device answered with HID++ 1.0 `ERROR_MSG` (legacy out-of-scope hardware).
    LegacyHidpp10,
    /// Device did not reply before deadline (may be asleep/powered off).
    TimeoutOrAsleep,
    /// Probe failed with another diagnostic error message.
    Error {
        /// Human-readable error summary (sanitized of any serial numbers).
        message: String,
    },
}

/// Summary row produced by `squeak list` for one discovered HID++ endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceListEntry {
    /// Local device node path (for example `/dev/hidraw1`).
    pub path: String,
    /// Physical bus (`USB`, `Bluetooth`, etc.).
    pub bus: String,
    /// Vendor ID formatted cleanly (`"0x046d"` in JSON).
    #[serde(with = "hex_u16_lower")]
    pub vendor_id: u16,
    /// Product ID formatted cleanly (`"0xc08b"` in JSON).
    #[serde(with = "hex_u16_lower")]
    pub product_id: u16,
    /// Device name reported by `HID_NAME`, if present.
    pub hid_name: Option<String>,
    /// Whether `data/devices/*.toml` matched this VID:PID.
    pub definition_match: DefinitionMatchKind,
    /// Matched TOML model name or generic fallback label.
    pub matched_name: String,
    /// Protocol version (`"4.2"`) if accessible and ping succeeded.
    pub protocol_version: Option<String>,
    /// Outcome of probing this endpoint.
    pub probe_status: ProbeStatus,
    /// Actionable hint (for example recommending `squeak setup-udev`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Capability and visibility bit summary for one feature entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureFlagSummary {
    /// Obsolete flag (`0x80`).
    pub obsolete: bool,
    /// Software-hidden flag (`0x40`).
    pub hidden: bool,
    /// Engineering/internal flag (`0x20`).
    pub engineering: bool,
    /// Raw wire feature flags byte.
    pub raw: u8,
}

/// Sanitized single-feature entry inside [`DeviceDump`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureDumpEntry {
    /// Runtime feature table index (`0..=count`).
    pub index: u8,
    /// 16-bit HID++ 2.0 feature ID (`"0x2201"` in JSON).
    #[serde(with = "hex_u16_upper")]
    pub feature_id: u16,
    /// Symbolic HID++ feature name or `"UNKNOWN"`.
    pub name: String,
    /// Feature revision reported by firmware.
    pub version: u8,
    /// Decoded feature metadata flags.
    pub flags: FeatureFlagSummary,
}

impl From<&FeatureEntry> for FeatureDumpEntry {
    fn from(entry: &FeatureEntry) -> Self {
        Self {
            index: entry.index.0,
            feature_id: entry.id.as_u16(),
            name: entry.id.display_name().to_owned(),
            version: entry.version,
            flags: FeatureFlagSummary {
                obsolete: entry.flags.is_obsolete(),
                hidden: entry.flags.is_hidden(),
                engineering: entry.flags.is_engineering(),
                raw: entry.flags.raw(),
            },
        }
    }
}

/// Non-identifying hardware metadata included inside [`DeviceDump`] and `squeak info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentityDump {
    /// Physical HID bus (`USB`, `Bluetooth`, etc.).
    pub bus: String,
    /// Vendor ID (`"0x046d"` in JSON).
    #[serde(with = "hex_u16_lower")]
    pub vendor_id: u16,
    /// Product ID (`"0xc08b"` in JSON).
    #[serde(with = "hex_u16_lower")]
    pub product_id: u16,
    /// Model string from `HID_NAME` (never contains serials).
    pub hid_name: Option<String>,
    /// Target device index probed (`"0xFF"` for direct USB/BT, `"0x01"`..`"0x06"` for receiver).
    pub device_index: String,
    /// Known TOML match vs. generic HID++ 2.0 fallback.
    pub definition_match: DefinitionMatchKind,
    /// Resolved display name.
    pub matched_name: String,
    /// Device kind from matching TOML or generic fallback.
    pub kind: DeviceKind,
    /// HID++ report IDs supported by this interface (`["0x10", "0x11"]`).
    pub supported_reports: Vec<String>,
}

/// Sanitized full runtime discovery dump produced by `squeak dump-device`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDump {
    /// Dump schema version ([`DUMP_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Squeak CLI/crate version that produced the dump.
    pub squeak_version: String,
    /// Non-identifying device descriptor/identity block (no serial numbers).
    pub device: DeviceIdentityDump,
    /// HID++ protocol version (`"major.minor"`).
    pub protocol_version: String,
    /// Complete runtime feature table discovered via `IRoot` + `IFeatureSet`.
    pub features: Vec<FeatureDumpEntry>,
}

impl DeviceDump {
    /// Constructs a sanitized [`DeviceDump`] from discovered transport metadata, runtime
    /// [`FeatureTable`], and TOML [`ResolvedDevice`] lookup result.
    #[must_use]
    pub fn from_discovery(
        squeak_version: &str,
        info: &HidDeviceInfo,
        table: &FeatureTable,
        resolved: ResolvedDevice<'_>,
    ) -> Self {
        let definition_match = if resolved.is_known() {
            DefinitionMatchKind::Known
        } else {
            DefinitionMatchKind::Generic
        };

        let matched_name = resolved.display_name(info.name.as_deref());
        let supported_reports = info
            .supported_reports
            .as_vec()
            .into_iter()
            .map(|r| r.to_string())
            .collect();

        let features = table.entries().iter().map(FeatureDumpEntry::from).collect();

        Self {
            schema_version: DUMP_SCHEMA_VERSION,
            squeak_version: squeak_version.to_owned(),
            device: DeviceIdentityDump {
                bus: info.bus.to_string(),
                vendor_id: info.vendor_id,
                product_id: info.product_id,
                hid_name: info.name.clone(),
                device_index: format!("0x{:02X}", table.device_index().as_byte()),
                definition_match,
                matched_name,
                kind: resolved.kind(),
                supported_reports,
            },
            protocol_version: table.protocol_version().to_string(),
            features,
        }
    }
}

/// Concise capability overview produced by `squeak info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfoReport {
    /// Device node path (for example `/dev/hidraw1`).
    pub path: String,
    /// Non-identifying device identity metadata.
    pub device: DeviceIdentityDump,
    /// HID++ protocol version (`"major.minor"`).
    pub protocol_version: String,
    /// Total number of discovered runtime feature entries (including `IRoot`).
    pub feature_count: usize,
    /// Highlighted user-relevant capabilities discovered at runtime.
    pub detected_capabilities: Vec<String>,
}

impl DeviceInfoReport {
    /// Summarizes a device's runtime feature table for `squeak info`.
    #[must_use]
    pub fn from_dump(path: String, dump: &DeviceDump, table: &FeatureTable) -> Self {
        let mut caps = Vec::new();
        let interesting = [
            (FeatureId::DEVICE_NAME, "Device name/type"),
            (FeatureId::FIRMWARE_INFO, "Firmware info"),
            (FeatureId::BATTERY_STATUS, "Battery status"),
            (FeatureId::UNIFIED_BATTERY, "Unified battery"),
            (FeatureId::ADJUSTABLE_DPI, "Adjustable DPI"),
            (FeatureId::REPORT_RATE, "Report rate"),
            (FeatureId::EXTENDED_REPORT_RATE, "Extended report rate"),
            (FeatureId::REPROG_CONTROLS_V4, "Programmable buttons (v4)"),
            (FeatureId::HIRES_WHEEL, "High-resolution wheel"),
            (FeatureId::SMART_SHIFT, "SmartShift wheel"),
            (FeatureId::COLOR_LED_EFFECTS, "Color LED effects"),
            (FeatureId::RGB_EFFECTS, "RGB effects"),
            (FeatureId::ONBOARD_PROFILES, "Onboard profiles"),
        ];
        for (fid, label) in interesting {
            if let Some(entry) = table.get(fid) {
                caps.push(format!(
                    "{label} (0x{:04X} v{})",
                    fid.as_u16(),
                    entry.version
                ));
            }
        }

        Self {
            path,
            device: dump.device.clone(),
            protocol_version: dump.protocol_version.clone(),
            feature_count: dump.features.len(),
            detected_capabilities: caps,
        }
    }
}

#[cfg(test)]
mod tests {
    use hidpp::descriptor::parse_sysfs_uevent;
    use hidpp::{
        DeviceIndex, FeatureEntry, FeatureFlags, FeatureIndex, ProtocolVersion, SupportedReports,
    };

    use super::*;
    use crate::device_data::DeviceDatabase;

    #[test]
    fn device_dump_omits_serial_numbers_and_formats_hex_cleanly() {
        let uevent = parse_sysfs_uevent(
            "HID_ID=0003:0000046D:0000C08B\nHID_NAME=Logitech G502 HERO Gaming Mouse\nHID_UNIQ=SN999988887777\n",
        )
        .unwrap();
        let info = HidDeviceInfo::from_sysfs(
            "/dev/hidraw1".into(),
            uevent,
            SupportedReports::SHORT_AND_LONG,
        );

        let table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 2 },
            vec![
                FeatureEntry {
                    index: FeatureIndex::IROOT,
                    id: FeatureId::IROOT,
                    version: 0,
                    flags: FeatureFlags::default(),
                },
                FeatureEntry {
                    index: FeatureIndex(1),
                    id: FeatureId::IFEATURE_SET,
                    version: 2,
                    flags: FeatureFlags::default(),
                },
                FeatureEntry {
                    index: FeatureIndex(5),
                    id: FeatureId::ADJUSTABLE_DPI,
                    version: 1,
                    flags: FeatureFlags::default(),
                },
            ],
        );

        let db = DeviceDatabase::builtin().unwrap();
        let resolved = db.resolve(info.vendor_id, info.product_id);
        let dump = DeviceDump::from_discovery("0.1.0", &info, &table, resolved);

        let json = serde_json::to_string_pretty(&dump).unwrap();
        assert!(json.contains("\"vendor_id\": \"0x046d\""));
        assert!(json.contains("\"product_id\": \"0xc08b\""));
        assert!(json.contains("\"feature_id\": \"0x2201\""));
        assert!(json.contains("\"definition_match\": \"known\""));
        assert!(!json.contains("SN999988887777"));
        assert!(!json.to_lowercase().contains("serial"));
        assert!(!json.to_lowercase().contains("hid_uniq"));
        assert!(!json.contains("/dev/hidraw1"));

        let roundtrip: DeviceDump = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, dump);
    }
}

//! Runtime HID++ 2.0 feature definitions, identifiers, flags, and discovery table.

pub mod cache;
pub mod feature_set;
pub mod root;

use core::fmt;

use tracing::warn;

use crate::frame::FeatureIndex;

pub use cache::{FeatureCache, FeatureTable};
pub use feature_set::IFeatureSet;
pub use root::{IRoot, ResolvedFeature};

/// 16-bit HID++ 2.0 feature identifier (for example `0x0000` for `IRoot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeatureId(pub u16);

impl FeatureId {
    /// `0x0000`: Root feature (`IRoot`, always at feature index `0x00`).
    pub const IROOT: Self = Self(0x0000);
    /// `0x0001`: Runtime feature enumeration (`IFeatureSet`).
    pub const IFEATURE_SET: Self = Self(0x0001);
    /// `0x0003`: Firmware, hardware, and protocol version info (`FIRMWARE_INFO`).
    pub const FIRMWARE_INFO: Self = Self(0x0003);
    /// `0x0005`: Device name and marketing type (`DEVICE_NAME`).
    pub const DEVICE_NAME: Self = Self(0x0005);
    /// `0x0007`: User-assignable friendly name (`DEVICE_FRIENDLY_NAME`).
    pub const DEVICE_FRIENDLY_NAME: Self = Self(0x0007);
    /// `0x1000`: Classic battery level/status (`BATTERY_STATUS`).
    pub const BATTERY_STATUS: Self = Self(0x1000);
    /// `0x1004`: Unified battery level/charging status (`UNIFIED_BATTERY`).
    pub const UNIFIED_BATTERY: Self = Self(0x1004);
    /// `0x1B04`: Programmable controls & button remapping v4 (`REPROG_CONTROLS_V4`).
    pub const REPROG_CONTROLS_V4: Self = Self(0x1B04);
    /// `0x2110`: SmartShift ratchet/freespin control (`SMART_SHIFT`).
    pub const SMART_SHIFT: Self = Self(0x2110);
    /// `0x2111`: SmartShift v2 (`SMART_SHIFT_V2`).
    pub const SMART_SHIFT_V2: Self = Self(0x2111);
    /// `0x2121`: High-resolution scroll wheel (`HIRES_WHEEL`).
    pub const HIRES_WHEEL: Self = Self(0x2121);
    /// `0x2201`: Adjustable DPI sensors and stages (`ADJUSTABLE_DPI`).
    pub const ADJUSTABLE_DPI: Self = Self(0x2201);
    /// `0x8060`: USB/gaming polling report rate (`REPORT_RATE`).
    pub const REPORT_RATE: Self = Self(0x8060);
    /// `0x8061`: Extended polling report rate up to 8 kHz (`EXTENDED_REPORT_RATE`).
    pub const EXTENDED_REPORT_RATE: Self = Self(0x8061);
    /// `0x8070`: Monochrome/single-zone color LED effects (`COLOR_LED_EFFECTS`).
    pub const COLOR_LED_EFFECTS: Self = Self(0x8070);
    /// `0x8071`: Multi-zone RGB LED effects (`RGB_EFFECTS`).
    pub const RGB_EFFECTS: Self = Self(0x8071);
    /// `0x8100`: Onboard profile memory management (`ONBOARD_PROFILES`).
    pub const ONBOARD_PROFILES: Self = Self(0x8100);

    /// Returns the raw 16-bit identifier.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns a human-readable symbolic name for documented standard features, if known.
    #[must_use]
    pub const fn known_name(self) -> Option<&'static str> {
        match self.0 {
            0x0000 => Some("IRoot"),
            0x0001 => Some("IFeatureSet"),
            0x0003 => Some("FIRMWARE_INFO"),
            0x0005 => Some("DEVICE_NAME"),
            0x0007 => Some("DEVICE_FRIENDLY_NAME"),
            0x0020 => Some("CONFIG_CHANGE"),
            0x0021 => Some("CRYPTO_ID"),
            0x1000 => Some("BATTERY_STATUS"),
            0x1004 => Some("UNIFIED_BATTERY"),
            0x1802 => Some("DEVICE_RESET"),
            0x1805 => Some("OOB_STATE"),
            0x1814 => Some("CHANGE_HOST"),
            0x1815 => Some("HOSTS_INFO"),
            0x1B04 => Some("REPROG_CONTROLS_V4"),
            0x1DF0 => Some("PERSISTENT_REMAPPABLE_ACTION"),
            0x2100 => Some("VERTICAL_SCROLLING"),
            0x2110 => Some("SMART_SHIFT"),
            0x2111 => Some("SMART_SHIFT_V2"),
            0x2120 => Some("HIRES_WHEEL_LEGACY"),
            0x2121 => Some("HIRES_WHEEL"),
            0x2200 => Some("MOUSE_POINTER"),
            0x2201 => Some("ADJUSTABLE_DPI"),
            0x2205 => Some("ANGLE_SNAPPING"),
            0x4531 => Some("DUAL_PLATFORM"),
            0x8010 => Some("GKEY"),
            0x8020 => Some("MKEYS"),
            0x8030 => Some("MR"),
            0x8060 => Some("REPORT_RATE"),
            0x8061 => Some("EXTENDED_REPORT_RATE"),
            0x8070 => Some("COLOR_LED_EFFECTS"),
            0x8071 => Some("RGB_EFFECTS"),
            0x8090 => Some("MODE_STATUS"),
            0x8100 => Some("ONBOARD_PROFILES"),
            0x8110 => Some("MOUSE_BUTTON_SPY"),
            _ => None,
        }
    }

    /// Returns the known symbolic feature name, or `"UNKNOWN"` if unrecognized.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self.known_name() {
            Some(name) => name,
            None => "UNKNOWN",
        }
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.known_name() {
            Some(name) => write!(f, "{name} (0x{:04X})", self.0),
            None => write!(f, "0x{:04X}", self.0),
        }
    }
}

/// Bitfield of feature metadata flags reported by `IRoot::getFeature` / `IFeatureSet::getFeatureID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FeatureFlags(u8);

impl FeatureFlags {
    /// Bit 7: Feature is obsolete.
    pub const OBSOLETE: u8 = 0x80;
    /// Bit 6: Feature is hidden from normal software discovery.
    pub const SW_HIDDEN: u8 = 0x40;
    /// Bit 5: Feature is an internal/engineering feature.
    pub const ENGINEERING: u8 = 0x20;
    /// Bit 4: Feature was deactivated during manufacturing.
    pub const MANUF_DEACTIVATED: u8 = 0x10;
    /// Bit 3: Feature was deactivated for compliance.
    pub const COMPLIANCE_DEACTIVATED: u8 = 0x08;

    /// Wraps raw wire feature type/flag byte.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    /// Returns the raw flags byte.
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Returns `true` if the obsolete bit (`0x80`) is set.
    #[must_use]
    pub const fn is_obsolete(self) -> bool {
        (self.0 & Self::OBSOLETE) != 0
    }

    /// Returns `true` if the software-hidden bit (`0x40`) is set.
    #[must_use]
    pub const fn is_hidden(self) -> bool {
        (self.0 & Self::SW_HIDDEN) != 0
    }

    /// Returns `true` if the engineering bit (`0x20`) is set.
    #[must_use]
    pub const fn is_engineering(self) -> bool {
        (self.0 & Self::ENGINEERING) != 0
    }
}

/// Single runtime-discovered feature entry in a device's HID++ 2.0 feature table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeatureEntry {
    /// Runtime table index (`0x00` for `IRoot`, `1..=count` for enumerated features).
    pub index: FeatureIndex,
    /// 16-bit feature ID (`0x0000`, `0x2201`, etc.).
    pub id: FeatureId,
    /// Feature revision reported by the firmware.
    pub version: u8,
    /// Feature visibility/type flags.
    pub flags: FeatureFlags,
}

/// HID++ 2.0+ protocol version reported by `IRoot::ping`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    /// Major protocol version (always `>= 2` for HID++ 2.0+ devices).
    pub major: u8,
    /// Minor protocol version.
    pub minor: u8,
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Metadata trait implemented by typed HID++ 2.0 feature drivers.
pub trait HidppFeature {
    /// 16-bit feature ID used for runtime lookup in [`FeatureTable`].
    const FEATURE_ID: FeatureId;
    /// Highest feature version understood by this implementation.
    const MAX_SUPPORTED_VERSION: u8;
}

/// Logs a warning when a device reports a higher revision of a known feature than Squeak supports.
///
/// Callers can inspect the returned boolean to degrade write operations to read-only if needed.
#[must_use]
pub fn check_feature_version(feature_id: FeatureId, version: u8, max_supported: u8) -> bool {
    if version > max_supported {
        warn!(
            feature = %feature_id,
            reported_version = version,
            max_supported_version = max_supported,
            "device implements newer feature version than supported; degrading gracefully"
        );
        false
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_known_and_unknown_feature_versions() {
        assert!(check_feature_version(FeatureId::ADJUSTABLE_DPI, 0, 2));
        assert!(check_feature_version(FeatureId::ADJUSTABLE_DPI, 2, 2));
        assert!(!check_feature_version(FeatureId::ADJUSTABLE_DPI, 3, 2));
        assert_eq!(FeatureId::ADJUSTABLE_DPI.display_name(), "ADJUSTABLE_DPI");
        assert_eq!(FeatureId(0x9999).display_name(), "UNKNOWN");
    }
}

//! Battery status drivers for `BATTERY_STATUS` (`0x1000`) and `UNIFIED_BATTERY` (`0x1004`).

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_LEGACY_GET_LEVEL_STATUS: u8 = 0;
const FN_LEGACY_GET_CAPABILITY: u8 = 1;

const FN_UNIFIED_GET_CAPABILITIES: u8 = 0;
const FN_UNIFIED_GET_STATUS: u8 = 1;

/// Validated battery percentage (`0..=100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatteryPercent(u8);

impl BatteryPercent {
    /// Constructs a validated [`BatteryPercent`] (`0..=100`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `value > 100`.
    pub fn new(value: u8) -> Result<Self, ProtocolError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(ProtocolError::InvalidArgument {
                detail: format!("battery percentage {value} out of bounds (expected 0..=100)"),
            })
        }
    }

    /// Parses a wire percentage byte (`0..=100`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedResponse`] if `value > 100`.
    pub fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(ProtocolError::UnexpectedResponse {
                detail: format!("battery percentage {value} exceeds 100%"),
            })
        }
    }

    /// Returns the raw percentage (`0..=100`).
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl fmt::Display for BatteryPercent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

/// Charging/power state reported by `BATTERY_STATUS` (`0x1000`) and `UNIFIED_BATTERY` (`0x1004`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatteryChargingStatus {
    /// `0x00`: Discharging on battery power.
    Discharging,
    /// `0x01`: Actively recharging.
    Recharging,
    /// `0x02`: Charge in final stage (nearly full).
    AlmostFull,
    /// `0x03`: Charge complete / full.
    Full,
    /// `0x04`: Recharging below optimal speed (slow charge).
    SlowRecharge,
    /// `0x05`: Invalid battery type detected.
    InvalidBattery,
    /// `0x06`: Thermal charging error.
    ThermalError,
    /// `0x07`: Other charging error.
    OtherChargingError,
    /// Unrecognized status byte from newer firmware.
    Unknown(u8),
}

impl BatteryChargingStatus {
    /// Parses a wire charging-status byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Discharging,
            0x01 => Self::Recharging,
            0x02 => Self::AlmostFull,
            0x03 => Self::Full,
            0x04 => Self::SlowRecharge,
            0x05 => Self::InvalidBattery,
            0x06 => Self::ThermalError,
            0x07 => Self::OtherChargingError,
            other => Self::Unknown(other),
        }
    }

    /// Returns the raw wire byte.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Discharging => 0x00,
            Self::Recharging => 0x01,
            Self::AlmostFull => 0x02,
            Self::Full => 0x03,
            Self::SlowRecharge => 0x04,
            Self::InvalidBattery => 0x05,
            Self::ThermalError => 0x06,
            Self::OtherChargingError => 0x07,
            Self::Unknown(b) => b,
        }
    }

    /// Returns a human-readable description.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Discharging => "discharging",
            Self::Recharging => "recharging",
            Self::AlmostFull => "almost-full",
            Self::Full => "full",
            Self::SlowRecharge => "slow-recharge",
            Self::InvalidBattery => "invalid-battery",
            Self::ThermalError => "thermal-error",
            Self::OtherChargingError => "charging-error",
            Self::Unknown(_) => "unknown",
        }
    }
}

impl fmt::Display for BatteryChargingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(b) => write!(f, "unknown (0x{b:02X})"),
            _ => f.write_str(self.label()),
        }
    }
}

/// Coarse battery bucket reported by `UNIFIED_BATTERY` (`0x1004`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatteryLevelBucket {
    /// `0x00`: Empty / unknown bucket.
    Empty,
    /// `0x01`: Critical battery level.
    Critical,
    /// `0x02`: Low battery level.
    Low,
    /// `0x04`: Good / medium battery level.
    Good,
    /// `0x08`: Full battery level.
    Full,
    /// Unrecognized bucket byte.
    Unknown(u8),
}

impl BatteryLevelBucket {
    /// Parses a wire level-bucket byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Empty,
            0x01 => Self::Critical,
            0x02 => Self::Low,
            0x04 => Self::Good,
            0x08 => Self::Full,
            other => Self::Unknown(other),
        }
    }

    /// Returns the raw wire byte.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Empty => 0x00,
            Self::Critical => 0x01,
            Self::Low => 0x02,
            Self::Good => 0x04,
            Self::Full => 0x08,
            Self::Unknown(b) => b,
        }
    }

    /// Approximate representative percentage for UI/CLI display when exact SoC is absent.
    #[must_use]
    pub const fn approximate_percent(self) -> Option<u8> {
        match self {
            Self::Empty => Some(0),
            Self::Critical => Some(5),
            Self::Low => Some(20),
            Self::Good => Some(50),
            Self::Full => Some(90),
            Self::Unknown(_) => None,
        }
    }
}

impl fmt::Display for BatteryLevelBucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::Critical => f.write_str("critical"),
            Self::Low => f.write_str("low"),
            Self::Good => f.write_str("good"),
            Self::Full => f.write_str("full"),
            Self::Unknown(b) => write!(f, "unknown (0x{b:02X})"),
        }
    }
}

/// Decoded `BATTERY_STATUS` (`0x1000`) level/status reply (`fn0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatusReply {
    /// Exact discharge percentage (`None` when device reports `0` / unknown).
    pub level: Option<BatteryPercent>,
    /// Next discharge threshold percentage at which an unsolicited notification fires.
    pub next_level: u8,
    /// Current charging/discharging state.
    pub charging_status: BatteryChargingStatus,
}

/// Decoded `BATTERY_STATUS` (`0x1000`) capability reply (`fn1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryCapability {
    /// Number of discrete levels reported (`2..=100`).
    pub number_of_levels: u8,
    /// Raw capability flags byte (bit 0: disable OSD, bit 1: rechargeable, bit 2: state-of-charge).
    pub flags: u8,
    /// Optional nominal battery life field.
    pub nominal_battery_life: Option<u16>,
    /// Optional critical percentage threshold.
    pub critical_level: Option<u8>,
}

impl BatteryCapability {
    /// Returns `true` if the battery is rechargeable.
    #[must_use]
    pub const fn is_rechargeable(self) -> bool {
        (self.flags & 0x02) != 0
    }
}

/// Decoded `UNIFIED_BATTERY` (`0x1004`) capability reply (`fn0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedBatteryCapabilities {
    /// Bitmask of supported coarse [`BatteryLevelBucket`] values.
    pub supported_levels: u8,
    /// Capability flags (bit 0: rechargeable, bit 1: exact state-of-charge supported).
    pub flags: u8,
}

impl UnifiedBatteryCapabilities {
    /// Returns `true` if exact 0–100% state-of-charge reporting is supported.
    #[must_use]
    pub const fn supports_state_of_charge(self) -> bool {
        (self.flags & 0x02) != 0
    }

    /// Returns `true` if the device battery is rechargeable.
    #[must_use]
    pub const fn is_rechargeable(self) -> bool {
        (self.flags & 0x01) != 0
    }
}

/// Decoded `UNIFIED_BATTERY` (`0x1004`) status reply (`fn1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedBatteryStatus {
    /// Exact state of charge (`None` when `0` / coarse bucket only).
    pub state_of_charge: Option<BatteryPercent>,
    /// Coarse level bucket (`Critical`, `Low`, `Good`, `Full`).
    pub level_bucket: BatteryLevelBucket,
    /// Charging status enum.
    pub charging_status: BatteryChargingStatus,
    /// External power indicator byte (`Some` on v1+ responses).
    pub external_power: Option<u8>,
}

/// Unified battery snapshot produced by either `UNIFIED_BATTERY` (`0x1004`) or
/// `BATTERY_STATUS` (`0x1000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading {
    /// Feature that produced this reading (`0x1004` or `0x1000`).
    pub source_feature: FeatureId,
    /// Exact percentage if reported by firmware.
    pub percentage: Option<BatteryPercent>,
    /// Coarse level bucket if reported (`UNIFIED_BATTERY`).
    pub level_bucket: Option<BatteryLevelBucket>,
    /// Next notification threshold (`BATTERY_STATUS`).
    pub next_level: Option<u8>,
    /// Charging/discharging status.
    pub charging_status: BatteryChargingStatus,
    /// External power indicator byte if reported.
    pub external_power: Option<u8>,
}

impl BatteryReading {
    /// Returns the exact percentage if available, otherwise the bucket approximation.
    #[must_use]
    pub const fn best_percent(self) -> Option<u8> {
        if let Some(pct) = self.percentage {
            Some(pct.as_u8())
        } else if let Some(bucket) = self.level_bucket {
            bucket.approximate_percent()
        } else {
            None
        }
    }
}

/// Trait implemented by battery-reporting HID++ 2.0 feature drivers.
pub trait BatteryFeature {
    /// Reads the current unified [`BatteryReading`] from the device.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn read_battery<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<BatteryReading, HidppError>;
}

/// Versioned runtime driver for `BATTERY_STATUS` (`0x1000`).
#[derive(Debug, Clone, Copy)]
pub struct BatteryStatusDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for BatteryStatusDriver {
    const FEATURE_ID: FeatureId = FeatureId::BATTERY_STATUS;
    const MAX_SUPPORTED_VERSION: u8 = 1;
}

impl BatteryStatusDriver {
    /// Creates a driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::BATTERY_STATUS`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected BATTERY_STATUS (0x1000), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Creates a driver directly from runtime feature index and version.
    #[must_use]
    pub fn new(feature_index: FeatureIndex, version: u8) -> Self {
        let _supported =
            check_feature_version(Self::FEATURE_ID, version, Self::MAX_SUPPORTED_VERSION);
        Self {
            feature_index,
            version,
        }
    }

    /// Returns the runtime feature index.
    #[must_use]
    pub const fn feature_index(&self) -> FeatureIndex {
        self.feature_index
    }

    /// Returns the firmware-reported feature version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// Encodes `getBatteryLevelStatus` (`fn0`) parameters.
    #[must_use]
    pub const fn encode_get_level_status_params() -> [u8; 0] {
        []
    }

    /// Encodes `getBatteryCapability` (`fn1`) parameters.
    #[must_use]
    pub const fn encode_get_capability_params() -> [u8; 0] {
        []
    }

    /// Pure parser for `getBatteryLevelStatus` (`fn0`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes exist, or
    /// [`ProtocolError::UnexpectedResponse`] if percentage exceeds `100`.
    pub fn parse_level_status_response(params: &[u8]) -> Result<BatteryStatusReply, ProtocolError> {
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let raw_level = params[0];
        let level = if raw_level == 0 {
            None
        } else {
            Some(BatteryPercent::from_wire(raw_level)?)
        };
        Ok(BatteryStatusReply {
            level,
            next_level: params[1],
            charging_status: BatteryChargingStatus::from_byte(params[2]),
        })
    }

    /// Pure parser for `getBatteryCapability` (`fn1`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 2 parameter bytes exist.
    pub fn parse_capability_response(params: &[u8]) -> Result<BatteryCapability, ProtocolError> {
        if params.len() < 2 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: params.len(),
            });
        }
        let nominal_battery_life = match (params.get(2), params.get(3)) {
            (Some(&msb), Some(&lsb)) => Some(u16::from_be_bytes([msb, lsb])),
            _ => None,
        };
        Ok(BatteryCapability {
            number_of_levels: params[0],
            flags: params[1],
            nominal_battery_life,
            critical_level: params.get(4).copied(),
        })
    }

    /// Queries `getBatteryLevelStatus` (`fn0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol failures.
    pub fn get_level_status<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<BatteryStatusReply, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_LEGACY_GET_LEVEL_STATUS)?,
            &Self::encode_get_level_status_params(),
        )?;
        Ok(Self::parse_level_status_response(reply.params())?)
    }

    /// Queries `getBatteryCapability` (`fn1`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol failures.
    pub fn get_capability<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<BatteryCapability, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_LEGACY_GET_CAPABILITY)?,
            &Self::encode_get_capability_params(),
        )?;
        Ok(Self::parse_capability_response(reply.params())?)
    }
}

impl BatteryFeature for BatteryStatusDriver {
    fn read_battery<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<BatteryReading, HidppError> {
        let reply = self.get_level_status(session, device_index)?;
        Ok(BatteryReading {
            source_feature: Self::FEATURE_ID,
            percentage: reply.level,
            level_bucket: None,
            next_level: Some(reply.next_level),
            charging_status: reply.charging_status,
            external_power: None,
        })
    }
}

/// Versioned runtime driver for `UNIFIED_BATTERY` (`0x1004`).
#[derive(Debug, Clone, Copy)]
pub struct UnifiedBatteryDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for UnifiedBatteryDriver {
    const FEATURE_ID: FeatureId = FeatureId::UNIFIED_BATTERY;
    const MAX_SUPPORTED_VERSION: u8 = 5;
}

impl UnifiedBatteryDriver {
    /// Creates a driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::UNIFIED_BATTERY`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected UNIFIED_BATTERY (0x1004), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Creates a driver directly from runtime feature index and version.
    #[must_use]
    pub fn new(feature_index: FeatureIndex, version: u8) -> Self {
        let _supported =
            check_feature_version(Self::FEATURE_ID, version, Self::MAX_SUPPORTED_VERSION);
        Self {
            feature_index,
            version,
        }
    }

    /// Returns the runtime feature index.
    #[must_use]
    pub const fn feature_index(&self) -> FeatureIndex {
        self.feature_index
    }

    /// Returns the firmware-reported feature version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// Encodes `getCapabilities` (`fn0`) parameters.
    #[must_use]
    pub const fn encode_get_capabilities_params() -> [u8; 0] {
        []
    }

    /// Encodes `getStatus` (`fn1`) parameters.
    #[must_use]
    pub const fn encode_get_status_params() -> [u8; 0] {
        []
    }

    /// Pure parser for `getCapabilities` (`fn0`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 2 parameter bytes exist.
    pub fn parse_capabilities_response(
        params: &[u8],
    ) -> Result<UnifiedBatteryCapabilities, ProtocolError> {
        if params.len() < 2 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: params.len(),
            });
        }
        Ok(UnifiedBatteryCapabilities {
            supported_levels: params[0],
            flags: params[1],
        })
    }

    /// Pure parser for `getStatus` (`fn1`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes exist, or
    /// [`ProtocolError::UnexpectedResponse`] if `state_of_charge` exceeds `100`.
    pub fn parse_status_response(params: &[u8]) -> Result<UnifiedBatteryStatus, ProtocolError> {
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let raw_soc = params[0];
        let state_of_charge = if raw_soc == 0 {
            None
        } else {
            Some(BatteryPercent::from_wire(raw_soc)?)
        };
        Ok(UnifiedBatteryStatus {
            state_of_charge,
            level_bucket: BatteryLevelBucket::from_byte(params[1]),
            charging_status: BatteryChargingStatus::from_byte(params[2]),
            external_power: params.get(3).copied(),
        })
    }

    /// Queries `getCapabilities` (`fn0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol failures.
    pub fn get_capabilities<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<UnifiedBatteryCapabilities, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_UNIFIED_GET_CAPABILITIES)?,
            &Self::encode_get_capabilities_params(),
        )?;
        Ok(Self::parse_capabilities_response(reply.params())?)
    }

    /// Queries `getStatus` (`fn1`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol failures.
    pub fn get_status<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<UnifiedBatteryStatus, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_UNIFIED_GET_STATUS)?,
            &Self::encode_get_status_params(),
        )?;
        Ok(Self::parse_status_response(reply.params())?)
    }
}

impl BatteryFeature for UnifiedBatteryDriver {
    fn read_battery<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<BatteryReading, HidppError> {
        let status = self.get_status(session, device_index)?;
        Ok(BatteryReading {
            source_feature: Self::FEATURE_ID,
            percentage: status.state_of_charge,
            level_bucket: Some(status.level_bucket),
            next_level: None,
            charging_status: status.charging_status,
            external_power: status.external_power,
        })
    }
}

/// Reads battery status using `UNIFIED_BATTERY` (`0x1004`) if present, falling back to
/// `BATTERY_STATUS` (`0x1000`), or returning `Ok(None)` for wired devices without battery features.
///
/// # Errors
///
/// Propagates any [`HidppError`] from the underlying battery query.
pub fn read_battery<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<BatteryReading>, HidppError> {
    if let Some(entry) = table.get(FeatureId::UNIFIED_BATTERY) {
        let driver = UnifiedBatteryDriver::from_entry(entry)?;
        return driver.read_battery(session, table.device_index()).map(Some);
    }
    if let Some(entry) = table.get(FeatureId::BATTERY_STATUS) {
        let driver = BatteryStatusDriver::from_entry(entry)?;
        return driver.read_battery(session, table.device_index()).map(Some);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;
    use crate::feature::{FeatureFlags, ProtocolVersion};
    use crate::frame::{HidppFrame, ReportId, SwId};
    use crate::session::SessionConfig;
    use crate::transport::{HidDeviceInfo, MockTransport};

    fn test_session(mock: MockTransport) -> HidppSession<MockTransport> {
        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        HidppSession::with_config(mock, cfg)
    }

    #[test]
    fn golden_battery_status_and_unified_battery() {
        let info = HidDeviceInfo::mock(0x046D, 0xC548, "Wireless Mouse");
        let mut mock = MockTransport::new(info);
        // 0x1000 getBatteryLevelStatus -> 75%, next 50%, discharging (0x00)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(0x05),
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[75, 50, 0x00],
            )
            .unwrap(),
        );
        // 0x1004 getStatus -> 88%, bucket Good (0x04), charging (0x01), ext_power=1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(0x06),
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[88, 0x04, 0x01, 0x01],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let legacy_table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 2 },
            vec![FeatureEntry {
                index: FeatureIndex(0x05),
                id: FeatureId::BATTERY_STATUS,
                version: 1,
                flags: FeatureFlags::default(),
            }],
        );
        let r1 = read_battery(&mut session, &legacy_table).unwrap().unwrap();
        assert_eq!(r1.source_feature, FeatureId::BATTERY_STATUS);
        assert_eq!(r1.best_percent(), Some(75));
        assert_eq!(r1.next_level, Some(50));
        assert_eq!(r1.charging_status, BatteryChargingStatus::Discharging);

        let unified_table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 5 },
            vec![FeatureEntry {
                index: FeatureIndex(0x06),
                id: FeatureId::UNIFIED_BATTERY,
                version: 3,
                flags: FeatureFlags::default(),
            }],
        );
        let r2 = read_battery(&mut session, &unified_table).unwrap().unwrap();
        assert_eq!(r2.source_feature, FeatureId::UNIFIED_BATTERY);
        assert_eq!(r2.best_percent(), Some(88));
        assert_eq!(r2.level_bucket, Some(BatteryLevelBucket::Good));
        assert_eq!(r2.charging_status, BatteryChargingStatus::Recharging);
        assert_eq!(r2.external_power, Some(1));

        // Wired device without battery features returns Ok(None)
        let wired_table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 2 },
            Vec::new(),
        );
        assert!(read_battery(&mut session, &wired_table).unwrap().is_none());
    }

    #[test]
    fn rejects_invalid_percentages_and_truncated_battery_frames() {
        assert!(matches!(
            BatteryStatusDriver::parse_level_status_response(&[101, 0, 0]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
        assert!(matches!(
            UnifiedBatteryDriver::parse_status_response(&[105, 8, 0]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
        assert!(matches!(
            BatteryStatusDriver::parse_level_status_response(&[50, 20]),
            Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: 2
            })
        ));
        assert!(matches!(
            UnifiedBatteryDriver::parse_capabilities_response(&[0x0F]),
            Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: 1
            })
        ));
    }

    proptest! {
        #[test]
        fn battery_status_roundtrip(
            level in 0u8..=100,
            next_level in 0u8..=100,
            status in any::<u8>(),
        ) {
            let parsed = BatteryStatusDriver::parse_level_status_response(&[level, next_level, status]).unwrap();
            prop_assert_eq!(parsed.level.map(BatteryPercent::as_u8), (level > 0).then_some(level));
            prop_assert_eq!(parsed.next_level, next_level);
            prop_assert_eq!(parsed.charging_status.as_byte(), status);
        }

        #[test]
        fn unified_battery_roundtrip(
            soc in 0u8..=100,
            bucket in any::<u8>(),
            status in any::<u8>(),
            ext in any::<u8>(),
        ) {
            let parsed = UnifiedBatteryDriver::parse_status_response(&[soc, bucket, status, ext]).unwrap();
            prop_assert_eq!(parsed.state_of_charge.map(BatteryPercent::as_u8), (soc > 0).then_some(soc));
            prop_assert_eq!(parsed.level_bucket.as_byte(), bucket);
            prop_assert_eq!(parsed.charging_status.as_byte(), status);
            prop_assert_eq!(parsed.external_power, Some(ext));
        }
    }
}

//! `FIRMWARE_INFO` (`0x0003`) feature driver for device metadata and firmware entity versions.
//!
//! Privacy invariant: per-unit identifier bytes (`unitId` in `getDeviceInfo` v1+) are
//! explicitly dropped during parsing and are never retained, logged, or serialized.

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_DEVICE_INFO: u8 = 0;
const FN_GET_FW_INFO: u8 = 1;

/// Supported transport protocol bits advertised by `FIRMWARE_INFO::getDeviceInfo` (v1+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TransportCapabilities(u16);

impl TransportCapabilities {
    /// Bit 0 (`0x0001`): Bluetooth Classic.
    pub const BLUETOOTH: u16 = 0x0001;
    /// Bit 1 (`0x0002`): Bluetooth Low Energy (BLE).
    pub const BLE: u16 = 0x0002;
    /// Bit 2 (`0x0004`): Logitech eQuad / Unifying / Lightspeed RF.
    pub const EQUAD: u16 = 0x0004;
    /// Bit 3 (`0x0008`): USB wired transport.
    pub const USB: u16 = 0x0008;

    /// Wraps raw 16-bit transport bitfield.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Returns the raw 16-bit transport bitfield.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Returns `true` if Bluetooth Classic is supported.
    #[must_use]
    pub const fn bluetooth(self) -> bool {
        (self.0 & Self::BLUETOOTH) != 0
    }

    /// Returns `true` if Bluetooth Low Energy is supported.
    #[must_use]
    pub const fn ble(self) -> bool {
        (self.0 & Self::BLE) != 0
    }

    /// Returns `true` if eQuad/Unifying/Lightspeed RF is supported.
    #[must_use]
    pub const fn equad(self) -> bool {
        (self.0 & Self::EQUAD) != 0
    }

    /// Returns `true` if USB wired transport is supported.
    #[must_use]
    pub const fn usb(self) -> bool {
        (self.0 & Self::USB) != 0
    }
}

/// Sanitized device-level summary returned by `FIRMWARE_INFO::getDeviceInfo` (`fn0`).
///
/// Note: per-unit `unitId` bytes are intentionally omitted and never collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFirmwareSummary {
    /// Total number of firmware/hardware entities (`0..entity_count`).
    pub entity_count: u8,
    /// Multi-transport capability bitfield (`Some` on `0x0003` v1+).
    pub transports: Option<TransportCapabilities>,
    /// Per-transport model PIDs parsed from `modelId` (`0x0003` v1+).
    pub model_pids: Vec<u16>,
    /// Extended capability byte reported by `0x0003` v2+ (`None` on v0/v1).
    pub capabilities: Option<u8>,
}

/// Classification of an entity returned by `FIRMWARE_INFO::getFwInfo` (`fn1`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FirmwareEntityKind {
    /// `0`: Main application firmware.
    MainApplication,
    /// `1`: Bootloader (DFU) firmware.
    Bootloader,
    /// `2`: Hardware revision entry.
    Hardware,
    /// Other/vendor-specific entity type nibble.
    Other(u8),
}

impl FirmwareEntityKind {
    /// Parses the low 4-bit entity kind from `getFwInfo` byte 0.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        match raw & 0x0F {
            0 => Self::MainApplication,
            1 => Self::Bootloader,
            2 => Self::Hardware,
            other => Self::Other(other),
        }
    }

    /// Returns a human-readable label for display.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MainApplication => "Main application",
            Self::Bootloader => "Bootloader",
            Self::Hardware => "Hardware",
            Self::Other(_) => "Other",
        }
    }
}

impl fmt::Display for FirmwareEntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(code) => write!(f, "Other (0x{code:02X})"),
            _ => f.write_str(self.label()),
        }
    }
}

/// Parsed firmware or hardware entity returned by `FIRMWARE_INFO::getFwInfo` (`fn1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareEntity {
    /// Queried entity index (`0..entity_count`).
    pub index: u8,
    /// Entity classification (`MainApplication`, `Bootloader`, `Hardware`, ...).
    pub kind: FirmwareEntityKind,
    /// Firmware class ASCII prefix (e.g., `"U1"`, `"MPM"`, `"BOT"`; empty for hardware entries).
    pub prefix: String,
    /// Packed BCD firmware number (for example `0x12` representing `12`).
    pub fw_number: u8,
    /// Packed BCD revision byte (for example `0x03` representing `03`).
    pub revision: u8,
    /// 16-bit build number.
    pub build: u16,
    /// Whether this firmware entity is the currently active responder.
    pub active: bool,
    /// Transport-specific PID reported for this entity (`0` when omitted).
    pub transport_pid: u16,
    /// Hardware revision byte when `kind == FirmwareEntityKind::Hardware`.
    pub hardware_revision: Option<u8>,
}

impl FirmwareEntity {
    /// Formats a clean human-readable version string (e.g. `"MPM 12.03.B0019"` or `"rev 4"`).
    #[must_use]
    pub fn display_version(&self) -> String {
        match self.kind {
            FirmwareEntityKind::Hardware => match self.hardware_revision {
                Some(rev) => format!("rev {rev}"),
                None => "hardware".to_owned(),
            },
            FirmwareEntityKind::MainApplication | FirmwareEntityKind::Bootloader => {
                let base = if self.prefix.is_empty() {
                    format!("{:02X}.{:02X}", self.fw_number, self.revision)
                } else {
                    format!(
                        "{} {:02X}.{:02X}",
                        self.prefix, self.fw_number, self.revision
                    )
                };
                if self.build != 0 {
                    format!("{base}.B{:04X}", self.build)
                } else {
                    base
                }
            }
            FirmwareEntityKind::Other(code) => format!("type 0x{code:02X}"),
        }
    }
}

/// Complete firmware inventory read from `FIRMWARE_INFO` (`0x0003`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareInventory {
    /// Device-level summary (entity count, transports, model PIDs).
    pub summary: DeviceFirmwareSummary,
    /// All enumerated firmware/hardware entities (`0..entity_count`).
    pub entities: Vec<FirmwareEntity>,
}

/// Trait implemented by versioned `FIRMWARE_INFO` (`0x0003`) drivers.
pub trait FirmwareInfoFeature {
    /// Queries device summary (`fn0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_device_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<DeviceFirmwareSummary, HidppError>;

    /// Queries metadata for a single firmware/hardware entity (`fn1`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_fw_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        entity_index: u8,
    ) -> Result<FirmwareEntity, HidppError>;

    /// Reads the device summary and iterates through all `0..entity_count` entities.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn read_inventory<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<FirmwareInventory, HidppError> {
        let summary = self.get_device_info(session, device_index)?;
        let mut entities = Vec::with_capacity(usize::from(summary.entity_count));
        for idx in 0..summary.entity_count {
            entities.push(self.get_fw_info(session, device_index, idx)?);
        }
        Ok(FirmwareInventory { summary, entities })
    }
}

/// Versioned runtime driver for `FIRMWARE_INFO` (`0x0003`).
#[derive(Debug, Clone, Copy)]
pub struct FirmwareInfoDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for FirmwareInfoDriver {
    const FEATURE_ID: FeatureId = FeatureId::FIRMWARE_INFO;
    const MAX_SUPPORTED_VERSION: u8 = 4;
}

impl FirmwareInfoDriver {
    /// Creates a versioned driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::FIRMWARE_INFO`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected FIRMWARE_INFO (0x0003), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Creates a driver directly from a runtime feature index and firmware version.
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

    /// Encodes request parameters for `getDeviceInfo` (`fn0`).
    #[must_use]
    pub const fn encode_get_device_info_params() -> [u8; 0] {
        []
    }

    /// Encodes request parameters for `getFwInfo(entity_idx)` (`fn1`).
    #[must_use]
    pub const fn encode_get_fw_info_params(entity_index: u8) -> [u8; 1] {
        [entity_index]
    }

    /// Pure parser for `getDeviceInfo` (`fn0`) response parameters, aware of `feature_version`.
    ///
    /// Privacy guarantee: `unitId` bytes `[1..5]` in v1+ are intentionally discarded.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than the minimum version-specific
    /// parameter bytes are present.
    pub fn parse_device_info_response(
        feature_version: u8,
        params: &[u8],
    ) -> Result<DeviceFirmwareSummary, ProtocolError> {
        let Some(&entity_count) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };

        if feature_version == 0 {
            return Ok(DeviceFirmwareSummary {
                entity_count,
                transports: None,
                model_pids: Vec::new(),
                capabilities: None,
            });
        }

        if params.len() < 13 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 13,
                actual: params.len(),
            });
        }

        // Bytes [1..5] contain per-unit `unitId`; intentionally skipped to avoid collecting serial IDs.
        let transport_raw = u16::from_be_bytes([params[5], params[6]]);
        let transports = TransportCapabilities::from_raw(transport_raw);

        // Up to 3 simultaneous transports packed sequentially into bytes [7..13].
        let set_transports = usize::try_from(transport_raw.count_ones()).unwrap_or(0);
        let pid_slots = set_transports.min(3);
        let mut model_pids = Vec::with_capacity(pid_slots);
        for slot in 0..pid_slots {
            let offset = 7 + slot * 2;
            let pid = u16::from_be_bytes([params[offset], params[offset + 1]]);
            if pid != 0 {
                model_pids.push(pid);
            }
        }

        let capabilities = if feature_version >= 2 {
            params.get(13).copied()
        } else {
            None
        };

        Ok(DeviceFirmwareSummary {
            entity_count,
            transports: Some(transports),
            model_pids,
            capabilities,
        })
    }

    /// Pure parser for `getFwInfo` (`fn1`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if required parameter bytes are missing.
    pub fn parse_fw_info_response(
        entity_index: u8,
        params: &[u8],
    ) -> Result<FirmwareEntity, ProtocolError> {
        let Some(&raw_type) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: 0,
            });
        };

        let kind = FirmwareEntityKind::from_raw(raw_type);
        match kind {
            FirmwareEntityKind::Hardware => {
                let Some(&hw_rev) = params.get(1) else {
                    return Err(ProtocolError::ResponseTruncated {
                        expected: 2,
                        actual: params.len(),
                    });
                };
                Ok(FirmwareEntity {
                    index: entity_index,
                    kind,
                    prefix: String::new(),
                    fw_number: 0,
                    revision: 0,
                    build: 0,
                    active: false,
                    transport_pid: 0,
                    hardware_revision: Some(hw_rev),
                })
            }
            FirmwareEntityKind::MainApplication | FirmwareEntityKind::Bootloader => {
                if params.len() < 8 {
                    return Err(ProtocolError::ResponseTruncated {
                        expected: 8,
                        actual: params.len(),
                    });
                }
                let prefix_bytes = &params[1..4];
                let prefix = prefix_bytes
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| {
                        if b.is_ascii_graphic() || b == b' ' {
                            char::from(b)
                        } else {
                            '?'
                        }
                    })
                    .collect::<String>()
                    .trim()
                    .to_owned();

                let fw_number = params[4];
                let revision = params[5];
                let build = u16::from_be_bytes([params[6], params[7]]);
                let active = params.get(8).is_some_and(|&b| (b & 0x01) != 0);
                let transport_pid = match (params.get(9), params.get(10)) {
                    (Some(&msb), Some(&lsb)) => u16::from_be_bytes([msb, lsb]),
                    _ => 0,
                };

                Ok(FirmwareEntity {
                    index: entity_index,
                    kind,
                    prefix,
                    fw_number,
                    revision,
                    build,
                    active,
                    transport_pid,
                    hardware_revision: None,
                })
            }
            FirmwareEntityKind::Other(_) => Ok(FirmwareEntity {
                index: entity_index,
                kind,
                prefix: String::new(),
                fw_number: 0,
                revision: 0,
                build: 0,
                active: params.get(8).is_some_and(|&b| (b & 0x01) != 0),
                transport_pid: 0,
                hardware_revision: None,
            }),
        }
    }
}

impl FirmwareInfoFeature for FirmwareInfoDriver {
    fn get_device_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<DeviceFirmwareSummary, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_DEVICE_INFO)?,
            &Self::encode_get_device_info_params(),
        )?;
        Ok(Self::parse_device_info_response(
            self.version,
            reply.params(),
        )?)
    }

    fn get_fw_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        entity_index: u8,
    ) -> Result<FirmwareEntity, HidppError> {
        let params = Self::encode_get_fw_info_params(entity_index);
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_FW_INFO)?,
            &params,
        )?;
        Ok(Self::parse_fw_info_response(entity_index, reply.params())?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;
    use crate::frame::{HidppFrame, LONG_PARAM_LEN, ReportId, SwId};
    use crate::session::SessionConfig;
    use crate::transport::{HidDeviceInfo, MockTransport};

    #[test]
    fn golden_firmware_info_v2_and_unit_id_redaction() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x03);

        // fn0 getDeviceInfo -> 2 entities, unitId [0xAA,0xBB,0xCC,0xDD] (must be dropped!),
        // transport 0x0008 (USB), modelId [0xC0, 0x8B], capabilities 0x01
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0x08, 0xC0, 0x8B, 0x00, 0x00, 0x00, 0x00,
                    0x01, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );

        // fn1(0) -> MainApplication MPM 12.01 build 0x0009, active=1, trPid=0xC08B
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x00, b'M', b'P', b'M', 0x12, 0x01, 0x00, 0x09, 0x01, 0xC0, 0x8B, 0x00, 0x00,
                    0x00, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );

        // fn1(1) -> Bootloader BOT 03.00 build 0x0001, active=0, trPid=0xC08B
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x01, b'B', b'O', b'T', 0x03, 0x00, 0x00, 0x01, 0x00, 0xC0, 0x8B, 0x00, 0x00,
                    0x00, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );

        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        let mut session = HidppSession::with_config(mock, cfg);
        let driver = FirmwareInfoDriver::new(feat_idx, 2);

        let inv = driver
            .read_inventory(&mut session, DeviceIndex::DIRECT)
            .unwrap();
        assert_eq!(inv.summary.entity_count, 2);
        assert!(inv.summary.transports.unwrap().usb());
        assert_eq!(inv.summary.model_pids, vec![0xC08B]);
        assert_eq!(inv.summary.capabilities, Some(0x01));
        assert!(!format!("{inv:?}").contains("AABBCCDD"));

        assert_eq!(inv.entities.len(), 2);
        assert_eq!(inv.entities[0].kind, FirmwareEntityKind::MainApplication);
        assert_eq!(inv.entities[0].display_version(), "MPM 12.01.B0009");
        assert!(inv.entities[0].active);
        assert_eq!(inv.entities[0].transport_pid, 0xC08B);

        assert_eq!(inv.entities[1].kind, FirmwareEntityKind::Bootloader);
        assert_eq!(inv.entities[1].display_version(), "BOT 03.00.B0001");
        assert!(!inv.entities[1].active);
    }

    #[test]
    fn parses_v0_device_info_and_hardware_entity_and_rejects_truncated() {
        let v0 = FirmwareInfoDriver::parse_device_info_response(0, &[3]).unwrap();
        assert_eq!(v0.entity_count, 3);
        assert!(v0.transports.is_none());
        assert!(v0.model_pids.is_empty());

        let hw = FirmwareInfoDriver::parse_fw_info_response(2, &[0x02, 0x04]).unwrap();
        assert_eq!(hw.kind, FirmwareEntityKind::Hardware);
        assert_eq!(hw.display_version(), "rev 4");

        assert!(matches!(
            FirmwareInfoDriver::parse_device_info_response(1, &[0x01, 0x02]),
            Err(ProtocolError::ResponseTruncated {
                expected: 13,
                actual: 2
            })
        ));
        assert!(matches!(
            FirmwareInfoDriver::parse_fw_info_response(0, &[0x00, b'A', b'B']),
            Err(ProtocolError::ResponseTruncated {
                expected: 8,
                actual: 3
            })
        ));
    }

    proptest! {
        #[test]
        fn fw_info_roundtrip(
            entity_idx in any::<u8>(),
            c0 in b'A'..=b'Z',
            c1 in b'A'..=b'Z',
            c2 in b'A'..=b'Z',
            fw_num in any::<u8>(),
            rev in any::<u8>(),
            build in any::<u16>(),
            active in any::<bool>(),
            tr_pid in any::<u16>(),
        ) {
            let [b_hi, b_lo] = build.to_be_bytes();
            let [p_hi, p_lo] = tr_pid.to_be_bytes();
            let mut params = [0u8; LONG_PARAM_LEN];
            params[0] = 0x00; // MainApplication
            params[1] = c0;
            params[2] = c1;
            params[3] = c2;
            params[4] = fw_num;
            params[5] = rev;
            params[6] = b_hi;
            params[7] = b_lo;
            params[8] = u8::from(active);
            params[9] = p_hi;
            params[10] = p_lo;

            let parsed = FirmwareInfoDriver::parse_fw_info_response(entity_idx, &params).unwrap();
            prop_assert_eq!(parsed.index, entity_idx);
            prop_assert_eq!(parsed.kind, FirmwareEntityKind::MainApplication);
            prop_assert_eq!(parsed.fw_number, fw_num);
            prop_assert_eq!(parsed.revision, rev);
            prop_assert_eq!(parsed.build, build);
            prop_assert_eq!(parsed.active, active);
            prop_assert_eq!(parsed.transport_pid, tr_pid);
        }
    }
}

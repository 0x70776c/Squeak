//! `REPROG_CONTROLS_V4` (`0x1B04`) feature driver.
//!
//! Enables enumerating remappable physical controls and buttons, querying capabilities,
//! and setting button diversions or remappings.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_CONTROL_COUNT: u8 = 0;
const FN_GET_CONTROL_INFO: u8 = 1;
const FN_GET_CONTROL_REPORTING: u8 = 2;
const FN_SET_CONTROL_REPORTING: u8 = 3;

/// Control Identifier (CID) for physical or logical controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControlId(pub u16);

impl ControlId {
    /// Left click button.
    pub const LEFT_CLICK: Self = Self(0x0050);
    /// Right click button.
    pub const RIGHT_CLICK: Self = Self(0x0051);
    /// Middle click button.
    pub const MIDDLE_CLICK: Self = Self(0x0052);
    /// Back / Previous button.
    pub const BACK: Self = Self(0x0053);
    /// Forward / Next button.
    pub const FORWARD: Self = Self(0x0056);

    /// Raw 16-bit identifier.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

/// Metadata describing a physical button or control on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInfo {
    /// Zero-based control index.
    pub index: u8,
    /// Standard Control Identifier (CID).
    pub cid: ControlId,
    /// Associated default Task ID.
    pub task_id: u16,
    /// Flags (e.g. divertable, remappable, raw XY, etc.).
    pub flags: u8,
    /// Physical location / position code.
    pub position: u8,
    /// Physical group index.
    pub group: u8,
}

/// Control reporting / remap configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlReporting {
    /// Zero-based control index.
    pub index: u8,
    /// Flags indicating diversion or software control.
    pub flags: u8,
    /// Remapped Control Identifier (CID), if any.
    pub remapped_cid: ControlId,
}

/// Driver for `REPROG_CONTROLS_V4` (`0x1B04`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReprogControlsV4Driver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for ReprogControlsV4Driver {
    const FEATURE_ID: FeatureId = FeatureId::REPROG_CONTROLS_V4;
    const MAX_SUPPORTED_VERSION: u8 = 4;
}

impl ReprogControlsV4Driver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x1B04`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected REPROG_CONTROLS_V4 (0x1B04), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Discovers the driver from the session's feature table.
    #[must_use]
    pub fn from_table(table: &FeatureTable) -> Option<Self> {
        table
            .get(Self::FEATURE_ID)
            .and_then(|e| Self::from_entry(e).ok())
    }

    /// Returns the resolved feature index.
    #[must_use]
    pub const fn feature_index(&self) -> FeatureIndex {
        self.feature_index
    }

    /// Returns the runtime feature version reported by firmware.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// Queries the total number of remappable controls (`fn 0 getControlCount`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_control_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let function = FunctionId::new(FN_GET_CONTROL_COUNT)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "REPROG_CONTROLS_V4 getControlCount returned empty parameters".into(),
            }));
        }
        Ok(params[0])
    }

    /// Queries information about a specific control index (`fn 1 getControlInfo`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_control_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        control_index: u8,
    ) -> Result<ControlInfo, HidppError> {
        let function = FunctionId::new(FN_GET_CONTROL_INFO)?;
        let reply =
            session.request(device_index, self.feature_index, function, &[control_index])?;
        let params = reply.params();
        if params.len() < 7 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "REPROG_CONTROLS_V4 getControlInfo returned {} bytes (expected >= 7)",
                    params.len()
                ),
            }));
        }
        let cid = ControlId(u16::from_be_bytes([params[0], params[1]]));
        let task_id = u16::from_be_bytes([params[2], params[3]]);
        let flags = params[4];
        let position = params[5];
        let group = params[6];
        Ok(ControlInfo {
            index: control_index,
            cid,
            task_id,
            flags,
            position,
            group,
        })
    }

    /// Queries current reporting / remap status for a control index (`fn 2 getControlReporting`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_control_reporting<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        control_index: u8,
    ) -> Result<ControlReporting, HidppError> {
        let function = FunctionId::new(FN_GET_CONTROL_REPORTING)?;
        let reply =
            session.request(device_index, self.feature_index, function, &[control_index])?;
        let params = reply.params();
        if params.len() < 4 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "REPROG_CONTROLS_V4 getControlReporting returned {} bytes (expected >= 4)",
                    params.len()
                ),
            }));
        }
        let flags = params[1];
        let remapped_cid = ControlId(u16::from_be_bytes([params[2], params[3]]));
        Ok(ControlReporting {
            index: control_index,
            flags,
            remapped_cid,
        })
    }

    /// Sets control reporting / remap for a control index (`fn 3 setControlReporting`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_control_reporting<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        control_index: u8,
        flags: u8,
        remapped_cid: ControlId,
    ) -> Result<(), HidppError> {
        let cid_bytes = remapped_cid.as_u16().to_be_bytes();
        let payload = [control_index, flags, cid_bytes[0], cid_bytes[1]];
        let function = FunctionId::new(FN_SET_CONTROL_REPORTING)?;
        let _ = session.request(device_index, self.feature_index, function, &payload)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
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
    fn reprog_controls_v4_enumeration_and_remap() {
        let feat_idx = FeatureIndex(0x04);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::REPROG_CONTROLS_V4,
            version: 4,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = ReprogControlsV4Driver::from_entry(&entry).unwrap();
        assert_eq!(driver.version(), 4);

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Mouse");
        let mut mock = MockTransport::new(info);

        // fn 0 -> 5 controls
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x05, 0x00],
            )
            .unwrap(),
        );

        // fn 1 for control 0 -> Left Click CID 0x0050
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x50, 0x00, 0x01, 0x03, 0x01, 0x01],
            )
            .unwrap(),
        );

        // fn 3 setControlReporting for control 0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(3).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let count = driver.get_control_count(&mut session, dev_idx).unwrap();
        assert_eq!(count, 5);

        let info = driver.get_control_info(&mut session, dev_idx, 0).unwrap();
        assert_eq!(info.cid, ControlId::LEFT_CLICK);

        driver
            .set_control_reporting(&mut session, dev_idx, 0, 0x01, ControlId::RIGHT_CLICK)
            .unwrap();
    }
}

//! `SMART_SHIFT` (`0x2110`) feature driver.
//!
//! Controls automatic or manual mechanical/electronic scroll wheel mode switching (SmartShift).

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_STATUS: u8 = 0;
const FN_SET_MODE: u8 = 1;

/// SmartShift operational mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmartShiftMode {
    /// Auto SmartShift: shifts automatically between ratchet and freespin based on spin speed.
    Auto,
    /// Locked in tactile ratchet mode.
    AlwaysRatchet,
    /// Locked in free-spinning frictionless mode.
    AlwaysFreespin,
    /// Unrecognized mode.
    Other(u8),
}

impl From<u8> for SmartShiftMode {
    fn from(val: u8) -> Self {
        match val {
            0 => Self::Auto,
            1 => Self::AlwaysRatchet,
            2 => Self::AlwaysFreespin,
            other => Self::Other(other),
        }
    }
}

impl From<SmartShiftMode> for u8 {
    fn from(mode: SmartShiftMode) -> Self {
        match mode {
            SmartShiftMode::Auto => 0,
            SmartShiftMode::AlwaysRatchet => 1,
            SmartShiftMode::AlwaysFreespin => 2,
            SmartShiftMode::Other(val) => val,
        }
    }
}

/// SmartShift status and configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartShiftStatus {
    /// Current mode.
    pub mode: SmartShiftMode,
    /// Shift threshold sensitivity (`1..=255`).
    pub threshold: u8,
}

/// Driver for `SMART_SHIFT` (`0x2110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartShiftDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for SmartShiftDriver {
    const FEATURE_ID: FeatureId = FeatureId::SMART_SHIFT;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl SmartShiftDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x2110`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected SMART_SHIFT (0x2110), got {}", entry.id),
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

    /// Queries SmartShift status and threshold (`fn 0 getStatus`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_status<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<SmartShiftStatus, HidppError> {
        let function = FunctionId::new(FN_GET_STATUS)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 2 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "SMART_SHIFT getStatus returned {} bytes (expected >= 2)",
                    params.len()
                ),
            }));
        }
        let mode = SmartShiftMode::from(params[0]);
        let threshold = params[1];
        Ok(SmartShiftStatus { mode, threshold })
    }

    /// Configures SmartShift mode and threshold (`fn 1 setMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        mode: SmartShiftMode,
        threshold: u8,
    ) -> Result<(), HidppError> {
        let payload = [u8::from(mode), threshold];
        let function = FunctionId::new(FN_SET_MODE)?;
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
    fn smart_shift_query_and_set() {
        let feat_idx = FeatureIndex(0x07);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::SMART_SHIFT,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = SmartShiftDriver::from_entry(&entry).unwrap();

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "SmartShift Mouse");
        let mut mock = MockTransport::new(info);

        // fn 0 -> mode 0 (Auto), threshold 15
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x0F],
            )
            .unwrap(),
        );

        // fn 1 -> set mode 1 (AlwaysRatchet), threshold 20
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let status = driver.get_status(&mut session, dev_idx).unwrap();
        assert_eq!(status.mode, SmartShiftMode::Auto);
        assert_eq!(status.threshold, 15);

        driver
            .set_mode(&mut session, dev_idx, SmartShiftMode::AlwaysRatchet, 20)
            .unwrap();
    }
}

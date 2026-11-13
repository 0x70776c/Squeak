//! `HIRES_WHEEL` (`0x2121`) feature driver.
//!
//! Controls high-resolution scrolling mode and wheel direction inversion.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_WHEEL_CAPABILITY: u8 = 0;
const FN_GET_WHEEL_MODE: u8 = 1;
const FN_SET_WHEEL_MODE: u8 = 2;

/// High-resolution wheel capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelCapability {
    /// Scroll resolution multiplier.
    pub multiplier: u8,
    /// Whether direction can be inverted in hardware.
    pub has_invert: bool,
    /// Whether the wheel has a mechanical/electronic ratchet switch.
    pub has_ratchet: bool,
}

/// Wheel configuration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelMode {
    /// Invert scroll direction.
    pub inverted: bool,
    /// High-resolution scrolling enabled.
    pub hires_enabled: bool,
}

/// Driver for `HIRES_WHEEL` (`0x2121`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiresWheelDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for HiresWheelDriver {
    const FEATURE_ID: FeatureId = FeatureId::HIRES_WHEEL;
    const MAX_SUPPORTED_VERSION: u8 = 1;
}

impl HiresWheelDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x2121`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected HIRES_WHEEL (0x2121), got {}", entry.id),
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

    /// Queries wheel capabilities (`fn 0 getWheelCapability`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_wheel_capability<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<WheelCapability, HidppError> {
        let function = FunctionId::new(FN_GET_WHEEL_CAPABILITY)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 2 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "HIRES_WHEEL getWheelCapability returned {} bytes (expected >= 2)",
                    params.len()
                ),
            }));
        }
        let multiplier = params[0];
        let flags = params[1];
        let has_invert = (flags & 0x01) != 0;
        let has_ratchet = (flags & 0x02) != 0;
        Ok(WheelCapability {
            multiplier,
            has_invert,
            has_ratchet,
        })
    }

    /// Queries current wheel mode (`fn 1 getWheelMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_wheel_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<WheelMode, HidppError> {
        let function = FunctionId::new(FN_GET_WHEEL_MODE)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "HIRES_WHEEL getWheelMode returned empty parameters".into(),
            }));
        }
        let flags = params[0];
        let inverted = (flags & 0x01) != 0;
        let hires_enabled = (flags & 0x02) != 0;
        Ok(WheelMode {
            inverted,
            hires_enabled,
        })
    }

    /// Sets wheel mode (`fn 2 setWheelMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_wheel_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        mode: WheelMode,
    ) -> Result<(), HidppError> {
        let mut flags = 0u8;
        if mode.inverted {
            flags |= 0x01;
        }
        if mode.hires_enabled {
            flags |= 0x02;
        }
        let function = FunctionId::new(FN_SET_WHEEL_MODE)?;
        let _ = session.request(device_index, self.feature_index, function, &[flags])?;
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
    fn hires_wheel_query_and_set() {
        let feat_idx = FeatureIndex(0x06);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::HIRES_WHEEL,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = HiresWheelDriver::from_entry(&entry).unwrap();

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Wheel Mouse");
        let mut mock = MockTransport::new(info);

        // fn 0 capability -> mult 8, has invert + ratchet
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x08, 0x03],
            )
            .unwrap(),
        );

        // fn 1 mode -> inverted=false, hires=true (0x02)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00],
            )
            .unwrap(),
        );

        // fn 2 set mode -> inverted=true, hires=true (0x03)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let cap = driver.get_wheel_capability(&mut session, dev_idx).unwrap();
        assert_eq!(cap.multiplier, 8);
        assert!(cap.has_invert);
        assert!(cap.has_ratchet);

        let mode = driver.get_wheel_mode(&mut session, dev_idx).unwrap();
        assert!(!mode.inverted);
        assert!(mode.hires_enabled);

        driver
            .set_wheel_mode(
                &mut session,
                dev_idx,
                WheelMode {
                    inverted: true,
                    hires_enabled: true,
                },
            )
            .unwrap();
    }
}

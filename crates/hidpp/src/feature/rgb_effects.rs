//! `RGB_EFFECTS` (`0x8071`) feature driver.
//!
//! Controls multi-zone RGB effects on supported HID++ 2.0 gaming hardware.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_ZONE_COUNT: u8 = 0;
const FN_SET_EFFECT: u8 = 2;

/// Driver for `RGB_EFFECTS` (`0x8071`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbEffectsDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for RgbEffectsDriver {
    const FEATURE_ID: FeatureId = FeatureId::RGB_EFFECTS;
    const MAX_SUPPORTED_VERSION: u8 = 2;
}

impl RgbEffectsDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x8071`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected RGB_EFFECTS (0x8071), got {}", entry.id),
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

    /// Queries the total number of RGB zones (`fn 0 getZoneCount`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_zone_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let function = FunctionId::new(FN_GET_ZONE_COUNT)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "RGB_EFFECTS getZoneCount returned empty parameters".into(),
            }));
        }
        Ok(params[0])
    }

    /// Sets an RGB effect for a zone (`fn 2 setEffect`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_effect<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        zone: u8,
        effect_payload: &[u8],
    ) -> Result<(), HidppError> {
        let mut full_payload = Vec::with_capacity(1 + effect_payload.len());
        full_payload.push(zone);
        full_payload.extend_from_slice(effect_payload);
        let function = FunctionId::new(FN_SET_EFFECT)?;
        let _ = session.request(device_index, self.feature_index, function, &full_payload)?;
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
    fn rgb_effects_driver_queries_zone_count_and_sets_effect() {
        let feat_idx = FeatureIndex(0x05);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::RGB_EFFECTS,
            version: 1,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = RgbEffectsDriver::from_entry(&entry).unwrap();

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "RGB Mouse");
        let mut mock = MockTransport::new(info);

        // fn 0 -> 2 zones
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00],
            )
            .unwrap(),
        );

        // fn 2 -> set effect on zone 0
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
        let count = driver.get_zone_count(&mut session, dev_idx).unwrap();
        assert_eq!(count, 2);

        driver
            .set_effect(&mut session, dev_idx, 0, &[0x01, 0xFF, 0x00, 0x00])
            .unwrap();
    }
}

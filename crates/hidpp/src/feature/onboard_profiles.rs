//! `ONBOARD_PROFILES` (`0x8100`) feature driver.
//!
//! Controls host software mode vs. onboard profile memory mode, profile slot selection,
//! and onboard memory layout inspection for devices such as the G502 HERO.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_DESCRIPTION: u8 = 0;
const FN_SET_ONBOARD_MODE: u8 = 1;
const FN_GET_ONBOARD_MODE: u8 = 2;
const FN_SET_CURRENT_PROFILE: u8 = 3;
const FN_GET_CURRENT_PROFILE: u8 = 4;

/// Operational mode of the device: onboard profile memory vs. host software mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardMode {
    /// Onboard profile memory mode (`0x01`).
    /// Device executes macros, DPI profiles, and LED settings directly from internal flash.
    /// In this mode, volatile host writes (e.g. `0x8060 setReportRate`) return `0x02 InvalidArgument`.
    Onboard,
    /// Host / software mode (`0x02`).
    /// Device delegates polling rate and volatile lighting control to host software without requiring keepalives.
    Host,
    /// Unrecognized mode code.
    Other(u8),
}

impl From<u8> for OnboardMode {
    fn from(val: u8) -> Self {
        match val {
            0x01 => Self::Onboard,
            0x02 => Self::Host,
            other => Self::Other(other),
        }
    }
}

impl From<OnboardMode> for u8 {
    fn from(mode: OnboardMode) -> Self {
        match mode {
            OnboardMode::Onboard => 0x01,
            OnboardMode::Host => 0x02,
            OnboardMode::Other(val) => val,
        }
    }
}

/// Description of device onboard memory geometry and capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardMemoryDescription {
    /// Memory model / format identifier (e.g. `0x01`).
    pub memory_model: u8,
    /// Number of onboard profile slots (e.g. `5`).
    pub profile_count: u8,
    /// Number of physical buttons mapped in profiles (e.g. `11`).
    pub button_count: u8,
    /// Number of memory sectors (e.g. `16`).
    pub sector_count: u8,
}

/// Driver for `ONBOARD_PROFILES` (`0x8100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardProfilesDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for OnboardProfilesDriver {
    const FEATURE_ID: FeatureId = FeatureId::ONBOARD_PROFILES;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl OnboardProfilesDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x8100`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected ONBOARD_PROFILES (0x8100), got {}", entry.id),
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

    /// Queries the onboard memory layout and capabilities (`fn 0 getDescription`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_description<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<OnboardMemoryDescription, HidppError> {
        let function = FunctionId::new(FN_GET_DESCRIPTION)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 7 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "ONBOARD_PROFILES getDescription returned {} bytes (expected >= 7)",
                    params.len()
                ),
            }));
        }
        Ok(OnboardMemoryDescription {
            memory_model: params[0],
            profile_count: params[3],
            button_count: params[4],
            sector_count: params[5],
        })
    }

    /// Queries the current operating mode (`fn 2 getOnboardMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_onboard_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<OnboardMode, HidppError> {
        let function = FunctionId::new(FN_GET_ONBOARD_MODE)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "ONBOARD_PROFILES getOnboardMode returned empty parameters".into(),
            }));
        }
        Ok(OnboardMode::from(params[0]))
    }

    /// Switches the operating mode (`fn 1 setOnboardMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_onboard_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        mode: OnboardMode,
    ) -> Result<(), HidppError> {
        let mode_byte = u8::from(mode);
        let function = FunctionId::new(FN_SET_ONBOARD_MODE)?;
        let _ = session.request(device_index, self.feature_index, function, &[mode_byte])?;
        Ok(())
    }

    /// Switches the active onboard profile slot (`fn 3 setCurrentProfile`).
    ///
    /// Slot numbers are 1-based (e.g. `1..=5`). Setting slot `0` will be rejected by firmware.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `slot == 0`, or an error if transport fails.
    pub fn set_current_profile<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        slot: u16,
    ) -> Result<(), HidppError> {
        if slot == 0 {
            return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
                detail: "profile slot index must be >= 1 (slot 0 is directory sector)".into(),
            }));
        }
        let slot_bytes = slot.to_be_bytes();
        let function = FunctionId::new(FN_SET_CURRENT_PROFILE)?;
        let _ = session.request(device_index, self.feature_index, function, &slot_bytes)?;
        Ok(())
    }

    /// Queries the currently active onboard profile slot (`fn 4 getCurrentProfile`).
    ///
    /// Returns `0` when the device is operating in Host mode.
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_current_profile<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u16, HidppError> {
        let function = FunctionId::new(FN_GET_CURRENT_PROFILE)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 2 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "ONBOARD_PROFILES getCurrentProfile returned {} bytes (expected >= 2)",
                    params.len()
                ),
            }));
        }
        Ok(u16::from_be_bytes([params[0], params[1]]))
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
    fn golden_g502_hero_onboard_profiles_mode_switch_and_query() {
        let feat_idx = FeatureIndex(0x0C);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::ONBOARD_PROFILES,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = OnboardProfilesDriver::from_entry(&entry).unwrap();
        assert_eq!(driver.version(), 0);

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // 1. fn 0 getDescription -> G502 HERO memory layout
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x02, 0x01, 0x05, 0x0B, 0x10, 0x00],
            )
            .unwrap(),
        );

        // 2. fn 2 getOnboardMode -> initially Onboard (0x01)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00],
            )
            .unwrap(),
        );

        // 3. fn 1 setOnboardMode -> switch to Host mode (0x02)
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

        // 4. fn 2 getOnboardMode -> now Host (0x02)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00],
            )
            .unwrap(),
        );

        // 5. fn 4 getCurrentProfile in host mode -> 0x0000
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(4).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let desc = driver.get_description(&mut session, dev_idx).unwrap();
        assert_eq!(desc.memory_model, 1);
        assert_eq!(desc.profile_count, 5);
        assert_eq!(desc.button_count, 11);
        assert_eq!(desc.sector_count, 16);

        let mode = driver.get_onboard_mode(&mut session, dev_idx).unwrap();
        assert_eq!(mode, OnboardMode::Onboard);

        driver
            .set_onboard_mode(&mut session, dev_idx, OnboardMode::Host)
            .unwrap();

        let mode_after = driver.get_onboard_mode(&mut session, dev_idx).unwrap();
        assert_eq!(mode_after, OnboardMode::Host);

        let active_prof = driver.get_current_profile(&mut session, dev_idx).unwrap();
        assert_eq!(active_prof, 0);
    }

    #[test]
    fn set_current_profile_rejects_slot_zero() {
        let feat_idx = FeatureIndex(0x0C);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::ONBOARD_PROFILES,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = OnboardProfilesDriver::from_entry(&entry).unwrap();
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mock = MockTransport::new(info);
        let mut session = test_session(mock);

        let res = driver.set_current_profile(&mut session, dev_idx, 0);
        assert!(res.is_err());
    }
}

//! `MOUSE_BUTTON_SPY` (`0x8110`) feature driver.
//!
//! Enables intercepting physical mouse button presses and releases via asynchronous
//! HID++ notification frames (`SwId = 0x00`).

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId, HidppFrame};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_BUTTON_COUNT: u8 = 0;
const FN_START_SPY: u8 = 1;
const FN_STOP_SPY: u8 = 2;

/// Bitmask representing currently pressed buttons (up to 16 buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ButtonMask(pub u16);

impl ButtonMask {
    /// Returns true if the zero-based button index is pressed.
    #[must_use]
    pub const fn is_pressed(self, button_index: u8) -> bool {
        if button_index >= 16 {
            return false;
        }
        (self.0 & (1 << button_index)) != 0
    }

    /// Returns a list of all zero-based button indices that are currently pressed.
    #[must_use]
    pub fn pressed_buttons(self) -> Vec<u8> {
        let mut list = Vec::new();
        for i in 0..16 {
            if self.is_pressed(i) {
                list.push(i);
            }
        }
        list
    }

    /// Returns the raw 16-bit mask.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

/// Asynchronous button event parsed from an event frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEvent {
    /// 16-bit bitmask of all currently pressed buttons.
    pub mask: ButtonMask,
}

/// Driver for `MOUSE_BUTTON_SPY` (`0x8110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseButtonSpyDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for MouseButtonSpyDriver {
    const FEATURE_ID: FeatureId = FeatureId::MOUSE_BUTTON_SPY;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl MouseButtonSpyDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x8110`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected MOUSE_BUTTON_SPY (0x8110), got {}", entry.id),
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

    /// Queries the total number of physical mouse buttons (`fn 0 getMouseButtonCount`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_button_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let function = FunctionId::new(FN_GET_BUTTON_COUNT)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "MOUSE_BUTTON_SPY getMouseButtonCount returned empty parameters".into(),
            }));
        }
        Ok(params[0])
    }

    /// Starts button spy reporting (`fn 1 startMouseButtonSpy`).
    ///
    /// The device will start sending asynchronous notifications with `SwId = 0x00`.
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn start_spy<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<(), HidppError> {
        let function = FunctionId::new(FN_START_SPY)?;
        let _ = session.request(device_index, self.feature_index, function, &[])?;
        Ok(())
    }

    /// Stops button spy reporting (`fn 2 stopMouseButtonSpy`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn stop_spy<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<(), HidppError> {
        let function = FunctionId::new(FN_STOP_SPY)?;
        let _ = session.request(device_index, self.feature_index, function, &[])?;
        Ok(())
    }

    /// Parses an incoming frame as a `MOUSE_BUTTON_SPY` event notification.
    ///
    /// Returns `Ok(Some(ButtonEvent))` if the frame matches this feature's index
    /// and has `software_id() == 0x00` (asynchronous event).
    /// Returns `Ok(None)` if the frame is unrelated.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedResponse`] if the frame matches this feature and is an event
    /// but has fewer than 2 parameter bytes.
    pub fn parse_notification(
        &self,
        frame: &HidppFrame,
    ) -> Result<Option<ButtonEvent>, ProtocolError> {
        if frame.feature_index() != self.feature_index {
            return Ok(None);
        }
        if !frame.sw_id().is_notification() {
            return Ok(None);
        }
        let params = frame.params();
        if params.len() < 2 {
            return Err(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "MOUSE_BUTTON_SPY event notification truncated: {} bytes (expected >= 2)",
                    params.len()
                ),
            });
        }
        let mask = u16::from_be_bytes([params[0], params[1]]);
        Ok(Some(ButtonEvent {
            mask: ButtonMask(mask),
        }))
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
    fn golden_g502_hero_button_spy_lifecycle_and_notifications() {
        let feat_idx = FeatureIndex(0x0D);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::MOUSE_BUTTON_SPY,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = MouseButtonSpyDriver::from_entry(&entry).unwrap();
        assert_eq!(driver.version(), 0);

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // 1. fn 0 getMouseButtonCount -> 11 buttons
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x0B, 0x00],
            )
            .unwrap(),
        );

        // 2. fn 1 startMouseButtonSpy
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

        // 3. fn 2 stopMouseButtonSpy
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

        let count = driver.get_button_count(&mut session, dev_idx).unwrap();
        assert_eq!(count, 11);

        driver.start_spy(&mut session, dev_idx).unwrap();
        driver.stop_spy(&mut session, dev_idx).unwrap();

        // 4. Test notification parsing:
        let left_click = HidppFrame::new(
            ReportId::Long,
            dev_idx,
            feat_idx,
            FunctionId::new(0).unwrap(),
            SwId::NOTIFICATION,
            &[0x00, 0x01],
        )
        .unwrap();

        let ev = driver.parse_notification(&left_click).unwrap().unwrap();
        assert!(ev.mask.is_pressed(0));
        assert!(!ev.mask.is_pressed(1));
        assert_eq!(ev.mask.pressed_buttons(), vec![0]);

        let release = HidppFrame::new(
            ReportId::Long,
            dev_idx,
            feat_idx,
            FunctionId::new(0).unwrap(),
            SwId::NOTIFICATION,
            &[0x00, 0x00],
        )
        .unwrap();
        let ev_rel = driver.parse_notification(&release).unwrap().unwrap();
        assert_eq!(ev_rel.mask.as_u16(), 0);
        assert!(ev_rel.mask.pressed_buttons().is_empty());
    }
}

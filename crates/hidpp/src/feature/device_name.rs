//! `DEVICE_NAME` (`0x0005`) feature driver for querying device marketing name and type.

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_NAME_COUNT: u8 = 0;
const FN_GET_NAME: u8 = 1;
const FN_GET_DEVICE_TYPE: u8 = 2;

/// Device classification byte returned by `DEVICE_NAME::getDeviceType` (`fn2`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HidppDeviceType {
    /// `0x00`: Keyboard.
    Keyboard,
    /// `0x01`: Remote control.
    RemoteControl,
    /// `0x02`: Numeric keypad.
    Numpad,
    /// `0x03`: Mouse.
    Mouse,
    /// `0x04`: Touchpad.
    Touchpad,
    /// `0x05`: Trackball.
    Trackball,
    /// `0x06`: Presenter.
    Presenter,
    /// `0x07`: Wireless receiver.
    Receiver,
    /// `0x08`: Headset.
    Headset,
    /// `0x09`: Webcam.
    Webcam,
    /// `0x0A`: Steering wheel.
    SteeringWheel,
    /// `0x0B`: Joystick.
    Joystick,
    /// `0x0C`: Gamepad.
    Gamepad,
    /// `0x0D`: Dial.
    Dial,
    /// Unrecognized device type byte.
    Other(u8),
}

impl HidppDeviceType {
    /// Parses a wire device-type byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Keyboard,
            0x01 => Self::RemoteControl,
            0x02 => Self::Numpad,
            0x03 => Self::Mouse,
            0x04 => Self::Touchpad,
            0x05 => Self::Trackball,
            0x06 => Self::Presenter,
            0x07 => Self::Receiver,
            0x08 => Self::Headset,
            0x09 => Self::Webcam,
            0x0A => Self::SteeringWheel,
            0x0B => Self::Joystick,
            0x0C => Self::Gamepad,
            0x0D => Self::Dial,
            other => Self::Other(other),
        }
    }

    /// Returns the raw wire byte.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Keyboard => 0x00,
            Self::RemoteControl => 0x01,
            Self::Numpad => 0x02,
            Self::Mouse => 0x03,
            Self::Touchpad => 0x04,
            Self::Trackball => 0x05,
            Self::Presenter => 0x06,
            Self::Receiver => 0x07,
            Self::Headset => 0x08,
            Self::Webcam => 0x09,
            Self::SteeringWheel => 0x0A,
            Self::Joystick => 0x0B,
            Self::Gamepad => 0x0C,
            Self::Dial => 0x0D,
            Self::Other(b) => b,
        }
    }

    /// Returns a human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::RemoteControl => "remote-control",
            Self::Numpad => "numpad",
            Self::Mouse => "mouse",
            Self::Touchpad => "touchpad",
            Self::Trackball => "trackball",
            Self::Presenter => "presenter",
            Self::Receiver => "receiver",
            Self::Headset => "headset",
            Self::Webcam => "webcam",
            Self::SteeringWheel => "steering-wheel",
            Self::Joystick => "joystick",
            Self::Gamepad => "gamepad",
            Self::Dial => "dial",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for HidppDeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(b) => write!(f, "other (0x{b:02X})"),
            _ => f.write_str(self.label()),
        }
    }
}

/// Trait implemented by versioned `DEVICE_NAME` (`0x0005`) drivers.
pub trait DeviceNameFeature {
    /// Returns the total byte length of the device marketing name (`fn0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_name_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError>;

    /// Reads the complete UTF-8 device marketing name by paging `fn1` chunks.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport failures, non-progressing chunks, or invalid UTF-8.
    fn get_name<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<String, HidppError>;

    /// Queries the device classification (`fn2`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_device_type<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<HidppDeviceType, HidppError>;
}

/// Versioned runtime driver for `DEVICE_NAME` (`0x0005`).
#[derive(Debug, Clone, Copy)]
pub struct DeviceNameDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for DeviceNameDriver {
    const FEATURE_ID: FeatureId = FeatureId::DEVICE_NAME;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl DeviceNameDriver {
    /// Creates a versioned driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::DEVICE_NAME`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected DEVICE_NAME (0x0005), got {}", entry.id),
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

    /// Encodes parameters for `getDeviceNameCount` (`fn0`).
    #[must_use]
    pub const fn encode_get_name_count_params() -> [u8; 0] {
        []
    }

    /// Encodes parameters for `getDeviceName(char_index)` (`fn1`).
    #[must_use]
    pub const fn encode_get_name_params(char_index: u8) -> [u8; 1] {
        [char_index]
    }

    /// Encodes parameters for `getDeviceType` (`fn2`).
    #[must_use]
    pub const fn encode_get_device_type_params() -> [u8; 0] {
        []
    }

    /// Pure parser for `getDeviceNameCount` (`fn0`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty.
    pub fn parse_name_count_response(params: &[u8]) -> Result<u8, ProtocolError> {
        let Some(&count) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        Ok(count)
    }

    /// Pure parser extracting up to `remaining` bytes from a `getDeviceName` (`fn1`) chunk.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty when `remaining > 0`.
    pub fn parse_name_chunk_response(
        params: &[u8],
        remaining: usize,
    ) -> Result<&[u8], ProtocolError> {
        if remaining == 0 {
            return Ok(&[]);
        }
        if params.is_empty() {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        }
        let take = remaining.min(params.len());
        Ok(&params[..take])
    }

    /// Decodes an assembled device-name byte buffer into a trimmed UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedResponse`] if the bytes are not valid UTF-8.
    pub fn decode_assembled_name(raw: &[u8]) -> Result<String, ProtocolError> {
        let trimmed = match raw.iter().position(|&b| b == 0) {
            Some(nul) => &raw[..nul],
            None => raw,
        };
        let text =
            std::str::from_utf8(trimmed).map_err(|err| ProtocolError::UnexpectedResponse {
                detail: format!("DEVICE_NAME returned invalid UTF-8: {err}"),
            })?;
        Ok(text.to_owned())
    }

    /// Pure parser for `getDeviceType` (`fn2`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty.
    pub fn parse_device_type_response(params: &[u8]) -> Result<HidppDeviceType, ProtocolError> {
        let Some(&raw_kind) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        Ok(HidppDeviceType::from_byte(raw_kind))
    }
}

impl DeviceNameFeature for DeviceNameDriver {
    fn get_name_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_NAME_COUNT)?,
            &Self::encode_get_name_count_params(),
        )?;
        Ok(Self::parse_name_count_response(reply.params())?)
    }

    fn get_name<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<String, HidppError> {
        let total_len = usize::from(self.get_name_count(session, device_index)?);
        if total_len == 0 {
            return Ok(String::new());
        }

        let mut buf = Vec::with_capacity(total_len);
        while buf.len() < total_len {
            let offset =
                u8::try_from(buf.len()).map_err(|_| ProtocolError::UnexpectedResponse {
                    detail: format!("DEVICE_NAME length {} exceeds u8 charIndex", buf.len()),
                })?;
            let req_params = Self::encode_get_name_params(offset);
            let reply = session.request(
                device_index,
                self.feature_index,
                FunctionId::new(FN_GET_NAME)?,
                &req_params,
            )?;
            let remaining = total_len - buf.len();
            let chunk = Self::parse_name_chunk_response(reply.params(), remaining)?;
            if chunk.is_empty() {
                return Err(ProtocolError::UnexpectedResponse {
                    detail: "DEVICE_NAME returned empty chunk before full name was read".to_owned(),
                }
                .into());
            }
            buf.extend_from_slice(chunk);
        }

        Ok(Self::decode_assembled_name(&buf)?)
    }

    fn get_device_type<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<HidppDeviceType, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_DEVICE_TYPE)?,
            &Self::encode_get_device_type_params(),
        )?;
        Ok(Self::parse_device_type_response(reply.params())?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;
    use crate::frame::{HidppFrame, ReportId, SwId};
    use crate::session::SessionConfig;
    use crate::transport::{HidDeviceInfo, MockTransport};

    #[test]
    fn golden_device_name_multi_chunk_and_type() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x04);

        // fn0 getCount -> 22 ("G502 HERO Gaming Mouse")
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[22, 0, 0],
            )
            .unwrap(),
        );
        // fn1(0) -> first 16 bytes
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                b"G502 HERO Gaming",
            )
            .unwrap(),
        );
        // fn1(16) -> remaining 6 bytes + zero pad
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                b" Mouse\0\0\0\0\0\0\0\0\0\0",
            )
            .unwrap(),
        );
        // fn2 getDeviceType -> 0x03 (Mouse)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x03, 0x00, 0x00],
            )
            .unwrap(),
        );

        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        let mut session = HidppSession::with_config(mock, cfg);
        let driver = DeviceNameDriver::new(feat_idx, 0);

        let name = driver.get_name(&mut session, DeviceIndex::DIRECT).unwrap();
        assert_eq!(name, "G502 HERO Gaming Mouse");

        let kind = driver
            .get_device_type(&mut session, DeviceIndex::DIRECT)
            .unwrap();
        assert_eq!(kind, HidppDeviceType::Mouse);

        // Verify chunk offset requests
        let sent = session.transport().written_frames();
        assert_eq!(sent[1].params()[0], 0);
        assert_eq!(sent[2].params()[0], 16);
    }

    #[test]
    fn rejects_invalid_utf8_and_truncated_responses() {
        assert!(matches!(
            DeviceNameDriver::parse_name_count_response(&[]),
            Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(
            DeviceNameDriver::parse_device_type_response(&[]),
            Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(
            DeviceNameDriver::decode_assembled_name(&[0xFF, 0xFE]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
    }

    proptest! {
        #[test]
        fn device_type_byte_roundtrip(raw in any::<u8>()) {
            let kind = HidppDeviceType::from_byte(raw);
            prop_assert_eq!(kind.as_byte(), raw);
        }

        #[test]
        fn ascii_name_assembly_roundtrip(name in "[A-Za-z0-9 _-]{0,30}") {
            let decoded = DeviceNameDriver::decode_assembled_name(name.as_bytes()).unwrap();
            prop_assert_eq!(decoded, name);
        }
    }
}

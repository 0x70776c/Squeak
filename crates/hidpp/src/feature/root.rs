//! `IRoot` (`0x0000`) feature driver: protocol version ping and feature ID resolution.
//!
//! `IRoot` is guaranteed by the HID++ 2.0 specification to live at feature index `0x00`.

use crate::error::{Hidpp10ErrorCode, HidppError, ProtocolError};
use crate::feature::{FeatureFlags, FeatureId, HidppFeature, ProtocolVersion};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_FEATURE: u8 = 0;
const FN_PING: u8 = 1;

/// Resolution result returned by [`IRoot::get_feature`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedFeature {
    /// Runtime-assigned non-zero feature table index.
    pub index: FeatureIndex,
    /// Feature visibility/deprecation flags.
    pub flags: FeatureFlags,
    /// Feature revision reported by firmware (`0` if omitted in v0 short responses).
    pub version: u8,
}

/// Pure encoder/decoder and session driver for HID++ 2.0 `IRoot` (`0x0000`).
#[derive(Debug, Clone, Copy, Default)]
pub struct IRoot;

impl HidppFeature for IRoot {
    const FEATURE_ID: FeatureId = FeatureId::IROOT;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl IRoot {
    /// Encodes request parameters for `IRoot::getFeature(feature_id)` (function `0`).
    #[must_use]
    pub const fn encode_get_feature_params(feature_id: FeatureId) -> [u8; 3] {
        let [msb, lsb] = feature_id.as_u16().to_be_bytes();
        [msb, lsb, 0x00]
    }

    /// Parses response parameters from `IRoot::getFeature` (`fn0`).
    ///
    /// Returns `Ok(None)` when firmware reports index `0x00` (feature not implemented).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 2 parameter bytes exist.
    pub fn parse_get_feature_response(
        params: &[u8],
    ) -> Result<Option<ResolvedFeature>, ProtocolError> {
        let Some(&idx) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: 0,
            });
        };
        let Some(&raw_flags) = params.get(1) else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: params.len(),
            });
        };

        if idx == 0 {
            return Ok(None);
        }
        if idx == FeatureIndex::ERROR.0 {
            return Err(ProtocolError::UnexpectedResponse {
                detail: "IRoot::getFeature returned reserved error index 0xFF".to_owned(),
            });
        }

        Ok(Some(ResolvedFeature {
            index: FeatureIndex(idx),
            flags: FeatureFlags::from_raw(raw_flags),
            version: params.get(2).copied().unwrap_or(0),
        }))
    }

    /// Encodes request parameters for `IRoot::ping(ping_data)` (function `1`).
    #[must_use]
    pub const fn encode_ping_params(ping_data: u8) -> [u8; 3] {
        [0x00, 0x00, ping_data]
    }

    /// Parses response parameters from `IRoot::ping` (`fn1`), verifying the ping echo byte.
    ///
    /// # Errors
    ///
    /// - [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes are present.
    /// - [`ProtocolError::UnexpectedResponse`] if the echoed ping byte mismatches.
    /// - [`ProtocolError::LegacyDeviceNotSupported`] if major protocol version `< 2`.
    pub fn parse_ping_response(
        params: &[u8],
        expected_ping_data: u8,
    ) -> Result<ProtocolVersion, ProtocolError> {
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let major = params[0];
        let minor = params[1];
        let echo = params[2];

        if echo != expected_ping_data {
            return Err(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "IRoot::ping echo mismatch: sent 0x{expected_ping_data:02X}, got 0x{echo:02X}"
                ),
            });
        }
        if major < 2 {
            return Err(ProtocolError::LegacyDeviceNotSupported {
                error_code: Hidpp10ErrorCode::Success,
            });
        }

        Ok(ProtocolVersion { major, minor })
    }

    /// Queries the device protocol version using `IRoot::ping` (function `1`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport failures, HID++ 1.0 legacy responses, or malformed frames.
    pub fn ping<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        ping_data: u8,
    ) -> Result<ProtocolVersion, HidppError> {
        let params = Self::encode_ping_params(ping_data);
        let reply = session.request(
            device_index,
            FeatureIndex::IROOT,
            FunctionId::new(FN_PING)?,
            &params,
        )?;
        Ok(Self::parse_ping_response(reply.params(), ping_data)?)
    }

    /// Resolves a feature ID to its runtime [`FeatureIndex`] using `IRoot::getFeature` (function `0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport, protocol, or device error responses.
    pub fn get_feature<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        feature_id: FeatureId,
    ) -> Result<Option<ResolvedFeature>, HidppError> {
        let params = Self::encode_get_feature_params(feature_id);
        let reply = session.request(
            device_index,
            FeatureIndex::IROOT,
            FunctionId::new(FN_GET_FEATURE)?,
            &params,
        )?;
        Ok(Self::parse_get_feature_response(reply.params())?)
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
    fn golden_iroot_ping_and_get_feature() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // ping(0x5A) -> v4.2 echo 0x5A
        mock.push_frame(HidppFrame::decode(&[0x10, 0xFF, 0x00, 0x1A, 0x04, 0x02, 0x5A]).unwrap());
        // getFeature(0x0001) -> index 0x01, flags 0x00, version 0x00
        mock.push_frame(HidppFrame::decode(&[0x10, 0xFF, 0x00, 0x0A, 0x01, 0x00, 0x00]).unwrap());
        // getFeature(0x9999) -> absent (index 0x00)
        mock.push_frame(HidppFrame::decode(&[0x10, 0xFF, 0x00, 0x0A, 0x00, 0x00, 0x00]).unwrap());

        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        let mut session = HidppSession::with_config(mock, cfg);

        let version = IRoot::ping(&mut session, DeviceIndex::DIRECT, 0x5A).unwrap();
        assert_eq!(version, ProtocolVersion { major: 4, minor: 2 });

        let fs = IRoot::get_feature(&mut session, DeviceIndex::DIRECT, FeatureId::IFEATURE_SET)
            .unwrap()
            .unwrap();
        assert_eq!(fs.index, FeatureIndex(0x01));
        assert_eq!(fs.version, 0);

        let missing =
            IRoot::get_feature(&mut session, DeviceIndex::DIRECT, FeatureId(0x9999)).unwrap();
        assert!(missing.is_none());

        // Verify exact outgoing wire bytes for ping(0x5A) and getFeature(0x0001)
        let sent = session.transport().written_frames();
        assert_eq!(
            sent[0].to_bytes(),
            [0x10, 0xFF, 0x00, 0x1A, 0x00, 0x00, 0x5A]
        );
        assert_eq!(
            sent[1].to_bytes(),
            [0x10, 0xFF, 0x00, 0x0A, 0x00, 0x01, 0x00]
        );
    }

    #[test]
    fn rejects_ping_echo_mismatch_and_truncated_payload() {
        assert!(matches!(
            IRoot::parse_ping_response(&[0x04, 0x05], 0x11),
            Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: 2
            })
        ));
        assert!(matches!(
            IRoot::parse_ping_response(&[0x04, 0x05, 0x22], 0x11),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
        assert!(matches!(
            IRoot::parse_ping_response(&[0x01, 0x00, 0x11], 0x11),
            Err(ProtocolError::LegacyDeviceNotSupported { .. })
        ));
        assert!(matches!(
            IRoot::parse_get_feature_response(&[0x01]),
            Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: 1
            })
        ));
        assert!(matches!(
            IRoot::parse_get_feature_response(&[0xFF, 0x00, 0x00]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
    }

    proptest! {
        #[test]
        fn get_feature_params_roundtrip(id in any::<u16>()) {
            let params = IRoot::encode_get_feature_params(FeatureId(id));
            prop_assert_eq!(u16::from_be_bytes([params[0], params[1]]), id);
            prop_assert_eq!(params[2], 0x00);
        }

        #[test]
        fn ping_response_roundtrip(major in 2u8..=10, minor in any::<u8>(), echo in any::<u8>()) {
            let frame = HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[major, minor, echo],
            ).unwrap();
            let parsed = IRoot::parse_ping_response(frame.params(), echo).unwrap();
            prop_assert_eq!(parsed.major, major);
            prop_assert_eq!(parsed.minor, minor);
        }
    }
}

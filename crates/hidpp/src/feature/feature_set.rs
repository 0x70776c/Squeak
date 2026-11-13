//! `IFeatureSet` (`0x0001`) feature driver for runtime feature table enumeration.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureFlags, FeatureId, HidppFeature};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_COUNT: u8 = 0;
const FN_GET_FEATURE_ID: u8 = 1;

/// Pure encoder/decoder and session driver for HID++ 2.0 `IFeatureSet` (`0x0001`).
#[derive(Debug, Clone, Copy, Default)]
pub struct IFeatureSet;

impl HidppFeature for IFeatureSet {
    const FEATURE_ID: FeatureId = FeatureId::IFEATURE_SET;
    const MAX_SUPPORTED_VERSION: u8 = 2;
}

impl IFeatureSet {
    /// Encodes request parameters for `IFeatureSet::getCount` (function `0`).
    #[must_use]
    pub const fn encode_get_count_params() -> [u8; 0] {
        []
    }

    /// Parses response parameters for `IFeatureSet::getCount` (function `0`).
    ///
    /// Returns the number of enumerated features excluding `IRoot` at index `0x00`
    /// (valid feature indices for [`Self::get_feature_id`] are `1..=count`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty.
    pub fn parse_get_count_response(params: &[u8]) -> Result<u8, ProtocolError> {
        let Some(&count) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        Ok(count)
    }

    /// Encodes request parameters for `IFeatureSet::getFeatureID(feature_index)` (function `1`).
    #[must_use]
    pub const fn encode_get_feature_id_params(feature_index: u8) -> [u8; 1] {
        [feature_index]
    }

    /// Parses response parameters for `IFeatureSet::getFeatureID` (function `1`).
    ///
    /// Handles both `Short` (`0x10`, 3 parameter bytes, v0 where `version` defaults to `0`)
    /// and `Long` (`0x11`, >= 4 parameter bytes, v1+ where `params[3]` carries `version`).
    ///
    /// Note: on `IFeatureSet` v0 devices replying with `Long` frames, byte `params[3]` is
    /// undefined and may hold stale buffer bytes; prefer [`Self::parse_get_feature_id_response_versioned`]
    /// with the `IFeatureSet` version discovered via `IRoot::getFeature`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes are present.
    pub fn parse_get_feature_id_response(
        index: FeatureIndex,
        params: &[u8],
    ) -> Result<FeatureEntry, ProtocolError> {
        Self::parse_get_feature_id_response_versioned(index, 1, params)
    }

    /// Parses `IFeatureSet::getFeatureID` response parameters knowing `feature_set_version`.
    ///
    /// When `feature_set_version == 0`, the returned feature revision is forced to `0` even if the
    /// response arrived in a 20-byte `Long` report with non-zero trailing buffer bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes are present.
    pub fn parse_get_feature_id_response_versioned(
        index: FeatureIndex,
        feature_set_version: u8,
        params: &[u8],
    ) -> Result<FeatureEntry, ProtocolError> {
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let id = FeatureId(u16::from_be_bytes([params[0], params[1]]));
        let flags = FeatureFlags::from_raw(params[2]);
        let version = if feature_set_version == 0 {
            0
        } else {
            params.get(3).copied().unwrap_or(0)
        };
        Ok(FeatureEntry {
            index,
            id,
            version,
            flags,
        })
    }

    /// Queries the number of discoverable features (`1..=count`) on the device.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport, protocol, or device error responses.
    pub fn get_count<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        feature_set_index: FeatureIndex,
    ) -> Result<u8, HidppError> {
        let reply = session.request(
            device_index,
            feature_set_index,
            FunctionId::new(FN_GET_COUNT)?,
            &Self::encode_get_count_params(),
        )?;
        Ok(Self::parse_get_count_response(reply.params())?)
    }

    /// Queries metadata for the feature at `query_index` (`1..=count`) assuming `IFeatureSet` v1+.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport, protocol, or device error responses.
    pub fn get_feature_id<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        feature_set_index: FeatureIndex,
        query_index: u8,
    ) -> Result<FeatureEntry, HidppError> {
        Self::get_feature_id_versioned(session, device_index, feature_set_index, 1, query_index)
    }

    /// Queries metadata for the feature at `query_index` (`1..=count`), masking `version` to `0`
    /// when `feature_set_version == 0`.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport, protocol, or device error responses.
    pub fn get_feature_id_versioned<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        feature_set_index: FeatureIndex,
        feature_set_version: u8,
        query_index: u8,
    ) -> Result<FeatureEntry, HidppError> {
        let params = Self::encode_get_feature_id_params(query_index);
        let reply = session.request(
            device_index,
            feature_set_index,
            FunctionId::new(FN_GET_FEATURE_ID)?,
            &params,
        )?;
        Ok(Self::parse_get_feature_id_response_versioned(
            FeatureIndex(query_index),
            feature_set_version,
            reply.params(),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn golden_parse_short_v0_and_long_v1_feature_id() {
        // Short 0x10 frame only carries 3 param bytes: [0x22, 0x01, 0x00] -> version defaults to 0
        let entry_short =
            IFeatureSet::parse_get_feature_id_response(FeatureIndex(0x04), &[0x22, 0x01, 0x00])
                .unwrap();
        assert_eq!(entry_short.index, FeatureIndex(0x04));
        assert_eq!(entry_short.id, FeatureId::ADJUSTABLE_DPI);
        assert_eq!(entry_short.version, 0);
        assert!(!entry_short.flags.is_obsolete());

        // Long 0x11 frame carries 4th byte version + hidden/obsolete flags
        let entry_long = IFeatureSet::parse_get_feature_id_response(
            FeatureIndex(0x07),
            &[0x80, 0x71, 0xC0, 0x03],
        )
        .unwrap();
        assert_eq!(entry_long.id, FeatureId::RGB_EFFECTS);
        assert_eq!(entry_long.version, 3);
        assert!(entry_long.flags.is_obsolete());
        assert!(entry_long.flags.is_hidden());
    }

    #[test]
    fn rejects_truncated_responses() {
        assert!(matches!(
            IFeatureSet::parse_get_count_response(&[]),
            Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(
            IFeatureSet::parse_get_feature_id_response(FeatureIndex(1), &[0x00, 0x05]),
            Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: 2
            })
        ));
    }

    #[test]
    fn versioned_parser_masks_v0_long_tail_byte() {
        let v0_entry = IFeatureSet::parse_get_feature_id_response_versioned(
            FeatureIndex(2),
            0,
            &[0x00, 0x05, 0x00, 0x7F],
        )
        .unwrap();
        assert_eq!(v0_entry.version, 0);

        let v1_entry = IFeatureSet::parse_get_feature_id_response_versioned(
            FeatureIndex(2),
            1,
            &[0x00, 0x05, 0x00, 0x7F],
        )
        .unwrap();
        assert_eq!(v1_entry.version, 0x7F);
    }

    proptest! {
        #[test]
        fn get_feature_id_response_roundtrip(
            idx in 1u8..=254,
            fid in any::<u16>(),
            flags in any::<u8>(),
            version in any::<u8>(),
        ) {
            let [msb, lsb] = fid.to_be_bytes();
            let parsed = IFeatureSet::parse_get_feature_id_response(
                FeatureIndex(idx),
                &[msb, lsb, flags, version],
            ).unwrap();
            prop_assert_eq!(parsed.index, FeatureIndex(idx));
            prop_assert_eq!(parsed.id, FeatureId(fid));
            prop_assert_eq!(parsed.flags.raw(), flags);
            prop_assert_eq!(parsed.version, version);
        }
    }
}

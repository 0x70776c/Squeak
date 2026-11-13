//! HID++ 2.0 wire frame types, constants, and bounds-checked encoding/decoding.
//!
//! Frame layout (all bytes):
//! - `[0]` `report_id`: `0x10` (short, 7B), `0x11` (long, 20B), `0x12` (very long, 64B)
//! - `[1]` `device_index`: `0xFF` for direct USB/BT, `1..=6` for receiver slots
//! - `[2]` `feature_index`: runtime-discovered index (`0x00` = `IRoot`, `0xFF` = HID++ 2.0 error)
//! - `[3]` `(function_id << 4) | (sw_id & 0x0F)`
//! - `[4..]` parameters (zero-padded to the selected report's fixed wire length)

use core::fmt;

use crate::error::ProtocolError;

/// Header byte length shared by all HID++ 2.0 reports.
pub const HEADER_LEN: usize = 4;

/// Total wire length (including report ID) of a short HID++ report (`0x10`).
pub const SHORT_REPORT_LEN: usize = 7;

/// Total wire length (including report ID) of a long HID++ report (`0x11`).
pub const LONG_REPORT_LEN: usize = 20;

/// Total wire length (including report ID) of a very long HID++ report (`0x12`).
pub const VERY_LONG_REPORT_LEN: usize = 64;

/// Maximum wire length of any supported HID++ report.
pub const MAX_REPORT_LEN: usize = VERY_LONG_REPORT_LEN;

/// Parameter byte capacity of a short HID++ report (`0x10`).
pub const SHORT_PARAM_LEN: usize = SHORT_REPORT_LEN - HEADER_LEN;

/// Parameter byte capacity of a long HID++ report (`0x11`).
pub const LONG_PARAM_LEN: usize = LONG_REPORT_LEN - HEADER_LEN;

/// Parameter byte capacity of a very long HID++ report (`0x12`).
pub const VERY_LONG_PARAM_LEN: usize = VERY_LONG_REPORT_LEN - HEADER_LEN;

/// Feature index marker indicating an HID++ 2.0 error response.
pub const ERROR_FEATURE_INDEX: u8 = 0xFF;

/// Sub-ID marker indicating an HID++ 1.0 legacy error response (`ERROR_MSG`).
pub const HIDPP10_ERROR_SUB_ID: u8 = 0x8F;

/// HID++ 2.0 report ID identifying the fixed frame size.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReportId {
    /// Short report (`0x10`, 7 bytes total, 3 parameter bytes).
    Short,
    /// Long report (`0x11`, 20 bytes total, 16 parameter bytes).
    Long,
    /// Very long report (`0x12`, 64 bytes total, 60 parameter bytes).
    VeryLong,
}

impl ReportId {
    /// Parses a wire report ID byte.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidReportId`] if `byte` is not `0x10`, `0x11`, or `0x12`.
    pub const fn from_byte(byte: u8) -> Result<Self, ProtocolError> {
        match byte {
            0x10 => Ok(Self::Short),
            0x11 => Ok(Self::Long),
            0x12 => Ok(Self::VeryLong),
            _ => Err(ProtocolError::InvalidReportId(byte)),
        }
    }

    /// Returns the wire report ID byte.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Short => 0x10,
            Self::Long => 0x11,
            Self::VeryLong => 0x12,
        }
    }

    /// Returns the fixed total wire frame size in bytes (including the report ID byte).
    #[must_use]
    pub const fn frame_len(self) -> usize {
        match self {
            Self::Short => SHORT_REPORT_LEN,
            Self::Long => LONG_REPORT_LEN,
            Self::VeryLong => VERY_LONG_REPORT_LEN,
        }
    }

    /// Returns the parameter capacity in bytes for this report size.
    #[must_use]
    pub const fn param_len(self) -> usize {
        match self {
            Self::Short => SHORT_PARAM_LEN,
            Self::Long => LONG_PARAM_LEN,
            Self::VeryLong => VERY_LONG_PARAM_LEN,
        }
    }

    /// Selects the smallest standard report ID that fits `payload_len` parameter bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] if `payload_len > VERY_LONG_PARAM_LEN`.
    pub const fn for_payload_len(payload_len: usize) -> Result<Self, ProtocolError> {
        if payload_len <= SHORT_PARAM_LEN {
            Ok(Self::Short)
        } else if payload_len <= LONG_PARAM_LEN {
            Ok(Self::Long)
        } else if payload_len <= VERY_LONG_PARAM_LEN {
            Ok(Self::VeryLong)
        } else {
            Err(ProtocolError::PayloadTooLarge {
                len: payload_len,
                max: VERY_LONG_PARAM_LEN,
            })
        }
    }
}

impl fmt::Display for ReportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:02X}", self.as_byte())
    }
}

/// Set of HID++ report IDs advertised by a device's HID report descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SupportedReports {
    /// Device supports `0x10` short reports (7 bytes).
    pub short: bool,
    /// Device supports `0x11` long reports (20 bytes).
    pub long: bool,
    /// Device supports `0x12` very long reports (64 bytes).
    pub very_long: bool,
}

impl SupportedReports {
    /// Standard wired/receiver HID++ 2.0 capabilities (`0x10` short and `0x11` long).
    pub const SHORT_AND_LONG: Self = Self {
        short: true,
        long: true,
        very_long: false,
    };

    /// All three HID++ report sizes supported (`0x10`, `0x11`, `0x12`).
    pub const ALL: Self = Self {
        short: true,
        long: true,
        very_long: true,
    };

    /// Long-only HID++ capabilities (`0x11` only, common on Bluetooth LE devices).
    pub const LONG_ONLY: Self = Self {
        short: false,
        long: true,
        very_long: false,
    };

    /// Returns `true` if at least one HID++ report ID is supported.
    #[must_use]
    pub const fn any(self) -> bool {
        self.short || self.long || self.very_long
    }

    /// Returns `true` if `report_id` is in this supported set.
    #[must_use]
    pub const fn supports(self, report_id: ReportId) -> bool {
        match report_id {
            ReportId::Short => self.short,
            ReportId::Long => self.long,
            ReportId::VeryLong => self.very_long,
        }
    }

    /// Chooses the smallest supported report ID able to hold `payload_len` parameter bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when no HID++ report can hold `payload_len`
    /// bytes, or [`ProtocolError::NoSupportedReportForPayload`] when the device lacks a large
    /// enough supported report ID.
    pub const fn select_for_payload(self, payload_len: usize) -> Result<ReportId, ProtocolError> {
        if payload_len > VERY_LONG_PARAM_LEN {
            return Err(ProtocolError::PayloadTooLarge {
                len: payload_len,
                max: VERY_LONG_PARAM_LEN,
            });
        }
        if payload_len <= SHORT_PARAM_LEN && self.short {
            return Ok(ReportId::Short);
        }
        if payload_len <= LONG_PARAM_LEN && self.long {
            return Ok(ReportId::Long);
        }
        if self.very_long {
            return Ok(ReportId::VeryLong);
        }
        Err(ProtocolError::NoSupportedReportForPayload {
            payload_len,
            supported: self,
        })
    }

    /// Returns the list of supported report IDs in ascending size order.
    #[must_use]
    pub fn as_vec(self) -> Vec<ReportId> {
        let mut out = Vec::with_capacity(3);
        if self.short {
            out.push(ReportId::Short);
        }
        if self.long {
            out.push(ReportId::Long);
        }
        if self.very_long {
            out.push(ReportId::VeryLong);
        }
        out
    }
}

impl Default for SupportedReports {
    fn default() -> Self {
        Self::SHORT_AND_LONG
    }
}

/// Target device address on a `hidraw` transport (`0xFF` direct or `1..=6` receiver slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceIndex(u8);

impl DeviceIndex {
    /// Direct-connected USB or Bluetooth device index (`0xFF`).
    pub const DIRECT: Self = Self(0xFF);

    /// Validates and constructs a receiver slot device index (`1..=6`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidReceiverSlot`] if `slot` is outside `1..=6`.
    pub const fn receiver(slot: u8) -> Result<Self, ProtocolError> {
        if slot >= 1 && slot <= 6 {
            Ok(Self(slot))
        } else {
            Err(ProtocolError::InvalidReceiverSlot(slot))
        }
    }

    /// Constructs a [`DeviceIndex`] from a raw wire byte (used when decoding incoming frames or
    /// targeting receiver slot `0x00` for receiver-level commands).
    #[must_use]
    pub const fn from_raw(byte: u8) -> Self {
        Self(byte)
    }

    /// Returns the raw wire byte.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        self.0
    }
}

impl fmt::Display for DeviceIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0xFF => write!(f, "0xFF (direct)"),
            slot @ 1..=6 => write!(f, "0x{slot:02X} (slot {slot})"),
            other => write!(f, "0x{other:02X}"),
        }
    }
}

/// Runtime-discovered feature table index (`0x00` = `IRoot`, `0xFF` = HID++ 2.0 error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeatureIndex(pub u8);

impl FeatureIndex {
    /// Fixed index `0x00` always assigned to `IRoot` (`0x0000`).
    pub const IROOT: Self = Self(0x00);

    /// Sentinel index `0xFF` indicating an HID++ 2.0 error response frame.
    pub const ERROR: Self = Self(ERROR_FEATURE_INDEX);

    /// Returns `true` when this index is the HID++ 2.0 error sentinel (`0xFF`).
    #[must_use]
    pub const fn is_error(self) -> bool {
        self.0 == ERROR_FEATURE_INDEX
    }

    /// Returns `true` when this index is the HID++ 1.0 `ERROR_MSG` sentinel (`0x8F`).
    #[must_use]
    pub const fn is_hidpp10_error(self) -> bool {
        self.0 == HIDPP10_ERROR_SUB_ID
    }
}

impl fmt::Display for FeatureIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:02X}", self.0)
    }
}

/// 4-bit HID++ 2.0 feature function identifier (`0..=15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(u8);

impl FunctionId {
    /// Validates and constructs a 4-bit [`FunctionId`] (`0..=15`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidFunctionId`] if `id > 0x0F`.
    pub const fn new(id: u8) -> Result<Self, ProtocolError> {
        if id <= 0x0F {
            Ok(Self(id))
        } else {
            Err(ProtocolError::InvalidFunctionId(id))
        }
    }

    /// Extracts a [`FunctionId`] from the upper nibble of byte `[3]` (`func_sw >> 4`).
    #[must_use]
    pub const fn from_func_sw(byte: u8) -> Self {
        Self((byte >> 4) & 0x0F)
    }

    /// Returns the 4-bit function value (`0..=15`).
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fn{}", self.0)
    }
}

/// 4-bit software identifier (`0` reserved for device notifications, `1..=15` for host requests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SwId(u8);

impl SwId {
    /// Software ID `0x0`, reserved exclusively for unsolicited device notifications/events.
    pub const NOTIFICATION: Self = Self(0x00);

    /// Default non-zero software ID used by [`crate::HidppSession`].
    pub const DEFAULT_REQUEST: Self = Self(0x0A);

    /// Validates and constructs a non-zero host request [`SwId`] (`1..=15`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidRequestSwId`] if `id == 0` or `id > 0x0F`.
    pub const fn request(id: u8) -> Result<Self, ProtocolError> {
        if id >= 1 && id <= 0x0F {
            Ok(Self(id))
        } else {
            Err(ProtocolError::InvalidRequestSwId(id))
        }
    }

    /// Extracts a [`SwId`] from the lower nibble of byte `[3]` (`func_sw & 0x0F`).
    #[must_use]
    pub const fn from_func_sw(byte: u8) -> Self {
        Self(byte & 0x0F)
    }

    /// Returns the 4-bit value (`0..=15`).
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns `true` if `self` is [`SwId::NOTIFICATION`] (`0x00`).
    #[must_use]
    pub const fn is_notification(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for SwId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:X}", self.0)
    }
}

/// Combines [`FunctionId`] and [`SwId`] into byte `[3]` of an HID++ 2.0 header.
#[must_use]
pub const fn pack_func_sw(function_id: FunctionId, sw_id: SwId) -> u8 {
    (function_id.as_u8() << 4) | (sw_id.as_u8() & 0x0F)
}

/// Decoded HID++ 2.0 wire frame with fixed-size stack storage (zero heap allocation).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HidppFrame {
    report_id: ReportId,
    device_index: DeviceIndex,
    feature_index: FeatureIndex,
    function_id: FunctionId,
    sw_id: SwId,
    params: [u8; VERY_LONG_PARAM_LEN],
}

impl HidppFrame {
    /// Builds a new HID++ 2.0 frame, zero-padding any unused parameter bytes in the report.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] if `payload.len() > report_id.param_len()`.
    pub fn new(
        report_id: ReportId,
        device_index: DeviceIndex,
        feature_index: FeatureIndex,
        function_id: FunctionId,
        sw_id: SwId,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        let cap = report_id.param_len();
        if payload.len() > cap {
            return Err(ProtocolError::PayloadTooLarge {
                len: payload.len(),
                max: cap,
            });
        }
        let mut params = [0u8; VERY_LONG_PARAM_LEN];
        params[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            report_id,
            device_index,
            feature_index,
            function_id,
            sw_id,
            params,
        })
    }

    /// Decodes a raw HID report buffer into a validated [`HidppFrame`].
    ///
    /// Strict length validation rejects truncated or oversized buffers for the declared report ID.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::FrameTooShort`], [`ProtocolError::InvalidReportId`], or
    /// [`ProtocolError::InvalidFrameLength`] on malformed wire input.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let Some(&first) = bytes.first() else {
            return Err(ProtocolError::FrameTooShort {
                actual: 0,
                min: SHORT_REPORT_LEN,
            });
        };
        let report_id = ReportId::from_byte(first)?;
        let expected_len = report_id.frame_len();
        if bytes.len() != expected_len {
            return Err(ProtocolError::InvalidFrameLength {
                report_id,
                expected: expected_len,
                actual: bytes.len(),
            });
        }

        // Safe: `expected_len` is at least `SHORT_REPORT_LEN` (7) > `HEADER_LEN` (4).
        let device_index = DeviceIndex::from_raw(bytes[1]);
        let feature_index = FeatureIndex(bytes[2]);
        let func_sw = bytes[3];
        let function_id = FunctionId::from_func_sw(func_sw);
        let sw_id = SwId::from_func_sw(func_sw);

        let mut params = [0u8; VERY_LONG_PARAM_LEN];
        let param_len = report_id.param_len();
        params[..param_len].copy_from_slice(&bytes[HEADER_LEN..expected_len]);

        Ok(Self {
            report_id,
            device_index,
            feature_index,
            function_id,
            sw_id,
            params,
        })
    }

    /// Writes the exact wire bytes of this frame into `out` and returns the written byte count.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::FrameTooShort`] if `out.len() < self.report_id().frame_len()`.
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, ProtocolError> {
        let len = self.report_id.frame_len();
        if out.len() < len {
            return Err(ProtocolError::FrameTooShort {
                actual: out.len(),
                min: len,
            });
        }
        out[0] = self.report_id.as_byte();
        out[1] = self.device_index.as_byte();
        out[2] = self.feature_index.0;
        out[3] = pack_func_sw(self.function_id, self.sw_id);
        out[HEADER_LEN..len].copy_from_slice(self.params());
        Ok(len)
    }

    /// Encodes the frame into a freshly allocated `Vec<u8>` of exact report length.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.report_id.frame_len();
        let mut buf = vec![0u8; len];
        buf[0] = self.report_id.as_byte();
        buf[1] = self.device_index.as_byte();
        buf[2] = self.feature_index.0;
        buf[3] = pack_func_sw(self.function_id, self.sw_id);
        buf[HEADER_LEN..len].copy_from_slice(self.params());
        buf
    }

    /// Returns the report ID.
    #[must_use]
    pub const fn report_id(&self) -> ReportId {
        self.report_id
    }

    /// Returns the target/source device index.
    #[must_use]
    pub const fn device_index(&self) -> DeviceIndex {
        self.device_index
    }

    /// Returns the feature index (or `0xFF` for HID++ 2.0 errors).
    #[must_use]
    pub const fn feature_index(&self) -> FeatureIndex {
        self.feature_index
    }

    /// Returns the 4-bit function ID.
    #[must_use]
    pub const fn function_id(&self) -> FunctionId {
        self.function_id
    }

    /// Returns the 4-bit software ID.
    #[must_use]
    pub const fn sw_id(&self) -> SwId {
        self.sw_id
    }

    /// Returns the raw byte `[3]` (`(function_id << 4) | sw_id`).
    #[must_use]
    pub const fn raw_func_sw(&self) -> u8 {
        pack_func_sw(self.function_id, self.sw_id)
    }

    /// Returns the parameter slice sized to this frame's [`ReportId`].
    #[must_use]
    pub fn params(&self) -> &[u8] {
        &self.params[..self.report_id.param_len()]
    }
}

impl fmt::Debug for HidppFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HidppFrame")
            .field("report_id", &self.report_id)
            .field("device_index", &self.device_index)
            .field("feature_index", &self.feature_index)
            .field("function_id", &self.function_id)
            .field("sw_id", &self.sw_id)
            .field("params", &self.params())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn golden_short_frame_roundtrip() {
        let raw = [0x10u8, 0xFF, 0x00, 0x1A, 0x00, 0x00, 0x42];
        let frame = HidppFrame::decode(&raw).unwrap();
        assert_eq!(frame.report_id(), ReportId::Short);
        assert_eq!(frame.device_index(), DeviceIndex::DIRECT);
        assert_eq!(frame.feature_index(), FeatureIndex::IROOT);
        assert_eq!(frame.function_id(), FunctionId::new(1).unwrap());
        assert_eq!(frame.sw_id(), SwId::request(0x0A).unwrap());
        assert_eq!(frame.params(), &[0x00, 0x00, 0x42]);
        assert_eq!(frame.to_bytes(), raw);
    }

    #[test]
    fn golden_long_frame_roundtrip() {
        let mut raw = [0u8; LONG_REPORT_LEN];
        raw[0] = 0x11;
        raw[1] = 0x02;
        raw[2] = 0x05;
        raw[3] = 0x2B;
        raw[4] = 0x1B;
        raw[5] = 0x04;
        raw[19] = 0xFE;

        let frame = HidppFrame::decode(&raw).unwrap();
        assert_eq!(frame.report_id(), ReportId::Long);
        assert_eq!(frame.device_index(), DeviceIndex::receiver(2).unwrap());
        assert_eq!(frame.feature_index(), FeatureIndex(0x05));
        assert_eq!(frame.function_id(), FunctionId::new(2).unwrap());
        assert_eq!(frame.sw_id(), SwId::request(0x0B).unwrap());
        assert_eq!(frame.params().len(), LONG_PARAM_LEN);
        assert_eq!(frame.params()[0], 0x1B);
        assert_eq!(frame.params()[15], 0xFE);
        assert_eq!(frame.to_bytes(), raw);
    }

    #[test]
    fn golden_very_long_frame_roundtrip() {
        let mut raw = [0u8; VERY_LONG_REPORT_LEN];
        raw[0] = 0x12;
        raw[1] = 0xFF;
        raw[2] = 0x09;
        raw[3] = 0xF1;
        raw[63] = 0xA5;

        let frame = HidppFrame::decode(&raw).unwrap();
        assert_eq!(frame.report_id(), ReportId::VeryLong);
        assert_eq!(frame.params().len(), VERY_LONG_PARAM_LEN);
        assert_eq!(frame.params()[59], 0xA5);

        let mut out = [0u8; VERY_LONG_REPORT_LEN];
        let written = frame.encode_into(&mut out).unwrap();
        assert_eq!(written, VERY_LONG_REPORT_LEN);
        assert_eq!(out, raw);
    }

    #[test]
    fn rejects_malformed_inputs() {
        assert!(matches!(
            HidppFrame::decode(&[]),
            Err(ProtocolError::FrameTooShort { actual: 0, .. })
        ));
        assert!(matches!(
            HidppFrame::decode(&[0x01, 0xFF, 0x00, 0x1A, 0x00, 0x00, 0x00]),
            Err(ProtocolError::InvalidReportId(0x01))
        ));
        // Truncated short frame
        assert!(matches!(
            HidppFrame::decode(&[0x10, 0xFF, 0x00, 0x1A, 0x00]),
            Err(ProtocolError::InvalidFrameLength {
                report_id: ReportId::Short,
                expected: 7,
                actual: 5
            })
        ));
        // Oversized short frame
        assert!(matches!(
            HidppFrame::decode(&[0x10; 20]),
            Err(ProtocolError::InvalidFrameLength {
                report_id: ReportId::Short,
                expected: 7,
                actual: 20
            })
        ));
        // Truncated long frame
        assert!(matches!(
            HidppFrame::decode(&[0x11; 7]),
            Err(ProtocolError::InvalidFrameLength {
                report_id: ReportId::Long,
                expected: 20,
                actual: 7
            })
        ));
        // Oversized payload for short frame construction
        assert!(matches!(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[1, 2, 3, 4],
            ),
            Err(ProtocolError::PayloadTooLarge { len: 4, max: 3 })
        ));
    }

    #[test]
    fn validates_nibble_and_slot_constructors() {
        assert!(DeviceIndex::receiver(0).is_err());
        assert!(DeviceIndex::receiver(7).is_err());
        assert!(DeviceIndex::receiver(1).is_ok());
        assert!(DeviceIndex::receiver(6).is_ok());

        assert!(FunctionId::new(16).is_err());
        assert!(FunctionId::new(15).is_ok());

        assert!(SwId::request(0).is_err());
        assert!(SwId::request(16).is_err());
        assert!(SwId::NOTIFICATION.is_notification());
        assert!(!SwId::DEFAULT_REQUEST.is_notification());
    }

    #[test]
    fn supported_reports_selection_handles_long_only_devices() {
        assert_eq!(
            SupportedReports::SHORT_AND_LONG
                .select_for_payload(3)
                .unwrap(),
            ReportId::Short
        );
        assert_eq!(
            SupportedReports::SHORT_AND_LONG
                .select_for_payload(4)
                .unwrap(),
            ReportId::Long
        );
        assert_eq!(
            SupportedReports::LONG_ONLY.select_for_payload(3).unwrap(),
            ReportId::Long
        );
        assert!(matches!(
            SupportedReports::SHORT_AND_LONG.select_for_payload(20),
            Err(ProtocolError::NoSupportedReportForPayload { .. })
        ));
    }

    proptest! {
        #[test]
        fn frame_roundtrip_proptest(
            report_choice in 0u8..3,
            device_index in any::<u8>(),
            feature_index in any::<u8>(),
            function_id in 0u8..16,
            sw_id in 0u8..16,
            raw_params in prop::collection::vec(any::<u8>(), 60..=60),
        ) {
            let report_id = match report_choice {
                0 => ReportId::Short,
                1 => ReportId::Long,
                _ => ReportId::VeryLong,
            };
            let payload = &raw_params[..report_id.param_len()];
            let frame = HidppFrame::new(
                report_id,
                DeviceIndex::from_raw(device_index),
                FeatureIndex(feature_index),
                FunctionId::from_func_sw(function_id << 4),
                SwId::from_func_sw(sw_id),
                payload,
            ).unwrap();
            let encoded = frame.to_bytes();
            let decoded = HidppFrame::decode(&encoded).unwrap();
            prop_assert_eq!(decoded, frame);
            prop_assert_eq!(decoded.params(), payload);
        }
    }
}

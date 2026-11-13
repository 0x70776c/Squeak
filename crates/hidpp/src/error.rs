//! Typed protocol and transport error definitions for HID++ 2.0.

use core::fmt;
use std::path::PathBuf;

use thiserror::Error;

use crate::feature::FeatureId;
use crate::frame::{
    DeviceIndex, FeatureIndex, FunctionId, HIDPP10_ERROR_SUB_ID, HidppFrame, ReportId,
    SupportedReports, SwId, VERY_LONG_PARAM_LEN,
};

/// HID++ 2.0 error code reported in error frames (`feature_index == 0xFF`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hidpp20ErrorCode {
    /// `0x00`: No error.
    NoError,
    /// `0x01`: Unspecified/unknown error.
    Unknown,
    /// `0x02`: Invalid argument supplied to feature function.
    InvalidArgument,
    /// `0x03`: Parameter out of range.
    OutOfRange,
    /// `0x04`: Hardware error reported by device firmware.
    HardwareError,
    /// `0x05`: Vendor/firmware internal error.
    VendorInternal,
    /// `0x06`: Referenced feature index is not present on this device.
    InvalidFeatureIndex,
    /// `0x07`: Referenced function ID is not valid for the feature/version.
    InvalidFunctionId,
    /// `0x08`: Device or receiver is temporarily busy; caller may retry with backoff.
    Busy,
    /// `0x09`: Operation is unsupported by this device.
    Unsupported,
    /// Unrecognized or reserved HID++ 2.0 error byte (`>= 0x0A`).
    Reserved(u8),
}

impl Hidpp20ErrorCode {
    /// Parses a wire error code byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::NoError,
            0x01 => Self::Unknown,
            0x02 => Self::InvalidArgument,
            0x03 => Self::OutOfRange,
            0x04 => Self::HardwareError,
            0x05 => Self::VendorInternal,
            0x06 => Self::InvalidFeatureIndex,
            0x07 => Self::InvalidFunctionId,
            0x08 => Self::Busy,
            0x09 => Self::Unsupported,
            other => Self::Reserved(other),
        }
    }

    /// Returns the raw wire byte of this error code.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::NoError => 0x00,
            Self::Unknown => 0x01,
            Self::InvalidArgument => 0x02,
            Self::OutOfRange => 0x03,
            Self::HardwareError => 0x04,
            Self::VendorInternal => 0x05,
            Self::InvalidFeatureIndex => 0x06,
            Self::InvalidFunctionId => 0x07,
            Self::Busy => 0x08,
            Self::Unsupported => 0x09,
            Self::Reserved(other) => other,
        }
    }
}

impl fmt::Display for Hidpp20ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoError => write!(f, "no error (0x00)"),
            Self::Unknown => write!(f, "unknown error (0x01)"),
            Self::InvalidArgument => write!(f, "invalid argument (0x02)"),
            Self::OutOfRange => write!(f, "out of range (0x03)"),
            Self::HardwareError => write!(f, "hardware error (0x04)"),
            Self::VendorInternal => write!(f, "firmware internal error (0x05)"),
            Self::InvalidFeatureIndex => write!(f, "invalid feature index (0x06)"),
            Self::InvalidFunctionId => write!(f, "invalid function ID (0x07)"),
            Self::Busy => write!(f, "device busy (0x08)"),
            Self::Unsupported => write!(f, "unsupported (0x09)"),
            Self::Reserved(code) => write!(f, "reserved error (0x{code:02X})"),
        }
    }
}

/// Decoded HID++ 2.0 error response (`feature_index == 0xFF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hidpp20Error {
    /// Original feature index echoed by the device (wire byte `[3]`, via [`HidppFrame::raw_func_sw`]).
    pub feature_index: FeatureIndex,
    /// Original function ID echoed in the upper nibble of wire byte `[4]` (`params()[0]`).
    pub function_id: FunctionId,
    /// Original software ID echoed in the lower nibble of wire byte `[4]` (`params()[0]`).
    pub sw_id: SwId,
    /// HID++ 2.0 error code (wire byte `[5]`, `params()[1]`).
    pub code: Hidpp20ErrorCode,
}

impl Hidpp20Error {
    /// Attempts to interpret `frame` as an HID++ 2.0 error frame (`feature_index == 0xFF`).
    ///
    /// Wire layout of an HID++ 2.0 error frame:
    /// `[report_id, device_index, 0xFF, orig_feature_index, orig_func_sw, error_code, ...]`
    /// Notice byte `[3]` carries `orig_feature_index` (not `(fn << 4) | sw_id`), byte `[4]`
    /// (`params()[0]`) carries `orig_func_sw`, and byte `[5]` (`params()[1]`) carries `error_code`.
    #[must_use]
    pub fn from_frame(frame: &HidppFrame) -> Option<Self> {
        if !frame.feature_index().is_error() {
            return None;
        }
        let params = frame.params();
        let orig_feature = frame.raw_func_sw();
        let orig_func_sw = *params.first()?;
        let raw_code = *params.get(1)?;
        Some(Self {
            feature_index: FeatureIndex(orig_feature),
            function_id: FunctionId::from_func_sw(orig_func_sw),
            sw_id: SwId::from_func_sw(orig_func_sw),
            code: Hidpp20ErrorCode::from_byte(raw_code),
        })
    }
}

impl fmt::Display for Hidpp20Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HID++ 2.0 error on feature {} {}: {}",
            self.feature_index, self.function_id, self.code
        )
    }
}

impl std::error::Error for Hidpp20Error {}

/// HID++ 1.0 error codes (`ERROR_MSG`, sub-ID `0x8F`).
///
/// Legacy HID++ 1.0 devices are out of scope for Squeak (`InvalidSubId` etc. during `IRoot`
/// ping classify the target as an unsupported legacy device). USB/Bolt receivers (`device_index`
/// `1..=6`) also emit `0x8F` short frames with `UnknownDevice`/`ResourceError`/`ConnectFail`
/// when a paired HID++ 2.0 wireless device is offline, or `Busy` (`0x07`) when temporarily busy.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hidpp10ErrorCode {
    /// `0x00`: Success.
    Success,
    /// `0x01`: Invalid sub-ID (typical response of HID++ 1.0 devices to HID++ 2.0 `IRoot`).
    InvalidSubId,
    /// `0x02`: Invalid register/address.
    InvalidAddress,
    /// `0x03`: Invalid value.
    InvalidValue,
    /// `0x04`: Connection failure.
    ConnectFail,
    /// `0x05`: Too many paired devices.
    TooManyDevices,
    /// `0x06`: Entry already exists.
    AlreadyExists,
    /// `0x07`: Device busy.
    Busy,
    /// `0x08`: Unknown device.
    UnknownDevice,
    /// `0x09`: Resource error.
    ResourceError,
    /// `0x0A`: Request unavailable in current state.
    RequestUnavailable,
    /// `0x0B`: Invalid parameter value.
    InvalidParamValue,
    /// `0x0C`: Wrong PIN code.
    WrongPinCode,
    /// Other/vendor-specific HID++ 1.0 error code.
    Other(u8),
}

impl Hidpp10ErrorCode {
    /// Parses an HID++ 1.0 wire error byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Success,
            0x01 => Self::InvalidSubId,
            0x02 => Self::InvalidAddress,
            0x03 => Self::InvalidValue,
            0x04 => Self::ConnectFail,
            0x05 => Self::TooManyDevices,
            0x06 => Self::AlreadyExists,
            0x07 => Self::Busy,
            0x08 => Self::UnknownDevice,
            0x09 => Self::ResourceError,
            0x0A => Self::RequestUnavailable,
            0x0B => Self::InvalidParamValue,
            0x0C => Self::WrongPinCode,
            other => Self::Other(other),
        }
    }

    /// Returns the raw byte representation.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Success => 0x00,
            Self::InvalidSubId => 0x01,
            Self::InvalidAddress => 0x02,
            Self::InvalidValue => 0x03,
            Self::ConnectFail => 0x04,
            Self::TooManyDevices => 0x05,
            Self::AlreadyExists => 0x06,
            Self::Busy => 0x07,
            Self::UnknownDevice => 0x08,
            Self::ResourceError => 0x09,
            Self::RequestUnavailable => 0x0A,
            Self::InvalidParamValue => 0x0B,
            Self::WrongPinCode => 0x0C,
            Self::Other(other) => other,
        }
    }
}

impl fmt::Display for Hidpp10ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HID++ 1.0 error 0x{:02X} ({:?})", self.as_byte(), self)
    }
}

/// Decoded HID++ 1.0 legacy `ERROR_MSG` (`0x8F`) response frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hidpp10Error {
    /// Echoed sub-ID (wire byte `[3]`, equals request byte `[2]`, e.g. `0x00` for `IRoot`).
    pub sub_id: u8,
    /// Echoed register/address (wire byte `[4]` / `params()[0]`, equals request byte `[3]` `(func<<4) | sw_id`).
    pub address: u8,
    /// HID++ 1.0 error code (wire byte `[5]` / `params()[1]`).
    pub code: Hidpp10ErrorCode,
}

impl Hidpp10Error {
    /// Attempts to interpret `frame` as an HID++ 1.0 `ERROR_MSG` (`0x8F`) frame.
    #[must_use]
    pub fn from_frame(frame: &HidppFrame) -> Option<Self> {
        if frame.feature_index().0 != HIDPP10_ERROR_SUB_ID {
            return None;
        }
        let params = frame.params();
        Some(Self {
            sub_id: frame.raw_func_sw(),
            address: *params.first()?,
            code: Hidpp10ErrorCode::from_byte(*params.get(1)?),
        })
    }
}

/// Errors produced by the underlying [`crate::HidTransport`] layer.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum TransportError {
    /// Opening or accessing the `hidraw` node failed due to missing permissions (`EACCES`/`EPERM`).
    #[error("permission denied opening {path} (install udev rule via `squeak setup-udev`)")]
    PermissionDenied {
        /// Path to the `hidraw` device node.
        path: PathBuf,
    },

    /// The requested `hidraw` device node does not exist.
    #[error("device node not found: {path}")]
    DeviceNotFound {
        /// Path that could not be opened.
        path: PathBuf,
    },

    /// The device was disconnected or returned `ENODEV`/`ENXIO`/`EIO` during I/O.
    #[error("device disconnected")]
    Disconnected,

    /// Reading a response frame exceeded the caller's deadline (device asleep or unresponsive).
    #[error("timed out waiting for device response")]
    Timeout,

    /// Underlying device does not expose a compatible HID++ report interface.
    #[error("device {path} does not support HID++ ({reason})")]
    NotSupportedHidpp {
        /// Path to the inspected `hidraw` node.
        path: PathBuf,
        /// Explanation of why the node was rejected.
        reason: String,
    },

    /// Lower-level OS or descriptor inspection I/O failure.
    #[error("transport I/O error ({context}): {source}")]
    Io {
        /// Human-readable operation context.
        context: String,
        /// Underlying OS I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Errors produced while validating, parsing, or sequencing HID++ 2.0 protocol frames.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Unrecognized HID report ID byte (expected `0x10`, `0x11`, or `0x12`).
    #[error("invalid HID++ report ID 0x{0:02X}")]
    InvalidReportId(u8),

    /// Raw frame length did not match the fixed length required for its report ID.
    #[error("invalid length for report {report_id}: expected {expected} bytes, got {actual}")]
    InvalidFrameLength {
        /// Parsed report ID.
        report_id: ReportId,
        /// Required wire length in bytes.
        expected: usize,
        /// Actual byte length received.
        actual: usize,
    },

    /// Buffer shorter than the minimum HID++ report size (`7` bytes) or target output slice.
    #[error("buffer too short: got {actual} bytes, need at least {min}")]
    FrameTooShort {
        /// Actual buffer length.
        actual: usize,
        /// Minimum required length.
        min: usize,
    },

    /// Parameter payload exceeded the capacity of a report frame.
    #[error("payload length {len} exceeds maximum capacity {max}")]
    PayloadTooLarge {
        /// Provided payload byte length.
        len: usize,
        /// Capacity of the target report (at most `VERY_LONG_PARAM_LEN` = 60).
        max: usize,
    },

    /// The device does not advertise any HID++ report ID large enough for the request payload.
    #[error("device reports {supported:?} cannot carry {payload_len}-byte payload")]
    NoSupportedReportForPayload {
        /// Payload length requested.
        payload_len: usize,
        /// Reports advertised by the device descriptor.
        supported: SupportedReports,
    },

    /// Receiver slot outside the valid range `1..=6`.
    #[error("invalid receiver device slot {0} (expected 1..=6)")]
    InvalidReceiverSlot(u8),

    /// 4-bit function ID outside `0..=15`.
    #[error("invalid HID++ function ID {0} (expected 0..=15)")]
    InvalidFunctionId(u8),

    /// Host request software ID must be non-zero (`1..=15`), since `0` is reserved for events.
    #[error("invalid request software ID {0} (expected 1..=15)")]
    InvalidRequestSwId(u8),

    /// Response payload had fewer bytes than required by the feature function parser.
    #[error(
        "truncated feature response: expected at least {expected} parameter bytes, got {actual}"
    )]
    ResponseTruncated {
        /// Minimum required parameter bytes.
        expected: usize,
        /// Actual available parameter bytes.
        actual: usize,
    },

    /// Device responded with malformed or mismatched fields for the queried feature function.
    #[error("unexpected feature response: {detail}")]
    UnexpectedResponse {
        /// Diagnostic details about the mismatch.
        detail: String,
    },

    /// Device responded with an HID++ 1.0 `ERROR_MSG` (`0x8F`) or `< 2.0` protocol version.
    #[error("legacy HID++ 1.0 device is out of scope ({error_code})")]
    LegacyDeviceNotSupported {
        /// Reported HID++ 1.0 error code.
        error_code: Hidpp10ErrorCode,
    },

    /// Receiver slot reports target wireless device is offline, asleep, or unpaired (`0x8F`).
    #[error("receiver device {device_index} is offline or unavailable ({error_code})")]
    ReceiverDeviceOffline {
        /// Receiver slot address (`1..=6`).
        device_index: DeviceIndex,
        /// Reported HID++ 1.0 error code (`UnknownDevice`, `ResourceError`, or `ConnectFail`).
        error_code: Hidpp10ErrorCode,
    },

    /// Requested feature index exceeds the count reported by `IFeatureSet::getCount`.
    #[error("feature index {requested} out of bounds for device feature count {count}")]
    FeatureIndexOutOfBounds {
        /// Queried feature index (`1..=count`).
        requested: u8,
        /// Count returned by `IFeatureSet::getCount`.
        count: u8,
    },

    /// Required feature is missing from the device's runtime feature table.
    #[error("device does not implement feature {feature_id}")]
    FeatureNotSupported {
        /// Missing feature identifier.
        feature_id: FeatureId,
    },

    /// Device reports a newer feature version than this driver supports for safe writes.
    #[error(
        "feature {feature_id} reports version {version} > max supported {max_supported}; write refused (read-only fallback)"
    )]
    UnsupportedFeatureVersion {
        /// Feature identifier.
        feature_id: FeatureId,
        /// Firmware-reported feature revision.
        version: u8,
        /// Highest revision verified safe for mutating writes.
        max_supported: u8,
    },

    /// Caller supplied an out-of-range or unsupported parameter value.
    #[error("invalid protocol argument: {detail}")]
    InvalidArgument {
        /// Explanation of the invalid value.
        detail: String,
    },

    /// Post-write readback from the device did not match the requested value.
    #[error("readback verification failed for {feature_id}: expected {expected}, got {actual}")]
    ReadbackMismatch {
        /// Feature identifier.
        feature_id: FeatureId,
        /// Requested target value.
        expected: String,
        /// Value observed during post-write readback.
        actual: String,
    },

    /// Device returned [`Hidpp20ErrorCode::Busy`] past the configured retry budget.
    #[error(
        "device busy retries exhausted for feature {feature_index} {function_id} after {attempts} attempts"
    )]
    BusyRetriesExhausted {
        /// Target feature index.
        feature_index: FeatureIndex,
        /// Target function ID.
        function_id: FunctionId,
        /// Total attempts performed.
        attempts: u32,
    },

    /// Device returned an explicit HID++ 2.0 protocol error frame (`0xFF`).
    #[error("{0}")]
    DeviceError(#[from] Hidpp20Error),

    /// USB HID report descriptor could not be parsed cleanly.
    #[error("invalid HID report descriptor: {0}")]
    InvalidDescriptor(String),
}

// Keep `VERY_LONG_PARAM_LEN` referenced in doc/type invariants.
const _: usize = VERY_LONG_PARAM_LEN;

/// Unified error returned by [`crate::HidppSession`] and feature discovery operations.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum HidppError {
    /// Transport-level failure (permissions, timeout, disconnect, OS error).
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// Protocol-level failure (framing, HID++ 2.0 error code, legacy 1.0 device, parse error).
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

impl From<Hidpp20Error> for HidppError {
    fn from(err: Hidpp20Error) -> Self {
        Self::Protocol(ProtocolError::DeviceError(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::LONG_REPORT_LEN;

    #[test]
    fn golden_hidpp20_short_error_frame() {
        // Short error frame: feature 0xFF, orig feature 0x05, orig func 2 / sw 0xA, error OutOfRange (0x03)
        let raw = [0x10u8, 0xFF, 0xFF, 0x05, 0x2A, 0x03, 0x00];
        let frame = HidppFrame::decode(&raw).unwrap();
        let err = Hidpp20Error::from_frame(&frame).unwrap();
        assert_eq!(err.feature_index, FeatureIndex(0x05));
        assert_eq!(err.function_id, FunctionId::new(2).unwrap());
        assert_eq!(err.sw_id, SwId::request(0x0A).unwrap());
        assert_eq!(err.code, Hidpp20ErrorCode::OutOfRange);
    }

    #[test]
    fn golden_hidpp20_long_error_frame() {
        // Some devices reply with a 20B long error frame carrying Busy (0x08).
        let mut raw = [0u8; LONG_REPORT_LEN];
        raw[0] = 0x11;
        raw[1] = 0x01;
        raw[2] = 0xFF;
        raw[3] = 0x00;
        raw[4] = 0x1A;
        raw[5] = 0x08;
        let frame = HidppFrame::decode(&raw).unwrap();
        let err = Hidpp20Error::from_frame(&frame).unwrap();
        assert_eq!(err.feature_index, FeatureIndex::IROOT);
        assert_eq!(err.function_id, FunctionId::new(1).unwrap());
        assert_eq!(err.sw_id, SwId::DEFAULT_REQUEST);
        assert_eq!(err.code, Hidpp20ErrorCode::Busy);
    }

    #[test]
    fn golden_hidpp10_legacy_error_frame() {
        // Legacy device replying ERR_INVALID_SUBID (0x01) to IRoot ping (0x00, 0x1A).
        let raw = [0x10u8, 0xFF, 0x8F, 0x00, 0x1A, 0x01, 0x00];
        let frame = HidppFrame::decode(&raw).unwrap();
        assert!(Hidpp20Error::from_frame(&frame).is_none());
        let err10 = Hidpp10Error::from_frame(&frame).unwrap();
        assert_eq!(err10.sub_id, 0x00);
        assert_eq!(err10.address, 0x1A);
        assert_eq!(err10.code, Hidpp10ErrorCode::InvalidSubId);
    }
}

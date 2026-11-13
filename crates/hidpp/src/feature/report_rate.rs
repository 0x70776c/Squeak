//! `REPORT_RATE` (`0x8060`) and `EXTENDED_REPORT_RATE` (`0x8061`) feature drivers.

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_STD_GET_LIST: u8 = 0;
const FN_STD_GET_RATE: u8 = 1;
const FN_STD_SET_RATE: u8 = 2;

const FN_EXT_GET_LIST: u8 = 1;
const FN_EXT_GET_RATE: u8 = 2;
const FN_EXT_SET_RATE: u8 = 3;

/// Polling report rate supported by `REPORT_RATE` (`0x8060`) and `EXTENDED_REPORT_RATE` (`0x8061`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReportRate {
    /// `8 ms` polling interval (`125 Hz`).
    Hz125,
    /// `4 ms` polling interval (`250 Hz`).
    Hz250,
    /// `2 ms` polling interval (`500 Hz`).
    Hz500,
    /// `1 ms` polling interval (`1000 Hz`).
    Hz1000,
    /// `500 us` polling interval (`2000 Hz`, `0x8061` only).
    Hz2000,
    /// `250 us` polling interval (`4000 Hz`, `0x8061` only).
    Hz4000,
    /// `125 us` polling interval (`8000 Hz`, `0x8061` only).
    Hz8000,
    /// Non-standard millisecond interval (`3`, `5`, `6`, or `7` ms) from `0x8060`.
    OtherMs(u8),
}

impl ReportRate {
    /// Parses a rate from its polling frequency in Hz (e.g., `125`, `250`, `500`, `1000`, `2000`, `4000`, `8000`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `hz` is not a standard HID++ polling frequency.
    pub fn from_hz(hz: u16) -> Result<Self, ProtocolError> {
        match hz {
            125 => Ok(Self::Hz125),
            250 => Ok(Self::Hz250),
            500 => Ok(Self::Hz500),
            1000 => Ok(Self::Hz1000),
            2000 => Ok(Self::Hz2000),
            4000 => Ok(Self::Hz4000),
            8000 => Ok(Self::Hz8000),
            _ => Err(ProtocolError::InvalidArgument {
                detail: format!(
                    "unsupported report rate {hz} Hz (expected 125, 250, 500, 1000, 2000, 4000, or 8000)"
                ),
            }),
        }
    }

    /// Parses a rate from the `0x8060` millisecond interval byte (`1..=8`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedResponse`] if `ms` is `0` or `> 8`.
    pub fn from_ms(ms: u8) -> Result<Self, ProtocolError> {
        match ms {
            1 => Ok(Self::Hz1000),
            2 => Ok(Self::Hz500),
            4 => Ok(Self::Hz250),
            8 => Ok(Self::Hz125),
            3 | 5..=7 => Ok(Self::OtherMs(ms)),
            _ => Err(ProtocolError::UnexpectedResponse {
                detail: format!("invalid 0x8060 report rate interval {ms} ms (expected 1..=8)"),
            }),
        }
    }

    /// Returns the `0x8060` millisecond interval byte (`1..=8`), if representable.
    #[must_use]
    pub const fn to_ms(self) -> Option<u8> {
        match self {
            Self::Hz1000 => Some(1),
            Self::Hz500 => Some(2),
            Self::Hz250 => Some(4),
            Self::Hz125 => Some(8),
            Self::OtherMs(ms) if ms >= 1 && ms <= 8 => Some(ms),
            _ => None,
        }
    }

    /// Parses a rate from the `0x8061` extended report-rate index (`0..=6`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedResponse`] if `code > 6`.
    pub fn from_extended_code(code: u8) -> Result<Self, ProtocolError> {
        match code {
            0 => Ok(Self::Hz125),
            1 => Ok(Self::Hz250),
            2 => Ok(Self::Hz500),
            3 => Ok(Self::Hz1000),
            4 => Ok(Self::Hz2000),
            5 => Ok(Self::Hz4000),
            6 => Ok(Self::Hz8000),
            _ => Err(ProtocolError::UnexpectedResponse {
                detail: format!("invalid 0x8061 extended report rate code {code} (expected 0..=6)"),
            }),
        }
    }

    /// Returns the `0x8061` extended report-rate code (`0..=6`), if representable.
    #[must_use]
    pub const fn to_extended_code(self) -> Option<u8> {
        match self {
            Self::Hz125 => Some(0),
            Self::Hz250 => Some(1),
            Self::Hz500 => Some(2),
            Self::Hz1000 => Some(3),
            Self::Hz2000 => Some(4),
            Self::Hz4000 => Some(5),
            Self::Hz8000 => Some(6),
            Self::OtherMs(_) => None,
        }
    }

    /// Approximate polling frequency in Hz.
    #[must_use]
    pub const fn hz(self) -> u16 {
        match self {
            Self::Hz125 => 125,
            Self::Hz250 => 250,
            Self::Hz500 => 500,
            Self::Hz1000 => 1000,
            Self::Hz2000 => 2000,
            Self::Hz4000 => 4000,
            Self::Hz8000 => 8000,
            Self::OtherMs(ms) => {
                if ms == 0 {
                    0
                } else {
                    1000 / (ms as u16)
                }
            }
        }
    }

    /// Polling interval in microseconds.
    #[must_use]
    pub const fn interval_us(self) -> u16 {
        match self {
            Self::Hz125 => 8000,
            Self::Hz250 => 4000,
            Self::Hz500 => 2000,
            Self::Hz1000 => 1000,
            Self::Hz2000 => 500,
            Self::Hz4000 => 250,
            Self::Hz8000 => 125,
            Self::OtherMs(ms) => (ms as u16) * 1000,
        }
    }
}

impl fmt::Display for ReportRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OtherMs(ms) => write!(f, "{} Hz ({ms} ms)", self.hz()),
            _ if self.interval_us() >= 1000 => {
                write!(f, "{} Hz ({} ms)", self.hz(), self.interval_us() / 1000)
            }
            _ => write!(f, "{} Hz ({} us)", self.hz(), self.interval_us()),
        }
    }
}

/// Snapshot of supported and active report rates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRateState {
    /// Feature that served this state (`0x8060` or `0x8061`).
    pub source_feature: FeatureId,
    /// Currently active polling rate.
    pub current: ReportRate,
    /// Supported polling rates advertised by device firmware.
    pub supported: Vec<ReportRate>,
}

/// Result of a verified report-rate write + readback check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportRateWriteVerification {
    /// Feature that performed the write (`0x8060` or `0x8061`).
    pub source_feature: FeatureId,
    /// Target rate requested.
    pub requested: ReportRate,
    /// Rate confirmed by immediate post-write readback.
    pub readback: ReportRate,
}

/// Trait implemented by versioned `REPORT_RATE` (`0x8060`) and `EXTENDED_REPORT_RATE` (`0x8061`)
/// drivers.
pub trait ReportRateFeature {
    /// Queries supported report rates advertised by the device.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_report_rate_list<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<Vec<ReportRate>, HidppError>;

    /// Reads the currently active report rate.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_report_rate<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<ReportRate, HidppError>;

    /// Sets `rate` after validating version support and device capabilities, then performs
    /// immediate readback verification.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if the feature version is unsupported for writes, `rate` is
    /// unsupported by the device, the write fails, or readback mismatches `rate`.
    fn set_report_rate_verified<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        rate: ReportRate,
    ) -> Result<ReportRateWriteVerification, HidppError>;
}

/// Versioned runtime driver for `REPORT_RATE` (`0x8060`).
#[derive(Debug, Clone, Copy)]
pub struct ReportRateDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for ReportRateDriver {
    const FEATURE_ID: FeatureId = FeatureId::REPORT_RATE;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl ReportRateDriver {
    /// Creates a driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::REPORT_RATE`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected REPORT_RATE (0x8060), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Creates a driver directly from runtime feature index and version.
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

    /// Returns `true` if mutating writes are supported for this firmware version.
    #[must_use]
    pub const fn supports_writes(&self) -> bool {
        self.version == Self::MAX_SUPPORTED_VERSION
    }

    /// Encodes `setReportRate` (`fn2`) request parameters for `0x8060`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `rate` cannot be represented as a 1..=8 ms
    /// interval (e.g., 2000–8000 Hz rates require `0x8061`).
    pub fn encode_set_rate_params(rate: ReportRate) -> Result<[u8; 1], ProtocolError> {
        let ms = rate.to_ms().ok_or_else(|| ProtocolError::InvalidArgument {
            detail: format!("{rate} requires EXTENDED_REPORT_RATE (0x8061)"),
        })?;
        Ok([ms])
    }

    /// Pure parser for `0x8060` `getReportRateList` (`fn0`) bitmask response.
    ///
    /// Bit `i` (`0..=7`) corresponds to `(i + 1)` ms interval.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty.
    pub fn parse_rate_list_response(params: &[u8]) -> Result<Vec<ReportRate>, ProtocolError> {
        let Some(&mask) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        let mut rates = Vec::new();
        for bit in 0..8u8 {
            if (mask & (1 << bit)) != 0 {
                rates.push(ReportRate::from_ms(bit + 1)?);
            }
        }
        rates.sort_by_key(|r| r.hz());
        Ok(rates)
    }

    /// Pure parser for `0x8060` `getReportRate` (`fn1`) response.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty, or
    /// [`ProtocolError::UnexpectedResponse`] if interval byte is outside `1..=8`.
    pub fn parse_get_rate_response(params: &[u8]) -> Result<ReportRate, ProtocolError> {
        let Some(&ms) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        ReportRate::from_ms(ms)
    }
}

impl ReportRateFeature for ReportRateDriver {
    fn get_report_rate_list<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<Vec<ReportRate>, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_STD_GET_LIST)?,
            &[],
        )?;
        Ok(Self::parse_rate_list_response(reply.params())?)
    }

    fn get_report_rate<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<ReportRate, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_STD_GET_RATE)?,
            &[],
        )?;
        Ok(Self::parse_get_rate_response(reply.params())?)
    }

    fn set_report_rate_verified<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        rate: ReportRate,
    ) -> Result<ReportRateWriteVerification, HidppError> {
        if !self.supports_writes() {
            return Err(ProtocolError::UnsupportedFeatureVersion {
                feature_id: Self::FEATURE_ID,
                version: self.version,
                max_supported: Self::MAX_SUPPORTED_VERSION,
            }
            .into());
        }

        let supported = self.get_report_rate_list(session, device_index)?;
        if !supported.contains(&rate) {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("requested report rate {rate} is not supported by this device"),
            }
            .into());
        }

        let params = Self::encode_set_rate_params(rate)?;
        let _reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_STD_SET_RATE)?,
            &params,
        )?;

        let readback = self.get_report_rate(session, device_index)?;
        if readback != rate {
            return Err(ProtocolError::ReadbackMismatch {
                feature_id: Self::FEATURE_ID,
                expected: rate.to_string(),
                actual: readback.to_string(),
            }
            .into());
        }

        Ok(ReportRateWriteVerification {
            source_feature: Self::FEATURE_ID,
            requested: rate,
            readback,
        })
    }
}

/// Versioned runtime driver for `EXTENDED_REPORT_RATE` (`0x8061`).
#[derive(Debug, Clone, Copy)]
pub struct ExtendedReportRateDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for ExtendedReportRateDriver {
    const FEATURE_ID: FeatureId = FeatureId::EXTENDED_REPORT_RATE;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl ExtendedReportRateDriver {
    /// Creates a driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not
    /// [`FeatureId::EXTENDED_REPORT_RATE`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected EXTENDED_REPORT_RATE (0x8061), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Creates a driver directly from runtime feature index and version.
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

    /// Returns `true` if mutating writes are supported for this firmware version.
    #[must_use]
    pub const fn supports_writes(&self) -> bool {
        self.version == Self::MAX_SUPPORTED_VERSION
    }

    /// Encodes `setReportRate` (`fn3`) request parameters for `0x8061`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `rate` has no `0x8061` extended code.
    pub fn encode_set_rate_params(rate: ReportRate) -> Result<[u8; 1], ProtocolError> {
        let code = rate
            .to_extended_code()
            .ok_or_else(|| ProtocolError::InvalidArgument {
                detail: format!("{rate} cannot be encoded for EXTENDED_REPORT_RATE (0x8061)"),
            })?;
        Ok([code])
    }

    /// Pure parser for `0x8061` `getExtendedReportRateList` (`fn1`) 16-bit bitmask response.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 2 parameter bytes exist.
    pub fn parse_rate_list_response(params: &[u8]) -> Result<Vec<ReportRate>, ProtocolError> {
        if params.len() < 2 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 2,
                actual: params.len(),
            });
        }
        let mask = u16::from_be_bytes([params[0], params[1]]);
        let mut rates = Vec::new();
        for bit in 0..7u8 {
            if (mask & (1 << bit)) != 0 {
                rates.push(ReportRate::from_extended_code(bit)?);
            }
        }
        rates.sort_by_key(|r| r.hz());
        Ok(rates)
    }

    /// Pure parser for `0x8061` `getReportRate` (`fn2`) response.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty, or
    /// [`ProtocolError::UnexpectedResponse`] if code is outside `0..=6`.
    pub fn parse_get_rate_response(params: &[u8]) -> Result<ReportRate, ProtocolError> {
        let Some(&code) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        ReportRate::from_extended_code(code)
    }
}

impl ReportRateFeature for ExtendedReportRateDriver {
    fn get_report_rate_list<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<Vec<ReportRate>, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_EXT_GET_LIST)?,
            &[],
        )?;
        Ok(Self::parse_rate_list_response(reply.params())?)
    }

    fn get_report_rate<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<ReportRate, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_EXT_GET_RATE)?,
            &[],
        )?;
        Ok(Self::parse_get_rate_response(reply.params())?)
    }

    fn set_report_rate_verified<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        rate: ReportRate,
    ) -> Result<ReportRateWriteVerification, HidppError> {
        if !self.supports_writes() {
            return Err(ProtocolError::UnsupportedFeatureVersion {
                feature_id: Self::FEATURE_ID,
                version: self.version,
                max_supported: Self::MAX_SUPPORTED_VERSION,
            }
            .into());
        }

        let supported = self.get_report_rate_list(session, device_index)?;
        if !supported.contains(&rate) {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("requested report rate {rate} is not supported by this device"),
            }
            .into());
        }

        let params = Self::encode_set_rate_params(rate)?;
        let _reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_EXT_SET_RATE)?,
            &params,
        )?;

        let readback = self.get_report_rate(session, device_index)?;
        if readback != rate {
            return Err(ProtocolError::ReadbackMismatch {
                feature_id: Self::FEATURE_ID,
                expected: rate.to_string(),
                actual: readback.to_string(),
            }
            .into());
        }

        Ok(ReportRateWriteVerification {
            source_feature: Self::FEATURE_ID,
            requested: rate,
            readback,
        })
    }
}

/// Reads supported and active report rates using `EXTENDED_REPORT_RATE` (`0x8061`) if available,
/// falling back to `REPORT_RATE` (`0x8060`), or `Ok(None)` if neither feature is present.
///
/// # Errors
///
/// Propagates any [`HidppError`] from the underlying query.
pub fn read_report_rate<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
) -> Result<Option<ReportRateState>, HidppError> {
    if let Some(entry) = table.get(FeatureId::EXTENDED_REPORT_RATE) {
        let driver = ExtendedReportRateDriver::from_entry(entry)?;
        let supported = driver.get_report_rate_list(session, table.device_index())?;
        let current = driver.get_report_rate(session, table.device_index())?;
        return Ok(Some(ReportRateState {
            source_feature: FeatureId::EXTENDED_REPORT_RATE,
            current,
            supported,
        }));
    }
    if let Some(entry) = table.get(FeatureId::REPORT_RATE) {
        let driver = ReportRateDriver::from_entry(entry)?;
        let supported = driver.get_report_rate_list(session, table.device_index())?;
        let current = driver.get_report_rate(session, table.device_index())?;
        return Ok(Some(ReportRateState {
            source_feature: FeatureId::REPORT_RATE,
            current,
            supported,
        }));
    }
    Ok(None)
}

/// Sets and verifies the report rate using `EXTENDED_REPORT_RATE` (`0x8061`) if available,
/// falling back to `REPORT_RATE` (`0x8060`).
///
/// # Errors
///
/// Returns [`ProtocolError::FeatureNotSupported`] if neither feature exists, or propagates
/// validation/write/readback errors.
pub fn set_report_rate_verified<T: HidTransport>(
    session: &mut HidppSession<T>,
    table: &FeatureTable,
    rate: ReportRate,
) -> Result<ReportRateWriteVerification, HidppError> {
    if let Some(entry) = table.get(FeatureId::EXTENDED_REPORT_RATE) {
        let driver = ExtendedReportRateDriver::from_entry(entry)?;
        return driver.set_report_rate_verified(session, table.device_index(), rate);
    }
    if let Some(entry) = table.get(FeatureId::REPORT_RATE) {
        let driver = ReportRateDriver::from_entry(entry)?;
        return driver.set_report_rate_verified(session, table.device_index(), rate);
    }
    Err(ProtocolError::FeatureNotSupported {
        feature_id: FeatureId::REPORT_RATE,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;
    use crate::feature::{FeatureFlags, ProtocolVersion};
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
    fn golden_standard_report_rate_read_and_verified_write() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x0B);

        // 0x8060 bitmask 0x8B = bits 0 (1ms/1000Hz), 1 (2ms/500Hz), 3 (4ms/250Hz), 7 (8ms/125Hz)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x8B, 0x00, 0x00],
            )
            .unwrap(),
        );
        // setReportRate(2 ms = 500 Hz)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        // readback getReportRate -> 2 ms (500 Hz)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 2 },
            vec![FeatureEntry {
                index: feat_idx,
                id: FeatureId::REPORT_RATE,
                version: 0,
                flags: FeatureFlags::default(),
            }],
        );

        let verified = set_report_rate_verified(&mut session, &table, ReportRate::Hz500).unwrap();
        assert_eq!(verified.source_feature, FeatureId::REPORT_RATE);
        assert_eq!(verified.readback, ReportRate::Hz500);

        let sent = session.transport().written_frames();
        assert_eq!(sent[1].params()[0], 2);
    }

    #[test]
    fn golden_extended_report_rate_up_to_8khz() {
        let info = HidDeviceInfo::mock(0x046D, 0xC099, "8kHz Mouse");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x0C);

        // 0x8061 bitmask 0x007F -> bits 0..=6 (125 Hz .. 8000 Hz)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x7F],
            )
            .unwrap(),
        );
        // fn2 getReportRate -> 6 (8000 Hz)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x06],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let table = FeatureTable::from_entries(
            DeviceIndex::DIRECT,
            ProtocolVersion { major: 4, minor: 5 },
            vec![FeatureEntry {
                index: feat_idx,
                id: FeatureId::EXTENDED_REPORT_RATE,
                version: 0,
                flags: FeatureFlags::default(),
            }],
        );

        let state = read_report_rate(&mut session, &table).unwrap().unwrap();
        assert_eq!(state.source_feature, FeatureId::EXTENDED_REPORT_RATE);
        assert_eq!(state.current, ReportRate::Hz8000);
        assert_eq!(state.supported.len(), 7);
        assert!(state.supported.contains(&ReportRate::Hz8000));
    }

    #[test]
    fn refuses_unsupported_rate_and_future_version_write() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x0B);
        // Only 1000Hz (bit 0) and 500Hz (bit 1) supported
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x03, 0x00, 0x00],
            )
            .unwrap(),
        );
        let mut session = test_session(mock);
        let driver = ReportRateDriver::new(feat_idx, 0);
        let err = driver
            .set_report_rate_verified(&mut session, DeviceIndex::DIRECT, ReportRate::Hz125)
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::InvalidArgument { .. })
        ));

        // Future version refuses write immediately
        let future_ext = ExtendedReportRateDriver::new(feat_idx, 3);
        let err2 = future_ext
            .set_report_rate_verified(&mut session, DeviceIndex::DIRECT, ReportRate::Hz1000)
            .unwrap_err();
        assert!(matches!(
            err2,
            HidppError::Protocol(ProtocolError::UnsupportedFeatureVersion {
                feature_id: FeatureId::EXTENDED_REPORT_RATE,
                version: 3,
                ..
            })
        ));
    }

    proptest! {
        #[test]
        fn standard_ms_roundtrip(ms in 1u8..=8) {
            let rate = ReportRate::from_ms(ms).unwrap();
            prop_assert_eq!(rate.to_ms(), Some(ms));
            let parsed = ReportRateDriver::parse_get_rate_response(&[ms]).unwrap();
            prop_assert_eq!(parsed, rate);
        }

        #[test]
        fn extended_code_roundtrip(code in 0u8..=6) {
            let rate = ReportRate::from_extended_code(code).unwrap();
            prop_assert_eq!(rate.to_extended_code(), Some(code));
            let parsed = ExtendedReportRateDriver::parse_get_rate_response(&[code]).unwrap();
            prop_assert_eq!(parsed, rate);
        }
    }
}

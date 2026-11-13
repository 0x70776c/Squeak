//! `ADJUSTABLE_DPI` (`0x2201`) feature driver with capability validation and readback verification.

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_SENSOR_COUNT: u8 = 0;
const FN_GET_SENSOR_DPI_LIST: u8 = 1;
const FN_GET_SENSOR_DPI: u8 = 2;
const FN_SET_SENSOR_DPI: u8 = 3;

/// Maximum numeric DPI representable in `0x2201` before the `0xE000` step-marker prefix.
pub const MAX_PROTOCOL_DPI: u16 = 0xDFFF;

/// Validated sensor DPI resolution (`1..=57343`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dpi(u16);

impl Dpi {
    /// Minimum valid protocol DPI value (`1`).
    pub const MIN: Self = Self(1);
    /// Maximum valid protocol DPI value (`57343` / `0xDFFF`).
    pub const MAX: Self = Self(MAX_PROTOCOL_DPI);

    /// Validates and wraps a raw DPI value (`1..=57343`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `value == 0` or `value > 0xDFFF`.
    pub fn new(value: u16) -> Result<Self, ProtocolError> {
        if (1..=MAX_PROTOCOL_DPI).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ProtocolError::InvalidArgument {
                detail: format!("DPI {value} out of valid protocol range 1..={MAX_PROTOCOL_DPI}"),
            })
        }
    }

    /// Returns the raw numeric DPI.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Dpi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} DPI", self.0)
    }
}

/// Stepped continuous DPI range `[min..=max]` encoded via `0xE000 | step` in `getSensorDpiList`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DpiRange {
    /// Minimum DPI in this range (inclusive).
    pub min: Dpi,
    /// Maximum DPI in this range (inclusive).
    pub max: Dpi,
    /// Step size in DPI (`1..=8191`).
    pub step: u16,
}

impl DpiRange {
    /// Returns `true` if `dpi` falls within `[min..=max]` aligned to `step`.
    #[must_use]
    pub const fn contains(self, dpi: Dpi) -> bool {
        let v = dpi.as_u16();
        let min = self.min.as_u16();
        let max = self.max.as_u16();
        v >= min && v <= max && self.step > 0 && (v - min).is_multiple_of(self.step)
    }
}

/// Supported DPI entries reported by `ADJUSTABLE_DPI::getSensorDpiList` (`fn1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiCapabilities {
    /// Sensor index (`0` for primary mouse sensor).
    pub sensor_index: u8,
    /// Discrete supported DPI values (if reported outside a range).
    pub discrete: Vec<Dpi>,
    /// Stepped continuous DPI ranges (`[min, 0xE000 | step, max]`).
    pub ranges: Vec<DpiRange>,
}

impl DpiCapabilities {
    /// Returns `true` if `dpi` is permitted by either discrete choices or a stepped range.
    #[must_use]
    pub fn supports(&self, dpi: Dpi) -> bool {
        self.discrete.contains(&dpi) || self.ranges.iter().any(|r| r.contains(dpi))
    }

    /// Returns the minimum supported DPI across all discrete values and ranges.
    #[must_use]
    pub fn min_dpi(&self) -> Option<Dpi> {
        self.discrete
            .iter()
            .copied()
            .chain(self.ranges.iter().map(|r| r.min))
            .min()
    }

    /// Returns the maximum supported DPI across all discrete values and ranges.
    #[must_use]
    pub fn max_dpi(&self) -> Option<Dpi> {
        self.discrete
            .iter()
            .copied()
            .chain(self.ranges.iter().map(|r| r.max))
            .max()
    }
}

/// Active and default DPI reported by `ADJUSTABLE_DPI::getSensorDpi` (`fn2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorDpiState {
    /// Sensor index queried.
    pub sensor_index: u8,
    /// Currently active sensor DPI.
    pub current: Dpi,
    /// Firmware default DPI (`None` on `0x2201` v0 or when reported as `0`).
    pub default_dpi: Option<Dpi>,
}

/// Result of a verified DPI write + readback check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiWriteVerification {
    /// Sensor index written.
    pub sensor_index: u8,
    /// Target DPI requested.
    pub requested: Dpi,
    /// State confirmed by immediate post-write readback.
    pub readback: SensorDpiState,
}

/// Trait implemented by versioned `ADJUSTABLE_DPI` (`0x2201`) drivers.
pub trait AdjustableDpiFeature {
    /// Queries the number of optical/motion sensors (`fn0`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_sensor_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError>;

    /// Queries supported discrete DPI values and stepped ranges for `sensor_index` (`fn1`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_sensor_dpi_list<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
    ) -> Result<DpiCapabilities, HidppError>;

    /// Reads the current and default DPI for `sensor_index` (`fn2`).
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport or protocol errors.
    fn get_sensor_dpi<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
    ) -> Result<SensorDpiState, HidppError>;

    /// Writes `dpi` to `sensor_index` (`fn3`) after checking feature version, sensor bounds,
    /// and supported DPI capabilities, followed by mandatory readback verification.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if the feature version is unsupported for writes, `sensor_index`
    /// or `dpi` fails capability validation, the write fails, or readback mismatches `dpi`.
    fn set_sensor_dpi_verified<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
        dpi: Dpi,
    ) -> Result<DpiWriteVerification, HidppError>;
}

/// Versioned runtime driver for `ADJUSTABLE_DPI` (`0x2201`).
#[derive(Debug, Clone, Copy)]
pub struct AdjustableDpiDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for AdjustableDpiDriver {
    const FEATURE_ID: FeatureId = FeatureId::ADJUSTABLE_DPI;
    const MAX_SUPPORTED_VERSION: u8 = 2;
}

impl AdjustableDpiDriver {
    /// Creates a versioned driver from a runtime-discovered [`FeatureEntry`].
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `entry.id` is not [`FeatureId::ADJUSTABLE_DPI`].
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected ADJUSTABLE_DPI (0x2201), got {}", entry.id),
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
        self.version <= Self::MAX_SUPPORTED_VERSION
    }

    /// Encodes request parameters for `getSensorCount` (`fn0`).
    #[must_use]
    pub const fn encode_get_sensor_count_params() -> [u8; 0] {
        []
    }

    /// Encodes request parameters for `getSensorDpiList(sensor_index)` (`fn1`).
    #[must_use]
    pub const fn encode_get_sensor_dpi_list_params(sensor_index: u8) -> [u8; 1] {
        [sensor_index]
    }

    /// Encodes request parameters for `getSensorDpi(sensor_index)` (`fn2`).
    #[must_use]
    pub const fn encode_get_sensor_dpi_params(sensor_index: u8) -> [u8; 1] {
        [sensor_index]
    }

    /// Encodes request parameters for `setSensorDpi(sensor_index, dpi)` (`fn3`).
    #[must_use]
    pub const fn encode_set_sensor_dpi_params(sensor_index: u8, dpi: Dpi) -> [u8; 3] {
        let [msb, lsb] = dpi.as_u16().to_be_bytes();
        [sensor_index, msb, lsb]
    }

    /// Pure parser for `getSensorCount` (`fn0`) response parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty.
    pub fn parse_sensor_count_response(params: &[u8]) -> Result<u8, ProtocolError> {
        let Some(&count) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        Ok(count)
    }

    /// Pure parser for `getSensorDpiList` (`fn1`) response parameters.
    ///
    /// Handles both discrete DPI lists and stepped ranges (`[min, 0xE000 | step, max]`).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if `params` is empty, or
    /// [`ProtocolError::UnexpectedResponse`] on sensor index mismatch or malformed range triplets.
    pub fn parse_sensor_dpi_list_response(
        expected_sensor_index: u8,
        params: &[u8],
    ) -> Result<DpiCapabilities, ProtocolError> {
        let Some(&sensor_index) = params.first() else {
            return Err(ProtocolError::ResponseTruncated {
                expected: 1,
                actual: 0,
            });
        };
        if sensor_index != expected_sensor_index {
            return Err(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "getSensorDpiList sensor mismatch: expected {expected_sensor_index}, got {sensor_index}"
                ),
            });
        }

        let (pairs, _) = params[1..].as_chunks::<2>();
        let mut words = pairs
            .iter()
            .map(|&pair| u16::from_be_bytes(pair))
            .peekable();

        let mut discrete = Vec::new();
        let mut ranges = Vec::new();

        while let Some(word) = words.next() {
            if word == 0 {
                break;
            }
            if (word >> 13) == 0b111 {
                let step = word & 0x1FFF;
                if step == 0 {
                    return Err(ProtocolError::UnexpectedResponse {
                        detail: "getSensorDpiList returned reserved 0xE000 step entry".to_owned(),
                    });
                }
                let min = discrete
                    .pop()
                    .ok_or_else(|| ProtocolError::UnexpectedResponse {
                        detail: format!(
                            "getSensorDpiList step marker 0x{word:04X} without preceding min DPI"
                        ),
                    })?;
                let max_raw = words
                    .next()
                    .ok_or_else(|| ProtocolError::UnexpectedResponse {
                        detail: format!(
                            "getSensorDpiList step marker 0x{word:04X} missing max DPI"
                        ),
                    })?;
                if max_raw == 0 || max_raw > MAX_PROTOCOL_DPI {
                    return Err(ProtocolError::UnexpectedResponse {
                        detail: format!("getSensorDpiList invalid range max DPI 0x{max_raw:04X}"),
                    });
                }
                let max = Dpi::new(max_raw)?;
                if max < min {
                    return Err(ProtocolError::UnexpectedResponse {
                        detail: format!(
                            "getSensorDpiList range max {} < min {}",
                            max.as_u16(),
                            min.as_u16()
                        ),
                    });
                }
                ranges.push(DpiRange { min, max, step });
            } else {
                discrete.push(Dpi::new(word)?);
            }
        }

        Ok(DpiCapabilities {
            sensor_index,
            discrete,
            ranges,
        })
    }

    /// Pure parser for `getSensorDpi` (`fn2`) response parameters, aware of `feature_version`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] if fewer than 3 parameter bytes exist, or
    /// [`ProtocolError::UnexpectedResponse`] if sensor index mismatches or DPI is `0`/out of range.
    pub fn parse_sensor_dpi_response(
        feature_version: u8,
        expected_sensor_index: u8,
        params: &[u8],
    ) -> Result<SensorDpiState, ProtocolError> {
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let sensor_index = params[0];
        if sensor_index != expected_sensor_index {
            return Err(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "getSensorDpi sensor mismatch: expected {expected_sensor_index}, got {sensor_index}"
                ),
            });
        }

        let current_raw = u16::from_be_bytes([params[1], params[2]]);
        let default_raw = if feature_version == 0 {
            0
        } else {
            match (params.get(3), params.get(4)) {
                (Some(&msb), Some(&lsb)) => u16::from_be_bytes([msb, lsb]),
                _ => 0,
            }
        };

        let default_dpi = if default_raw == 0 {
            None
        } else {
            Some(
                Dpi::new(default_raw).map_err(|err| ProtocolError::UnexpectedResponse {
                    detail: err.to_string(),
                })?,
            )
        };

        let current = if current_raw == 0 {
            default_dpi.ok_or_else(|| ProtocolError::UnexpectedResponse {
                detail: "getSensorDpi reported 0 current and 0 default DPI".to_owned(),
            })?
        } else {
            Dpi::new(current_raw).map_err(|err| ProtocolError::UnexpectedResponse {
                detail: err.to_string(),
            })?
        };

        Ok(SensorDpiState {
            sensor_index,
            current,
            default_dpi,
        })
    }

    /// Pure parser for `setSensorDpi` (`fn3`) response parameters, verifying echo on `v1+`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseTruncated`] or [`ProtocolError::UnexpectedResponse`] on
    /// `v1+` when the echoed sensor index or DPI differs from the request.
    pub fn parse_set_sensor_dpi_response(
        feature_version: u8,
        expected_sensor_index: u8,
        expected_dpi: Dpi,
        params: &[u8],
    ) -> Result<(), ProtocolError> {
        if feature_version == 0 {
            // Version 0 does not echo parameters in setSensorDpi replies.
            return Ok(());
        }
        if params.len() < 3 {
            return Err(ProtocolError::ResponseTruncated {
                expected: 3,
                actual: params.len(),
            });
        }
        let sensor_index = params[0];
        let echoed_dpi = u16::from_be_bytes([params[1], params[2]]);
        if sensor_index != expected_sensor_index || echoed_dpi != expected_dpi.as_u16() {
            return Err(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "setSensorDpi echo mismatch: expected sensor {expected_sensor_index} @ {}, got sensor {sensor_index} @ {echoed_dpi}",
                    expected_dpi.as_u16()
                ),
            });
        }
        Ok(())
    }

    /// Sends raw `setSensorDpi` (`fn3`) without capability pre-checks or readback.
    ///
    /// Prefer [`Self::set_sensor_dpi_verified`] for hardware safety.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if version > `MAX_SUPPORTED_VERSION` or on transport/protocol error.
    pub fn set_sensor_dpi_raw<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
        dpi: Dpi,
    ) -> Result<(), HidppError> {
        if !self.supports_writes() {
            return Err(ProtocolError::UnsupportedFeatureVersion {
                feature_id: Self::FEATURE_ID,
                version: self.version,
                max_supported: Self::MAX_SUPPORTED_VERSION,
            }
            .into());
        }
        let params = Self::encode_set_sensor_dpi_params(sensor_index, dpi);
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_SET_SENSOR_DPI)?,
            &params,
        )?;
        Ok(Self::parse_set_sensor_dpi_response(
            self.version,
            sensor_index,
            dpi,
            reply.params(),
        )?)
    }
}

impl AdjustableDpiFeature for AdjustableDpiDriver {
    fn get_sensor_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_SENSOR_COUNT)?,
            &Self::encode_get_sensor_count_params(),
        )?;
        Ok(Self::parse_sensor_count_response(reply.params())?)
    }

    fn get_sensor_dpi_list<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
    ) -> Result<DpiCapabilities, HidppError> {
        let params = Self::encode_get_sensor_dpi_list_params(sensor_index);
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_SENSOR_DPI_LIST)?,
            &params,
        )?;
        Ok(Self::parse_sensor_dpi_list_response(
            sensor_index,
            reply.params(),
        )?)
    }

    fn get_sensor_dpi<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
    ) -> Result<SensorDpiState, HidppError> {
        let params = Self::encode_get_sensor_dpi_params(sensor_index);
        let reply = session.request(
            device_index,
            self.feature_index,
            FunctionId::new(FN_GET_SENSOR_DPI)?,
            &params,
        )?;
        Ok(Self::parse_sensor_dpi_response(
            self.version,
            sensor_index,
            reply.params(),
        )?)
    }

    fn set_sensor_dpi_verified<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sensor_index: u8,
        dpi: Dpi,
    ) -> Result<DpiWriteVerification, HidppError> {
        if !self.supports_writes() {
            return Err(ProtocolError::UnsupportedFeatureVersion {
                feature_id: Self::FEATURE_ID,
                version: self.version,
                max_supported: Self::MAX_SUPPORTED_VERSION,
            }
            .into());
        }

        let sensor_count = self.get_sensor_count(session, device_index)?;
        if sensor_index >= sensor_count {
            return Err(ProtocolError::InvalidArgument {
                detail: format!(
                    "sensor index {sensor_index} out of bounds (device reports {sensor_count} sensor(s))"
                ),
            }
            .into());
        }

        let caps = self.get_sensor_dpi_list(session, device_index, sensor_index)?;
        if !caps.supports(dpi) {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("requested {dpi} is not supported by sensor {sensor_index}"),
            }
            .into());
        }

        self.set_sensor_dpi_raw(session, device_index, sensor_index, dpi)?;
        let readback = self.get_sensor_dpi(session, device_index, sensor_index)?;
        if readback.current != dpi {
            return Err(ProtocolError::ReadbackMismatch {
                feature_id: Self::FEATURE_ID,
                expected: dpi.to_string(),
                actual: readback.current.to_string(),
            }
            .into());
        }

        Ok(DpiWriteVerification {
            sensor_index,
            requested: dpi,
            readback,
        })
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

    fn test_session(mock: MockTransport) -> HidppSession<MockTransport> {
        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        HidppSession::with_config(mock, cfg)
    }

    #[test]
    fn golden_adjustable_dpi_range_read_write_and_readback() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x0A);

        // 1. fn0 getSensorCount -> 1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[1, 0, 0],
            )
            .unwrap(),
        );
        // 2. fn1 getSensorDpiList(0) -> [0x00, 100 (0x0064), step 50 (0xE032), 25600 (0x6400), 0x0000]
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        // 3. fn3 setSensorDpi(0, 1600) -> echo [0x00, 0x06, 0x40]
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(3).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x06, 0x40],
            )
            .unwrap(),
        );
        // 4. fn2 readback getSensorDpi(0) -> current 1600 (0x0640), default 800 (0x0320)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x06, 0x40, 0x03, 0x20],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let driver = AdjustableDpiDriver::new(feat_idx, 1);

        let target = Dpi::new(1600).unwrap();
        let verified = driver
            .set_sensor_dpi_verified(&mut session, DeviceIndex::DIRECT, 0, target)
            .unwrap();
        assert_eq!(verified.requested, target);
        assert_eq!(verified.readback.current, target);
        assert_eq!(verified.readback.default_dpi, Some(Dpi::new(800).unwrap()));
    }

    #[test]
    fn rejects_unsupported_dpi_version_mismatch_and_malformed_lists() {
        // Discrete list parsing
        let discrete = AdjustableDpiDriver::parse_sensor_dpi_list_response(
            0,
            &[0x00, 0x01, 0x90, 0x03, 0x20, 0x06, 0x40, 0x00, 0x00],
        )
        .unwrap();
        assert_eq!(
            discrete.discrete,
            vec![
                Dpi::new(400).unwrap(),
                Dpi::new(800).unwrap(),
                Dpi::new(1600).unwrap()
            ]
        );
        assert!(discrete.supports(Dpi::new(800).unwrap()));
        assert!(!discrete.supports(Dpi::new(1000).unwrap()));

        // Malformed range entries
        assert!(matches!(
            AdjustableDpiDriver::parse_sensor_dpi_list_response(0, &[0x00, 0xE0, 0x00, 0x00, 0x00]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
        assert!(matches!(
            AdjustableDpiDriver::parse_sensor_dpi_list_response(0, &[0x00, 0xE0, 0x32, 0x06, 0x40]),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));
        assert!(matches!(
            AdjustableDpiDriver::parse_sensor_dpi_list_response(
                0,
                &[0x00, 0x06, 0x40, 0xE0, 0x32, 0x01, 0x90]
            ),
            Err(ProtocolError::UnexpectedResponse { .. })
        ));

        // Unknown future version degrades to read-only (refuses writes before any transport I/O)
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Future Mouse");
        let mock = MockTransport::new(info);
        let mut session = test_session(mock);
        let future_driver = AdjustableDpiDriver::new(FeatureIndex(0x0A), 9);
        assert!(!future_driver.supports_writes());
        let err = future_driver
            .set_sensor_dpi_verified(&mut session, DeviceIndex::DIRECT, 0, Dpi::new(800).unwrap())
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::UnsupportedFeatureVersion {
                feature_id: FeatureId::ADJUSTABLE_DPI,
                version: 9,
                ..
            })
        ));
        assert!(session.transport().written_frames().is_empty());
    }

    #[test]
    fn detects_readback_mismatch() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);
        let feat_idx = FeatureIndex(0x0A);

        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[1, 0, 0],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(3).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x03, 0x20],
            )
            .unwrap(),
        );
        // Readback unexpectedly reports 400 DPI instead of 800 DPI
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x90, 0x03, 0x20],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);
        let driver = AdjustableDpiDriver::new(feat_idx, 1);
        let err = driver
            .set_sensor_dpi_verified(&mut session, DeviceIndex::DIRECT, 0, Dpi::new(800).unwrap())
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::ReadbackMismatch {
                feature_id: FeatureId::ADJUSTABLE_DPI,
                ..
            })
        ));
    }

    proptest! {
        #[test]
        fn dpi_params_and_state_roundtrip(
            sensor in any::<u8>(),
            dpi_raw in 1u16..=MAX_PROTOCOL_DPI,
            def_raw in 1u16..=MAX_PROTOCOL_DPI,
        ) {
            let dpi = Dpi::new(dpi_raw).unwrap();
            let req = AdjustableDpiDriver::encode_set_sensor_dpi_params(sensor, dpi);
            let parsed_set = AdjustableDpiDriver::parse_set_sensor_dpi_response(1, sensor, dpi, &req);
            prop_assert!(parsed_set.is_ok());

            let [d_hi, d_lo] = dpi_raw.to_be_bytes();
            let [f_hi, f_lo] = def_raw.to_be_bytes();
            let state = AdjustableDpiDriver::parse_sensor_dpi_response(
                1,
                sensor,
                &[sensor, d_hi, d_lo, f_hi, f_lo],
            ).unwrap();
            prop_assert_eq!(state.sensor_index, sensor);
            prop_assert_eq!(state.current.as_u16(), dpi_raw);
            prop_assert_eq!(state.default_dpi.map(Dpi::as_u16), Some(def_raw));
        }
    }
}

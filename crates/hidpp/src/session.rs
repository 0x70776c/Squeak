//! HID++ 2.0 request/response session multiplexer with notification filtering and `Busy` retries.

use std::time::{Duration, Instant};

use tracing::{debug, trace, warn};

use crate::error::{
    Hidpp10Error, Hidpp20Error, Hidpp20ErrorCode, HidppError, ProtocolError, TransportError,
};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId, HidppFrame, SwId, pack_func_sw};
use crate::transport::{HidDeviceInfo, HidTransport};

const MAX_BUSY_BACKOFF: Duration = Duration::from_millis(500);

/// Tuning parameters for [`HidppSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Non-zero software ID placed on outgoing requests (`1..=15`) to correlate replies.
    pub sw_id: SwId,
    /// Total deadline to wait for a single matching response (excluding retries).
    pub response_timeout: Duration,
    /// Maximum retries after receiving [`Hidpp20ErrorCode::Busy`] (`0x08`).
    pub max_busy_retries: u32,
    /// Base delay for exponential backoff after [`Hidpp20ErrorCode::Busy`].
    pub busy_initial_backoff: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            sw_id: SwId::DEFAULT_REQUEST,
            response_timeout: Duration::from_millis(1000),
            max_busy_retries: 5,
            busy_initial_backoff: Duration::from_millis(25),
        }
    }
}

/// Stateful request/response driver over any [`HidTransport`].
#[derive(Debug)]
pub struct HidppSession<T: HidTransport> {
    transport: T,
    config: SessionConfig,
}

impl<T: HidTransport> HidppSession<T> {
    /// Wraps `transport` with [`SessionConfig::default`].
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            config: SessionConfig::default(),
        }
    }

    /// Wraps `transport` with explicit [`SessionConfig`].
    #[must_use]
    pub fn with_config(transport: T, config: SessionConfig) -> Self {
        Self { transport, config }
    }

    /// Returns endpoint metadata from the underlying transport.
    #[must_use]
    pub fn device_info(&self) -> &HidDeviceInfo {
        self.transport.device_info()
    }

    /// Returns the active [`SessionConfig`].
    #[must_use]
    pub const fn config(&self) -> SessionConfig {
        self.config
    }

    /// Returns a shared reference to the underlying transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Returns a mutable reference to the underlying transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Consumes the session and returns the inner transport.
    #[must_use]
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// Sends one HID++ 2.0 request and waits for the correlated response frame.
    ///
    /// - Automatically chooses the smallest supported report ID (`0x10`, `0x11`, or `0x12`)
    ///   for `params` according to [`HidDeviceInfo::supported_reports`].
    /// - Ignores device notifications (`sw_id == 0`) and frames targeting other devices or `sw_id`s.
    /// - Translates HID++ 2.0 `0xFF` and HID++ 1.0 `0x8F` error frames into typed errors.
    /// - Retries up to [`SessionConfig::max_busy_retries`] times with bounded exponential backoff
    ///   when the device responds with [`Hidpp20ErrorCode::Busy`].
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] on transport failures, timeouts, malformed frames, legacy
    /// HID++ 1.0 responses, exhausted `Busy` retries, or device protocol errors.
    pub fn request(
        &mut self,
        device_index: DeviceIndex,
        feature_index: FeatureIndex,
        function_id: FunctionId,
        params: &[u8],
    ) -> Result<HidppFrame, HidppError> {
        let supported = self.transport.device_info().supported_reports;
        let report_id = supported.select_for_payload(params.len())?;
        let req = HidppFrame::new(
            report_id,
            device_index,
            feature_index,
            function_id,
            self.config.sw_id,
            params,
        )?;
        let expected_func_sw = pack_func_sw(function_id, self.config.sw_id);

        let total_attempts = self.config.max_busy_retries.saturating_add(1);
        'attempt: for attempt in 0..total_attempts {
            self.transport.write_frame(&req)?;

            let deadline = Instant::now() + self.config.response_timeout;
            let mut first_read = true;

            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if !first_read && remaining.is_zero() {
                    return Err(TransportError::Timeout.into());
                }
                first_read = false;

                let frame = self.transport.read_frame(remaining)?;

                if frame.device_index() != device_index {
                    trace!(
                        expected = %device_index,
                        actual = %frame.device_index(),
                        "ignoring frame for different device index"
                    );
                    continue;
                }

                if let Some(err20) = Hidpp20Error::from_frame(&frame) {
                    if err20.feature_index == feature_index
                        && err20.function_id == function_id
                        && err20.sw_id == self.config.sw_id
                    {
                        if err20.code == Hidpp20ErrorCode::Busy {
                            if attempt + 1 < total_attempts {
                                let shift = attempt.min(6);
                                let backoff = self
                                    .config
                                    .busy_initial_backoff
                                    .saturating_mul(1u32 << shift)
                                    .min(MAX_BUSY_BACKOFF);
                                debug!(
                                    attempt = attempt + 1,
                                    max = total_attempts,
                                    ?backoff,
                                    feature = %feature_index,
                                    func = %function_id,
                                    "device busy; backing off before retry"
                                );
                                if !backoff.is_zero() {
                                    std::thread::sleep(backoff);
                                }
                                continue 'attempt;
                            }
                            return Err(ProtocolError::BusyRetriesExhausted {
                                feature_index,
                                function_id,
                                attempts: total_attempts,
                            }
                            .into());
                        }
                        return Err(HidppError::from(err20));
                    }
                    trace!(?err20, "ignoring HID++ 2.0 error for unrelated request");
                    continue;
                }

                if let Some(err10) = Hidpp10Error::from_frame(&frame) {
                    if err10.sub_id == feature_index.0 && err10.address == expected_func_sw {
                        warn!(?err10, "device answered with legacy HID++ 1.0 ERROR_MSG");
                        return Err(ProtocolError::LegacyDeviceNotSupported {
                            error_code: err10.code,
                        }
                        .into());
                    }
                    trace!(?err10, "ignoring unrelated HID++ 1.0 error frame");
                    continue;
                }

                if frame.sw_id().is_notification() {
                    trace!(
                        feature = %frame.feature_index(),
                        func = %frame.function_id(),
                        "skipping unsolicited device notification while awaiting reply"
                    );
                    continue;
                }

                if frame.sw_id() == self.config.sw_id
                    && frame.feature_index() == feature_index
                    && frame.function_id() == function_id
                {
                    return Ok(frame);
                }

                debug!(
                    expected_feature = %feature_index,
                    expected_func = %function_id,
                    expected_sw = %self.config.sw_id,
                    actual_feature = %frame.feature_index(),
                    actual_func = %frame.function_id(),
                    actual_sw = %frame.sw_id(),
                    "ignoring uncorrelated HID++ frame"
                );
            }
        }

        Err(TransportError::Timeout.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Hidpp10ErrorCode;
    use crate::frame::{ReportId, SupportedReports};
    use crate::transport::MockTransport;

    fn test_config() -> SessionConfig {
        SessionConfig {
            sw_id: SwId::DEFAULT_REQUEST,
            response_timeout: Duration::from_millis(50),
            max_busy_retries: 2,
            busy_initial_backoff: Duration::ZERO,
        }
    }

    fn fn_id(id: u8) -> FunctionId {
        FunctionId::new(id).unwrap()
    }

    #[test]
    fn skips_notifications_and_other_sw_ids_before_matching_reply() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Test Mouse");
        let mut mock = MockTransport::new(info);

        // Unsolicited event (sw_id == 0)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(0x05),
                fn_id(0),
                SwId::NOTIFICATION,
                &[0x01, 0x02, 0x03],
            )
            .unwrap(),
        );
        // Frame belonging to another software client (sw_id == 0x05)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::request(5).unwrap(),
                &[0x04, 0x05, 0x42],
            )
            .unwrap(),
        );
        // Frame targeting a different receiver slot (slot 1 != direct)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::receiver(1).unwrap(),
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x02, 0x42],
            )
            .unwrap(),
        );
        // Matching reply
        let expected = HidppFrame::new(
            ReportId::Short,
            DeviceIndex::DIRECT,
            FeatureIndex::IROOT,
            fn_id(1),
            SwId::DEFAULT_REQUEST,
            &[0x04, 0x02, 0x42],
        )
        .unwrap();
        mock.push_frame(expected);

        let mut session = HidppSession::with_config(mock, test_config());
        let reply = session
            .request(
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                &[0, 0, 0x42],
            )
            .unwrap();
        assert_eq!(reply, expected);
        assert_eq!(session.transport().written_frames().len(), 1);
    }

    #[test]
    fn retries_on_busy_and_succeeds() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Test Mouse");
        let mut mock = MockTransport::new(info);

        // Busy error frame: [0x10, 0xFF, 0xFF, 0x03, 0x0A, 0x08, 0x00]
        let busy1 = HidppFrame::decode(&[0x10, 0xFF, 0xFF, 0x03, 0x0A, 0x08, 0x00]).unwrap();
        let busy2 = HidppFrame::decode(&[0x10, 0xFF, 0xFF, 0x03, 0x0A, 0x08, 0x00]).unwrap();
        let success = HidppFrame::new(
            ReportId::Short,
            DeviceIndex::DIRECT,
            FeatureIndex(0x03),
            fn_id(0),
            SwId::DEFAULT_REQUEST,
            &[0xAA, 0xBB, 0xCC],
        )
        .unwrap();

        mock.push_frame(busy1);
        mock.push_frame(busy2);
        mock.push_frame(success);

        let mut session = HidppSession::with_config(mock, test_config());
        let reply = session
            .request(DeviceIndex::DIRECT, FeatureIndex(0x03), fn_id(0), &[])
            .unwrap();
        assert_eq!(reply, success);
        assert_eq!(session.transport().written_frames().len(), 3);
    }

    #[test]
    fn returns_busy_retries_exhausted_when_budget_exceeded() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Test Mouse");
        let mut mock = MockTransport::new(info);

        let busy = HidppFrame::decode(&[0x10, 0xFF, 0xFF, 0x03, 0x0A, 0x08, 0x00]).unwrap();
        mock.push_frame(busy);
        mock.push_frame(busy);
        mock.push_frame(busy);

        let mut session = HidppSession::with_config(mock, test_config());
        let err = session
            .request(DeviceIndex::DIRECT, FeatureIndex(0x03), fn_id(0), &[])
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::BusyRetriesExhausted { attempts: 3, .. })
        ));
    }

    #[test]
    fn maps_hidpp20_non_busy_error_immediately() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Test Mouse");
        let mut mock = MockTransport::new(info);
        let invalid_arg = HidppFrame::decode(&[0x10, 0xFF, 0xFF, 0x04, 0x1A, 0x02, 0x00]).unwrap();
        mock.push_frame(invalid_arg);

        let mut session = HidppSession::with_config(mock, test_config());
        let err = session
            .request(DeviceIndex::DIRECT, FeatureIndex(0x04), fn_id(1), &[])
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::DeviceError(Hidpp20Error {
                code: Hidpp20ErrorCode::InvalidArgument,
                ..
            }))
        ));
        assert_eq!(session.transport().written_frames().len(), 1);
    }

    #[test]
    fn maps_hidpp10_legacy_error_immediately() {
        let info = HidDeviceInfo::mock(0x046D, 0xC000, "Legacy Mouse");
        let mut mock = MockTransport::new(info);
        let legacy = HidppFrame::decode(&[0x10, 0xFF, 0x8F, 0x00, 0x1A, 0x01, 0x00]).unwrap();
        mock.push_frame(legacy);

        let mut session = HidppSession::with_config(mock, test_config());
        let err = session
            .request(
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                &[0, 0, 0x42],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::LegacyDeviceNotSupported {
                error_code: Hidpp10ErrorCode::InvalidSubId
            })
        ));
    }

    #[test]
    fn times_out_even_while_receiving_continuous_notifications() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Chatty Mouse");
        let mut mock = MockTransport::new(info);
        let notif = HidppFrame::new(
            ReportId::Short,
            DeviceIndex::DIRECT,
            FeatureIndex(0x02),
            fn_id(0),
            SwId::NOTIFICATION,
            &[0x01, 0x00, 0x00],
        )
        .unwrap();
        for _ in 0..50 {
            mock.push_frame(notif);
        }

        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(5),
            ..test_config()
        };
        let mut session = HidppSession::with_config(mock, cfg);
        let err = session
            .request(
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                &[0, 0, 1],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            HidppError::Transport(TransportError::Timeout)
        ));

        let info2 = HidDeviceInfo::mock(0x046D, 0xC08B, "Chatty Mouse 2");
        let mut mock2 = MockTransport::new(info2);
        mock2.push_frame(notif);
        mock2.push_frame(notif);
        let cfg2 = SessionConfig {
            response_timeout: Duration::ZERO,
            ..test_config()
        };
        let mut session2 = HidppSession::with_config(mock2, cfg2);
        let err2 = session2
            .request(
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                &[0, 0, 1],
            )
            .unwrap_err();
        assert!(matches!(
            err2,
            HidppError::Transport(TransportError::Timeout)
        ));
        assert_eq!(session2.transport().remaining_reads(), 1);
    }

    #[test]
    fn uses_long_report_automatically_when_short_unsupported() {
        let mut info = HidDeviceInfo::mock(0x046D, 0xB034, "BLE Mouse");
        info.supported_reports = SupportedReports::LONG_ONLY;
        let mut mock = MockTransport::new(info);

        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x05, 0x99],
            )
            .unwrap(),
        );

        let mut session = HidppSession::with_config(mock, test_config());
        let reply = session
            .request(
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                &[0, 0, 0x99],
            )
            .unwrap();
        assert_eq!(reply.report_id(), ReportId::Long);
        assert_eq!(
            session.transport().written_frames()[0].report_id(),
            ReportId::Long
        );
    }
}

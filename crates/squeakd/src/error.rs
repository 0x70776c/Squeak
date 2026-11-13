//! Typed error definitions and D-Bus error conversions for `squeakd`.

use hidpp::{HidppError, TransportError};
use squeak_core::DeviceDataError;
use thiserror::Error;

/// Daemon errors returned by device registry and D-Bus handlers.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Invalid selector or request argument.
    #[error("{0}")]
    InvalidArgument(String),

    /// Requested device selector did not match any known endpoint.
    #[error("no device matching {0:?}")]
    DeviceNotFound(String),

    /// Requested selector matched multiple endpoints and needs disambiguation.
    #[error("ambiguous device selector {0:?}; pass exact device_key or hidraw path")]
    AmbiguousDevice(String),

    /// Built-in device database failed validation.
    #[error("device database error: {0}")]
    DeviceDatabase(#[from] DeviceDataError),

    /// Underlying transport/hidraw error.
    #[error("{0}")]
    Transport(#[from] TransportError),

    /// HID++ 2.0 protocol error.
    #[error("{0}")]
    Hidpp(#[from] HidppError),

    /// JSON serialization failure.
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    /// D-Bus connection or interface registration error.
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
}

impl From<DaemonError> for zbus::fdo::Error {
    fn from(err: DaemonError) -> Self {
        match err {
            DaemonError::InvalidArgument(msg)
            | DaemonError::DeviceNotFound(msg)
            | DaemonError::AmbiguousDevice(msg) => Self::InvalidArgs(msg),
            other => Self::Failed(other.to_string()),
        }
    }
}

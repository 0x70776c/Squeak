//! Abstract HID transport trait and deterministic in-memory mock implementation.
//!
//! No serial numbers or persistent unique hardware identifiers are ever stored on
//! [`HidDeviceInfo`].

use core::fmt;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use crate::descriptor::SysfsUeventInfo;
use crate::error::TransportError;
use crate::frame::{HidppFrame, SupportedReports};

/// Physical HID bus type reported by Linux sysfs.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusType {
    /// USB (`0x0003`).
    Usb,
    /// Bluetooth / BLE (`0x0005`).
    Bluetooth,
    /// Other HID bus constant reported by the kernel.
    Other(u16),
}

impl BusType {
    /// Converts a Linux `HID_ID` bus constant into [`BusType`].
    #[must_use]
    pub const fn from_raw(bus: u16) -> Self {
        match bus {
            0x0003 => Self::Usb,
            0x0005 => Self::Bluetooth,
            other => Self::Other(other),
        }
    }

    /// Returns the raw kernel bus type identifier.
    #[must_use]
    pub const fn as_raw(self) -> u16 {
        match self {
            Self::Usb => 0x0003,
            Self::Bluetooth => 0x0005,
            Self::Other(val) => val,
        }
    }
}

impl fmt::Display for BusType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb => write!(f, "USB"),
            Self::Bluetooth => write!(f, "Bluetooth"),
            Self::Other(raw) => write!(f, "Bus(0x{raw:04X})"),
        }
    }
}

/// Non-identifying metadata describing a discovered HID++ transport endpoint.
///
/// Serial numbers (`HID_UNIQ`) are intentionally excluded from this structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidDeviceInfo {
    /// Device node path (for example `/dev/hidraw1` or `/dev/mock0`).
    pub path: PathBuf,
    /// Physical HID bus.
    pub bus: BusType,
    /// USB/BT vendor ID (e.g. `0x046D`).
    pub vendor_id: u16,
    /// USB/BT product ID (e.g. `0xC08B`).
    pub product_id: u16,
    /// Human-readable device name from `HID_NAME`, if available.
    pub name: Option<String>,
    /// Exact set of HID++ report IDs advertised by the HID report descriptor.
    pub supported_reports: SupportedReports,
}

impl HidDeviceInfo {
    /// Builds [`HidDeviceInfo`] from parsed sysfs `uevent` metadata and descriptor capabilities.
    #[must_use]
    pub fn from_sysfs(
        path: PathBuf,
        uevent: SysfsUeventInfo,
        supported_reports: SupportedReports,
    ) -> Self {
        Self {
            path,
            bus: BusType::from_raw(uevent.bus_type),
            vendor_id: uevent.vendor_id,
            product_id: uevent.product_id,
            name: uevent.name,
            supported_reports,
        }
    }

    /// Convenience constructor for synthetic test devices (`BusType::Usb`).
    #[must_use]
    pub fn mock(vendor_id: u16, product_id: u16, name: impl Into<String>) -> Self {
        Self {
            path: PathBuf::from("/dev/mock-hidraw0"),
            bus: BusType::Usb,
            vendor_id,
            product_id,
            name: Some(name.into()),
            supported_reports: SupportedReports::SHORT_AND_LONG,
        }
    }
}

/// Synchronous HID frame transport implemented by Linux `hidraw` and test mocks.
///
/// Higher-level async services (`squeakd`) wrap this trait on dedicated blocking worker
/// threads so blocking I/O never stalls async executors.
pub trait HidTransport: Send {
    /// Returns metadata for the open HID device endpoint.
    fn device_info(&self) -> &HidDeviceInfo;

    /// Writes a single HID++ report frame to the device.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if permissions are missing, the device disconnects,
    /// or the underlying OS write fails.
    fn write_frame(&mut self, frame: &HidppFrame) -> Result<(), TransportError>;

    /// Waits up to `timeout` and reads a single HID++ report frame from the device.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Timeout`] when no HID++ report arrives within `timeout`,
    /// or [`TransportError::Disconnected`] / [`TransportError::Io`] on transport failure.
    fn read_frame(&mut self, timeout: Duration) -> Result<HidppFrame, TransportError>;
}

/// Deterministic mock [`HidTransport`] for unit and integration testing without hardware.
#[derive(Debug)]
pub struct MockTransport {
    info: HidDeviceInfo,
    read_queue: VecDeque<Result<HidppFrame, TransportError>>,
    write_errors: VecDeque<TransportError>,
    written: Vec<HidppFrame>,
    default_timeout_on_empty: bool,
}

impl MockTransport {
    /// Creates an empty [`MockTransport`] for the given synthetic [`HidDeviceInfo`].
    #[must_use]
    pub fn new(info: HidDeviceInfo) -> Self {
        Self {
            info,
            read_queue: VecDeque::new(),
            write_errors: VecDeque::new(),
            written: Vec::new(),
            default_timeout_on_empty: true,
        }
    }

    /// Appends a frame to be returned on the next `read_frame` call.
    pub fn push_frame(&mut self, frame: HidppFrame) {
        self.read_queue.push_back(Ok(frame));
    }

    /// Appends a transport error to be returned on the next `read_frame` call.
    pub fn push_read_error(&mut self, err: TransportError) {
        self.read_queue.push_back(Err(err));
    }

    /// Schedules an error for the next `write_frame` call.
    pub fn push_write_error(&mut self, err: TransportError) {
        self.write_errors.push_back(err);
    }

    /// Returns all frames successfully passed to [`HidTransport::write_frame`].
    #[must_use]
    pub fn written_frames(&self) -> &[HidppFrame] {
        &self.written
    }

    /// Drains and returns all recorded written frames.
    pub fn take_written_frames(&mut self) -> Vec<HidppFrame> {
        std::mem::take(&mut self.written)
    }

    /// Returns the number of queued read events remaining.
    #[must_use]
    pub fn remaining_reads(&self) -> usize {
        self.read_queue.len()
    }
}

impl HidTransport for MockTransport {
    fn device_info(&self) -> &HidDeviceInfo {
        &self.info
    }

    fn write_frame(&mut self, frame: &HidppFrame) -> Result<(), TransportError> {
        if let Some(err) = self.write_errors.pop_front() {
            return Err(err);
        }
        self.written.push(*frame);
        Ok(())
    }

    fn read_frame(&mut self, _timeout: Duration) -> Result<HidppFrame, TransportError> {
        if let Some(item) = self.read_queue.pop_front() {
            return item;
        }
        if self.default_timeout_on_empty {
            Err(TransportError::Timeout)
        } else {
            Err(TransportError::Disconnected)
        }
    }
}

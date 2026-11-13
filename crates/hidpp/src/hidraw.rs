//! Linux `hidraw` transport and sysfs discovery for HID++ 2.0 devices.
//!
//! Enumeration reads only `/sys/class/hidraw` metadata and HID report descriptors so
//! unprivileged callers can detect connected devices and report actionable permission
//! errors before opening `/dev/hidraw*`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::{Errno, read, write};
use tracing::{debug, trace, warn};

use crate::descriptor::{analyze_report_descriptor, parse_sysfs_uevent};
use crate::error::TransportError;
use crate::frame::{HidppFrame, MAX_REPORT_LEN};
use crate::transport::{HidDeviceInfo, HidTransport};

/// USB vendor ID used by HID++ devices targeted by Squeak (`0x046D`).
pub const HIDPP_VENDOR_ID: u16 = 0x046D;

const SYSFS_HIDRAW_DIR: &str = "/sys/class/hidraw";
const DEV_DIR: &str = "/dev";
const WRITE_POLL_TIMEOUT: Duration = Duration::from_millis(500);

/// Discovers connected HID++ `hidraw` endpoints from `/sys/class/hidraw`.
///
/// Only vendor-matched (`0x046D`) interfaces whose HID report descriptor exposes a vendor
/// HID++ collection (`0xFF00..=0xFFFF`) with HID++ report IDs (`0x10`/`0x11`/`0x12`) are
/// returned. Standard pointer/keyboard interfaces are ignored.
///
/// This function never reads `HID_UNIQ` (serial numbers) and never opens `/dev/hidraw*`.
///
/// # Errors
///
/// Returns [`TransportError::Io`] if `/sys/class/hidraw` exists but cannot be read.
pub fn discover_hidpp_devices() -> Result<Vec<HidDeviceInfo>, TransportError> {
    discover_hidpp_devices_in(Path::new(SYSFS_HIDRAW_DIR), Path::new(DEV_DIR))
}

/// Discovers HID++ `hidraw` endpoints under custom sysfs and `/dev` roots.
///
/// Primarily intended for testing sysfs parsing without touching real kernel nodes.
///
/// # Errors
///
/// Returns [`TransportError::Io`] if `sys_class_hidraw` exists and listing it fails.
pub fn discover_hidpp_devices_in(
    sys_class_hidraw: &Path,
    dev_dir: &Path,
) -> Result<Vec<HidDeviceInfo>, TransportError> {
    if !sys_class_hidraw.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(sys_class_hidraw).map_err(|source| TransportError::Io {
        context: format!("reading sysfs directory {}", sys_class_hidraw.display()),
        source,
    })?;

    let mut names: Vec<String> = Vec::new();
    for entry_res in entries {
        let entry = entry_res.map_err(|source| TransportError::Io {
            context: format!("iterating {}", sys_class_hidraw.display()),
            source,
        })?;
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with("hidraw")
        {
            names.push(name.to_owned());
        }
    }

    names.sort_by_key(|name| {
        name.strip_prefix("hidraw")
            .and_then(|num| num.parse::<u32>().ok())
            .map_or((1u8, 0u32, name.clone()), |n| (0u8, n, name.clone()))
    });

    let mut devices = Vec::new();
    for name in names {
        let node_sys_dir = sys_class_hidraw.join(&name).join("device");
        let uevent_path = node_sys_dir.join("uevent");
        let desc_path = node_sys_dir.join("report_descriptor");

        let Ok(uevent_raw) = fs::read_to_string(&uevent_path) else {
            debug!(path = %uevent_path.display(), "skipping hidraw entry without readable uevent");
            continue;
        };
        let uevent = match parse_sysfs_uevent(&uevent_raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                debug!(path = %uevent_path.display(), error = %err, "skipping malformed hidraw uevent");
                continue;
            }
        };

        if uevent.vendor_id != HIDPP_VENDOR_ID {
            continue;
        }

        let Ok(descriptor_bytes) = fs::read(&desc_path) else {
            debug!(
                path = %desc_path.display(),
                vid = format_args!("0x{:04X}", uevent.vendor_id),
                pid = format_args!("0x{:04X}", uevent.product_id),
                "skipping hidraw entry without readable report_descriptor"
            );
            continue;
        };

        let analysis = match analyze_report_descriptor(&descriptor_bytes) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(
                    path = %desc_path.display(),
                    error = %err,
                    "ignoring hidraw node with malformed report descriptor"
                );
                continue;
            }
        };

        if !analysis.is_hidpp {
            trace!(
                node = %name,
                vid = format_args!("0x{:04X}", uevent.vendor_id),
                pid = format_args!("0x{:04X}", uevent.product_id),
                "skipping non-HID++ vendor hidraw interface"
            );
            continue;
        }

        let dev_path = dev_dir.join(&name);
        devices.push(HidDeviceInfo::from_sysfs(
            dev_path,
            uevent,
            analysis.supported_reports,
        ));
    }

    Ok(devices)
}

/// Pure-Linux `hidraw` implementation of [`HidTransport`] backed by safe `rustix` syscalls.
#[derive(Debug)]
pub struct HidrawTransport {
    info: HidDeviceInfo,
    fd: OwnedFd,
}

impl HidrawTransport {
    /// Opens an already discovered [`HidDeviceInfo`] `/dev/hidraw*` node with `O_RDWR | O_NONBLOCK | O_CLOEXEC`.
    ///
    /// # Errors
    ///
    /// - [`TransportError::PermissionDenied`] on `EACCES`/`EPERM` (suggests `squeak setup-udev`)
    /// - [`TransportError::DeviceNotFound`] on `ENOENT`/`ENODEV`/`ENXIO`
    /// - [`TransportError::Io`] on other OS failures
    pub fn open(info: HidDeviceInfo) -> Result<Self, TransportError> {
        let flags = OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let fd = open(&info.path, flags, Mode::empty())
            .map_err(|errno| map_open_errno(&info.path, errno))?;
        Ok(Self { info, fd })
    }

    /// Opens a raw `/dev/hidraw*` path using metadata discovered from `/sys/class/hidraw`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::DeviceNotFound`] or [`TransportError::NotSupportedHidpp`]
    /// if the node is absent or does not expose HID++, or open permission/I/O errors.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, TransportError> {
        let target: PathBuf = path.as_ref().to_path_buf();
        let all = discover_hidpp_devices()?;
        let Some(info) = all.into_iter().find(|dev| dev.path == target) else {
            if target.exists() {
                return Err(TransportError::NotSupportedHidpp {
                    path: target,
                    reason: "device is not a recognized HID++ interface".to_owned(),
                });
            }
            return Err(TransportError::DeviceNotFound { path: target });
        };
        Self::open(info)
    }
}

const MAX_IO_TIMEOUT: Duration = Duration::from_secs(86_400);

pub(crate) fn deadline_after(now: Instant, timeout: Duration) -> Instant {
    now.checked_add(timeout.min(MAX_IO_TIMEOUT)).unwrap_or(now)
}

fn next_remaining_or_timeout(
    deadline: Instant,
    now: Instant,
    first_iter: &mut bool,
) -> Result<Duration, TransportError> {
    let remaining = deadline.saturating_duration_since(now);
    if !*first_iter && remaining.is_zero() {
        return Err(TransportError::Timeout);
    }
    *first_iter = false;
    Ok(remaining)
}

impl HidTransport for HidrawTransport {
    fn device_info(&self) -> &HidDeviceInfo {
        &self.info
    }

    fn write_frame(&mut self, frame: &HidppFrame) -> Result<(), TransportError> {
        if !self.info.supported_reports.supports(frame.report_id()) {
            return Err(TransportError::Io {
                context: format!(
                    "report ID {} unsupported by {}",
                    frame.report_id(),
                    self.info.path.display()
                ),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported HID++ report ID for endpoint",
                ),
            });
        }

        let mut buf = [0u8; MAX_REPORT_LEN];
        let len = frame
            .encode_into(&mut buf)
            .map_err(|err| TransportError::Io {
                context: "encoding HID++ frame".to_owned(),
                source: io::Error::new(io::ErrorKind::InvalidInput, err.to_string()),
            })?;
        let payload = &buf[..len];

        let deadline = deadline_after(Instant::now(), WRITE_POLL_TIMEOUT);
        let mut first_write = true;
        loop {
            let remaining = next_remaining_or_timeout(deadline, Instant::now(), &mut first_write)?;
            match write(&self.fd, payload) {
                Ok(written) if written == len => return Ok(()),
                Ok(written) => {
                    return Err(TransportError::Io {
                        context: format!(
                            "short write to {} (wrote {written} of {len} bytes)",
                            self.info.path.display()
                        ),
                        source: io::Error::new(io::ErrorKind::WriteZero, "incomplete HID write"),
                    });
                }
                Err(Errno::INTR) => continue,
                Err(Errno::AGAIN) => {
                    if remaining.is_zero() {
                        return Err(TransportError::Timeout);
                    }
                    poll_fd(&self.fd, PollFlags::OUT, remaining, &self.info.path)?;
                }
                Err(errno) => {
                    return Err(map_io_errno(&self.info.path, "writing hidraw frame", errno));
                }
            }
        }
    }

    fn read_frame(&mut self, timeout: Duration) -> Result<HidppFrame, TransportError> {
        let deadline = deadline_after(Instant::now(), timeout);
        let mut buf = [0u8; MAX_REPORT_LEN];
        let mut first_read = true;

        loop {
            let remaining = next_remaining_or_timeout(deadline, Instant::now(), &mut first_read)?;
            match read(&self.fd, &mut buf) {
                Ok(0) => return Err(TransportError::Disconnected),
                Ok(n) => {
                    let raw = &buf[..n];
                    if matches!(raw.first().copied(), Some(0x10..=0x12)) {
                        return HidppFrame::decode(raw).map_err(|err| {
                            warn!(
                                path = %self.info.path.display(),
                                len = n,
                                error = %err,
                                "received malformed HID++ report"
                            );
                            TransportError::Io {
                                context: format!(
                                    "decoding HID++ report from {}",
                                    self.info.path.display()
                                ),
                                source: io::Error::new(io::ErrorKind::InvalidData, err.to_string()),
                            }
                        });
                    }
                    // Shared HID interfaces (e.g. consumer keys + HID++) can deliver non-HID++ reports;
                    // ignore them and continue waiting within the caller's deadline.
                    trace!(
                        path = %self.info.path.display(),
                        report_id = raw.first().copied().unwrap_or(0),
                        len = n,
                        "ignoring non-HID++ HID report"
                    );
                }
                Err(Errno::INTR) => continue,
                Err(Errno::AGAIN) => {
                    if remaining.is_zero() {
                        return Err(TransportError::Timeout);
                    }
                    poll_fd(&self.fd, PollFlags::IN, remaining, &self.info.path)?;
                }
                Err(errno) => {
                    return Err(map_io_errno(&self.info.path, "reading hidraw frame", errno));
                }
            }
        }
    }
}

fn poll_fd(
    fd: &OwnedFd,
    events: PollFlags,
    timeout: Duration,
    path: &Path,
) -> Result<(), TransportError> {
    let ts = Timespec::try_from(timeout.min(MAX_IO_TIMEOUT)).map_err(|_| TransportError::Io {
        context: "converting poll timeout".to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidInput, "timeout overflow"),
    })?;

    let mut fds = [PollFd::new(fd, events)];
    match poll(&mut fds, Some(&ts)) {
        Ok(0) => Err(TransportError::Timeout),
        Ok(_) => {
            let revents = fds[0].revents();
            if !revents.intersects(events)
                && revents.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL)
            {
                return Err(TransportError::Disconnected);
            }
            Ok(())
        }
        Err(Errno::INTR) => Ok(()),
        Err(errno) => Err(map_io_errno(path, "polling hidraw fd", errno)),
    }
}

fn map_open_errno(path: &Path, errno: Errno) -> TransportError {
    match errno {
        Errno::ACCESS | Errno::PERM => TransportError::PermissionDenied {
            path: path.to_path_buf(),
        },
        Errno::NOENT | Errno::NODEV | Errno::NXIO => TransportError::DeviceNotFound {
            path: path.to_path_buf(),
        },
        other => TransportError::Io {
            context: format!("opening {}", path.display()),
            source: io::Error::from(other),
        },
    }
}

fn map_io_errno(path: &Path, context: &str, errno: Errno) -> TransportError {
    match errno {
        Errno::NODEV | Errno::NXIO | Errno::IO | Errno::PIPE => TransportError::Disconnected,
        Errno::ACCESS | Errno::PERM => TransportError::PermissionDenied {
            path: path.to_path_buf(),
        },
        other => TransportError::Io {
            context: format!("{context} ({})", path.display()),
            source: io::Error::from(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::frame::SupportedReports;
    use crate::transport::BusType;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "squeak-hidpp-test-{prefix}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn sysfs_discovery_filters_vendor_and_hidpp_interfaces_in_numeric_order() {
        let root = TempDir::new("sysfs");
        let sys_hidraw = root.path.join("sys_class_hidraw");
        let dev_dir = root.path.join("dev");

        // hidraw0: Logitech mouse pointer interface (NOT HID++)
        let h0 = sys_hidraw.join("hidraw0").join("device");
        fs::create_dir_all(&h0).unwrap();
        fs::write(
            h0.join("uevent"),
            "HID_ID=0003:0000046D:0000C08B\nHID_NAME=G502 HERO Gaming Mouse\nHID_UNIQ=DO_NOT_LEAK\n",
        )
        .unwrap();
        fs::write(
            h0.join("report_descriptor"),
            [0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x81, 0x02, 0xC0],
        )
        .unwrap();

        // hidraw10: Logitech BT HID++ long-only interface
        let h10 = sys_hidraw.join("hidraw10").join("device");
        fs::create_dir_all(&h10).unwrap();
        fs::write(
            h10.join("uevent"),
            "HID_ID=0005:0000046D:0000B034\nHID_NAME=Wireless Mouse\n",
        )
        .unwrap();
        fs::write(
            h10.join("report_descriptor"),
            [
                0x06, 0x43, 0xFF, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x11, 0x81, 0x00, 0xC0,
            ],
        )
        .unwrap();

        // hidraw2: Logitech USB HID++ short+long interface (should sort before hidraw10!)
        let h2 = sys_hidraw.join("hidraw2").join("device");
        fs::create_dir_all(&h2).unwrap();
        fs::write(
            h2.join("uevent"),
            "HID_ID=0003:0000046D:0000C08B\nHID_NAME=G502 HERO Gaming Mouse\nHID_UNIQ=ANOTHER_SERIAL\n",
        )
        .unwrap();
        #[rustfmt::skip]
        let desc2 = [
            0x06, 0x00, 0xFF, 0x09, 0x01, 0xA1, 0x01,
            0x85, 0x10, 0x81, 0x00, 0x91, 0x00,
            0x85, 0x11, 0x81, 0x00, 0x91, 0x00,
            0xC0,
        ];
        fs::write(h2.join("report_descriptor"), desc2).unwrap();

        // hidraw3: Non-Logitech vendor HID device with 0xFF00 page (must be ignored)
        let h3 = sys_hidraw.join("hidraw3").join("device");
        fs::create_dir_all(&h3).unwrap();
        fs::write(
            h3.join("uevent"),
            "HID_ID=0003:00001234:00005678\nHID_NAME=Other Device\n",
        )
        .unwrap();
        fs::write(h3.join("report_descriptor"), desc2).unwrap();

        let found = discover_hidpp_devices_in(&sys_hidraw, &dev_dir).unwrap();
        assert_eq!(found.len(), 2);

        assert_eq!(found[0].path, dev_dir.join("hidraw2"));
        assert_eq!(found[0].bus, BusType::Usb);
        assert_eq!(found[0].vendor_id, 0x046D);
        assert_eq!(found[0].product_id, 0xC08B);
        assert_eq!(found[0].supported_reports, SupportedReports::SHORT_AND_LONG);

        assert_eq!(found[1].path, dev_dir.join("hidraw10"));
        assert_eq!(found[1].bus, BusType::Bluetooth);
        assert_eq!(found[1].vendor_id, 0x046D);
        assert_eq!(found[1].product_id, 0xB034);
        assert_eq!(found[1].supported_reports, SupportedReports::LONG_ONLY);
    }

    #[test]
    fn sysfs_discovery_returns_empty_when_dir_missing() {
        let missing = Path::new("/nonexistent/squeak-sysfs-dir");
        let found = discover_hidpp_devices_in(missing, Path::new("/dev")).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn deadline_helper_clamps_duration_max_and_enforces_expiry_after_first_iter() {
        let now = Instant::now();
        let clamped = deadline_after(now, Duration::MAX);
        assert_eq!(clamped.saturating_duration_since(now), MAX_IO_TIMEOUT);

        let expired_deadline = now;
        let mut first_iter = true;
        // First iteration always runs at least one poll/read check.
        let first = next_remaining_or_timeout(expired_deadline, now, &mut first_iter).unwrap();
        assert!(first.is_zero());
        assert!(!first_iter);

        // Subsequent iteration with expired deadline must return Timeout rather than looping forever.
        let later = now + Duration::from_millis(1);
        let second = next_remaining_or_timeout(expired_deadline, later, &mut first_iter);
        assert!(matches!(second, Err(TransportError::Timeout)));
    }
}

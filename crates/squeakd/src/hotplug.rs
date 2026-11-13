//! Linux `NETLINK_KOBJECT_UEVENT` hotplug monitor for `hidraw` devices.
//!
//! Uses pure `rustix` syscalls (`#![forbid(unsafe_code)]`) without C `libudev` FFI or
//! collecting any device serial numbers (`HID_UNIQ` is intentionally ignored).

use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, bind, recv, socket_with};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Kernel netlink protocol constant for `NETLINK_KOBJECT_UEVENT`.
const NETLINK_KOBJECT_UEVENT: u32 = 15;
/// Multicast group `1` (kernel uevents).
const UEVENT_MCAST_GROUP_KERNEL: u32 = 1;
/// Poll slice in milliseconds so shutdown flags are observed promptly.
const POLL_SLICE_MS: i32 = 250;

/// Action reported by a kernel `hidraw` uevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugAction {
    /// Device node created (`ACTION=add`).
    Add,
    /// Device node removed (`ACTION=remove`).
    Remove,
    /// Device state changed (`ACTION=change`).
    Change,
    /// Other uevent action (`bind`, `unbind`, etc.).
    Other,
}

impl HotplugAction {
    fn parse(raw: &str) -> Self {
        match raw {
            "add" => Self::Add,
            "remove" => Self::Remove,
            "change" => Self::Change,
            _ => Self::Other,
        }
    }
}

/// Sanitized `hidraw` hotplug event parsed from a kernel uevent datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidrawUevent {
    /// Event action (`add`, `remove`, `change`, ...).
    pub action: HotplugAction,
    /// Kernel device name relative to `/dev` (e.g. `"hidraw6"`).
    pub devname: Option<String>,
}

/// Pure parser for a raw NUL-separated `NETLINK_KOBJECT_UEVENT` payload.
///
/// Returns `Some` only when `SUBSYSTEM=hidraw` is present. Any serial-like keys
/// (`HID_UNIQ`, `SERIAL`, etc.) are discarded and never returned.
#[must_use]
pub fn parse_uevent_datagram(buf: &[u8]) -> Option<HidrawUevent> {
    let mut action: Option<HotplugAction> = None;
    let mut subsystem: Option<&str> = None;
    let mut devname: Option<String> = None;

    for part in buf.split(|&b| b == 0) {
        let Ok(line) = std::str::from_utf8(part) else {
            continue;
        };
        if let Some((key, val)) = line.split_once('=') {
            match key {
                "ACTION" => action = Some(HotplugAction::parse(val)),
                "SUBSYSTEM" => subsystem = Some(val),
                "DEVNAME" => {
                    let clean = val.strip_prefix("/dev/").unwrap_or(val);
                    if !clean.is_empty() {
                        devname = Some(clean.to_owned());
                    }
                }
                _ => {}
            }
        }
    }

    if subsystem != Some("hidraw") {
        return None;
    }

    Some(HidrawUevent {
        action: action.unwrap_or(HotplugAction::Other),
        devname,
    })
}

/// Opens a non-blocking `NETLINK_KOBJECT_UEVENT` socket bound to kernel events and spawns a task
/// forwarding `SUBSYSTEM=hidraw` events to `tx` until `shutdown` becomes `true`.
///
/// If netlink binding fails (e.g. restricted sandbox environments), logs a warning and returns `None`
/// cleanly so the daemon continues working via D-Bus `RefreshDevices`.
pub fn spawn_netlink_hotplug_watcher(
    tx: mpsc::Sender<HidrawUevent>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    let fd = match open_uevent_netlink_socket() {
        Ok(fd) => fd,
        Err(err) => {
            warn!(%err, "failed to open NETLINK_KOBJECT_UEVENT socket; hotplug disabled");
            return None;
        }
    };

    let handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        while !shutdown.load(Ordering::Relaxed) {
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timeout = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: i64::from(POLL_SLICE_MS) * 1_000_000,
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => continue,
                Ok(_) => {
                    if !fds[0].revents().contains(PollFlags::IN) {
                        continue;
                    }
                    loop {
                        match recv_uevent(&fd, &mut buf) {
                            Ok(0) => return,
                            Ok(n) => {
                                if let Some(ev) = parse_uevent_datagram(&buf[..n]) {
                                    debug!(
                                        ?ev.action,
                                        devname = ?ev.devname,
                                        "hidraw uevent received"
                                    );
                                    if tx.blocking_send(ev).is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(err) => {
                                warn!(%err, "netlink recv error; stopping hotplug watcher");
                                return;
                            }
                        }
                    }
                }
                Err(rustix::io::Errno::INTR) => continue,
                Err(err) => {
                    warn!(%err, "netlink poll error; stopping hotplug watcher");
                    return;
                }
            }
        }
    });

    Some(handle)
}

fn open_uevent_netlink_socket() -> std::io::Result<OwnedFd> {
    let proto = rustix::net::Protocol::from_raw(
        std::num::NonZeroU32::new(NETLINK_KOBJECT_UEVENT).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid netlink protocol")
        })?,
    );
    let fd = socket_with(
        AddressFamily::NETLINK,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC,
        Some(proto),
    )
    .map_err(std::io::Error::from)?;

    let flags = fcntl_getfl(&fd).map_err(std::io::Error::from)?;
    fcntl_setfl(&fd, flags | OFlags::NONBLOCK).map_err(std::io::Error::from)?;

    let addr = SocketAddrNetlink::new(0, UEVENT_MCAST_GROUP_KERNEL);
    bind(fd.as_fd(), &addr).map_err(std::io::Error::from)?;
    Ok(fd)
}

fn recv_uevent(fd: &OwnedFd, buf: &mut [u8]) -> std::io::Result<usize> {
    let (len, _) = recv(fd, buf, RecvFlags::empty()).map_err(std::io::Error::from)?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hidraw_add_and_remove_and_ignores_other_subsystems() {
        let add_payload = b"add@/devices/pci0000:00/hidraw/hidraw6\0ACTION=add\0DEVPATH=/devices/pci0000:00/hidraw/hidraw6\0SUBSYSTEM=hidraw\0DEVNAME=hidraw6\0HID_UNIQ=SECRET123456\0";
        let ev = parse_uevent_datagram(add_payload).unwrap();
        assert_eq!(ev.action, HotplugAction::Add);
        assert_eq!(ev.devname.as_deref(), Some("hidraw6"));

        let remove_payload = b"remove@/devices/hidraw/hidraw6\0ACTION=remove\0SUBSYSTEM=hidraw\0DEVNAME=/dev/hidraw6\0";
        let rem = parse_uevent_datagram(remove_payload).unwrap();
        assert_eq!(rem.action, HotplugAction::Remove);
        assert_eq!(rem.devname.as_deref(), Some("hidraw6"));

        let usb_payload = b"add@/devices/usb\0ACTION=add\0SUBSYSTEM=usb\0DEVNAME=bus/usb/001/002\0";
        assert!(parse_uevent_datagram(usb_payload).is_none());
    }
}

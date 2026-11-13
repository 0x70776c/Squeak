//! Pure-Rust USB HID report descriptor and Linux sysfs `uevent` parsers.
//!
//! Parsing the raw HID report descriptor lets Squeak identify which `hidraw` nodes
//! expose Logitech vendor HID++ collections (`0xFF00` / `0xFF43`) and which fixed
//! report sizes (`0x10`, `0x11`, `0x12`) are supported without opening the device.

use crate::error::ProtocolError;
use crate::frame::SupportedReports;

/// Parsed bus/ID metadata extracted from `/sys/class/hidraw/<node>/device/uevent`.
///
/// Serial numbers (`HID_UNIQ`) are intentionally never extracted or retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysfsUeventInfo {
    /// Linux HID bus constant (`0x0003` = USB, `0x0005` = Bluetooth, etc.).
    pub bus_type: u16,
    /// USB/BT vendor ID (e.g. `0x046D`).
    pub vendor_id: u16,
    /// USB/BT product ID (e.g. `0xC08B`).
    pub product_id: u16,
    /// Human-readable device name reported in `HID_NAME`, if present and non-empty.
    pub name: Option<String>,
}

/// Parses a Linux sysfs `hid/device/uevent` text file.
///
/// Any `HID_UNIQ` (serial number) or `HID_PHYS` field is deliberately discarded.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidDescriptor`] if `HID_ID` is missing or malformed.
pub fn parse_sysfs_uevent(contents: &str) -> Result<SysfsUeventInfo, ProtocolError> {
    let mut hid_id: Option<(u16, u16, u16)> = None;
    let mut name: Option<String> = None;

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if let Some(val) = line.strip_prefix("HID_ID=") {
            hid_id = Some(parse_hid_id(val)?);
        } else if let Some(val) = line.strip_prefix("HID_NAME=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                name = Some(trimmed.to_owned());
            }
        }
        // Deliberately ignore HID_UNIQ (serial numbers) and HID_PHYS.
    }

    let (bus_type, vendor_id, product_id) = hid_id.ok_or_else(|| {
        ProtocolError::InvalidDescriptor("sysfs uevent missing HID_ID entry".to_owned())
    })?;

    Ok(SysfsUeventInfo {
        bus_type,
        vendor_id,
        product_id,
        name,
    })
}

fn parse_hid_id(value: &str) -> Result<(u16, u16, u16), ProtocolError> {
    let mut parts = value.split(':');
    let (Some(bus_str), Some(vid_str), Some(pid_str), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ProtocolError::InvalidDescriptor(format!(
            "malformed HID_ID value: {value:?}"
        )));
    };

    let parse_hex_u32 = |s: &str, field: &str| -> Result<u16, ProtocolError> {
        let parsed = u32::from_str_radix(s, 16).map_err(|_| {
            ProtocolError::InvalidDescriptor(format!("invalid hex {field} in HID_ID: {s:?}"))
        })?;
        u16::try_from(parsed).map_err(|_| {
            ProtocolError::InvalidDescriptor(format!("{field} out of 16-bit range: {parsed:#x}"))
        })
    };

    let bus = parse_hex_u32(bus_str, "bus")?;
    let vid = parse_hex_u32(vid_str, "vendor_id")?;
    let pid = parse_hex_u32(pid_str, "product_id")?;
    Ok((bus, vid, pid))
}

/// Summary of an inspected USB/BT HID report descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorAnalysis {
    /// True if at least one vendor-defined HID++ usage page (`0xFF00..=0xFFFF`) with
    /// HID++ report IDs (`0x10`, `0x11`, or `0x12`) was found.
    pub is_hidpp: bool,
    /// Exact set of HID++ report IDs declared under vendor HID++ collections.
    pub supported_reports: SupportedReports,
}

#[derive(Debug, Clone, Copy, Default)]
struct GlobalState {
    usage_page: u32,
    report_id: Option<u8>,
}

/// Parses raw USB HID report descriptor bytes and detects Logitech HID++ interfaces.
///
/// Only reports declared inside a vendor usage page (`0xFF00..=0xFFFF`, such as `0xFF00`
/// for classic HID++ 2.0 or `0xFF43` for Bluetooth/Bolt collections) with report IDs
/// `0x10`, `0x11`, or `0x12` are counted. Standard mouse/keyboard interfaces on usage
/// pages `0x01`/`0x07`/`0x0C` yield `is_hidpp = false`.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidDescriptor`] if an item extends past the end of `bytes`
/// or exceeds the bounded global-state stack.
pub fn analyze_report_descriptor(bytes: &[u8]) -> Result<DescriptorAnalysis, ProtocolError> {
    const MAX_PUSH_STACK: usize = 32;

    let mut supported = SupportedReports {
        short: false,
        long: false,
        very_long: false,
    };
    let mut state = GlobalState::default();
    let mut stack: Vec<GlobalState> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let prefix = bytes[i];
        i += 1;

        // Long item format (USB HID 1.11 section 6.2.2.3): prefix 0xFE, length byte, tag byte, data...
        if prefix == 0xFE {
            let Some(&data_len) = bytes.get(i) else {
                return Err(ProtocolError::InvalidDescriptor(
                    "truncated HID descriptor long-item length".to_owned(),
                ));
            };
            let skip = 2usize + usize::from(data_len);
            if bytes.len().saturating_sub(i) < skip {
                return Err(ProtocolError::InvalidDescriptor(
                    "truncated HID descriptor long-item payload".to_owned(),
                ));
            }
            i += skip;
            continue;
        }

        let size_code = prefix & 0x03;
        let data_len = match size_code {
            0 => 0usize,
            1 => 1usize,
            2 => 2usize,
            _ => 4usize,
        };
        let Some(data) = i.checked_add(data_len).and_then(|end| bytes.get(i..end)) else {
            return Err(ProtocolError::InvalidDescriptor(format!(
                "truncated HID descriptor short item at offset {}",
                i - 1
            )));
        };
        i += data_len;

        let val = decode_le_u32(data);
        let item_type = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0F;

        match item_type {
            // Global items
            1 => match tag {
                // Usage Page
                0x0 => state.usage_page = val,
                // Report ID
                0x8 => {
                    if let Ok(id) = u8::try_from(val) {
                        state.report_id = Some(id);
                    } else {
                        state.report_id = None;
                    }
                }
                // Push
                0xA => {
                    if stack.len() >= MAX_PUSH_STACK {
                        return Err(ProtocolError::InvalidDescriptor(
                            "HID descriptor global stack overflow".to_owned(),
                        ));
                    }
                    stack.push(state);
                }
                // Pop
                0xB => {
                    state = stack.pop().ok_or_else(|| {
                        ProtocolError::InvalidDescriptor(
                            "HID descriptor global stack underflow".to_owned(),
                        )
                    })?;
                }
                _ => {}
            },
            // Main items (Input = 0x8, Output = 0x9, Feature = 0xB)
            0 if matches!(tag, 0x8 | 0x9 | 0xB) => {
                if (0xFF00..=0xFFFF).contains(&state.usage_page)
                    && let Some(rid) = state.report_id
                {
                    match rid {
                        0x10 => supported.short = true,
                        0x11 => supported.long = true,
                        0x12 => supported.very_long = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(DescriptorAnalysis {
        is_hidpp: supported.any(),
        supported_reports: supported,
    })
}

fn decode_le_u32(data: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    for (dst, &src) in buf.iter_mut().zip(data.iter()) {
        *dst = src;
    }
    u32::from_le_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sysfs_uevent_and_drops_serial() {
        let sample = "\
DRIVER=hid-generic
HID_ID=0003:0000046D:0000C08B
HID_NAME=Logitech G502 HERO Gaming Mouse
HID_PHYS=usb-0000:00:14.0-1/input1
HID_UNIQ=SECRET_SERIAL_123456
MODALIAS=hid:b0003g0001v0000046Dp0000C08B
";
        let info = parse_sysfs_uevent(sample).unwrap();
        assert_eq!(info.bus_type, 0x0003);
        assert_eq!(info.vendor_id, 0x046D);
        assert_eq!(info.product_id, 0xC08B);
        assert_eq!(
            info.name.as_deref(),
            Some("Logitech G502 HERO Gaming Mouse")
        );
        // Ensure debug representation never carries HID_UNIQ.
        let dbg = format!("{info:?}");
        assert!(!dbg.contains("SECRET_SERIAL_123456"));
    }

    #[test]
    fn detects_standard_hidpp_short_and_long_descriptor() {
        // Composite keyboard + HID++ vendor descriptor (typical wired mouse interface 1):
        // Keyboard collection (Report ID 0x01) + Vendor 0xFF00 collection (Report IDs 0x10 & 0x11).
        #[rustfmt::skip]
        let desc: &[u8] = &[
            0x05, 0x01,       // Usage Page (Generic Desktop)
            0x09, 0x06,       // Usage (Keyboard)
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x01,       //   Report ID (0x01)
            0x81, 0x02,       //   Input (Data,Var,Abs)
            0xC0,             // End Collection
            0x06, 0x00, 0xFF, // Usage Page (Vendor Defined 0xFF00)
            0x09, 0x01,       // Usage (0x01)
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x10,       //   Report ID (0x10)
            0x75, 0x08,       //   Report Size (8)
            0x95, 0x06,       //   Report Count (6)
            0x81, 0x00,       //   Input (Data,Ary,Abs)
            0x91, 0x00,       //   Output (Data,Ary,Abs)
            0x85, 0x11,       //   Report ID (0x11)
            0x75, 0x08,       //   Report Size (8)
            0x95, 0x13,       //   Report Count (19)
            0x81, 0x00,       //   Input (Data,Ary,Abs)
            0x91, 0x00,       //   Output (Data,Ary,Abs)
            0xC0,             // End Collection
        ];

        let analysis = analyze_report_descriptor(desc).unwrap();
        assert!(analysis.is_hidpp);
        assert_eq!(analysis.supported_reports, SupportedReports::SHORT_AND_LONG);
    }

    #[test]
    fn detects_bluetooth_long_only_ff43_descriptor() {
        #[rustfmt::skip]
        let desc: &[u8] = &[
            0x06, 0x43, 0xFF, // Usage Page (Vendor Defined 0xFF43)
            0x09, 0x02,       // Usage (0x02)
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x11,       //   Report ID (0x11)
            0x81, 0x00,       //   Input
            0x91, 0x00,       //   Output
            0xC0,             // End Collection
        ];

        let analysis = analyze_report_descriptor(desc).unwrap();
        assert!(analysis.is_hidpp);
        assert_eq!(analysis.supported_reports, SupportedReports::LONG_ONLY);
    }

    #[test]
    fn ignores_plain_boot_mouse_interface() {
        #[rustfmt::skip]
        let desc: &[u8] = &[
            0x05, 0x01,       // Usage Page (Generic Desktop)
            0x09, 0x02,       // Usage (Mouse)
            0xA1, 0x01,       // Collection (Application)
            0x09, 0x01,       //   Usage (Pointer)
            0x81, 0x02,       //   Input
            0xC0,             // End Collection
        ];

        let analysis = analyze_report_descriptor(desc).unwrap();
        assert!(!analysis.is_hidpp);
        assert!(!analysis.supported_reports.any());
    }

    #[test]
    fn rejects_truncated_descriptor_without_panicking() {
        // Claims 4-byte item (size_code == 3) but only 1 byte follows.
        assert!(matches!(
            analyze_report_descriptor(&[0x07, 0x00]),
            Err(ProtocolError::InvalidDescriptor(_))
        ));
        assert!(matches!(
            analyze_report_descriptor(&[0xFE]),
            Err(ProtocolError::InvalidDescriptor(_))
        ));
    }
}

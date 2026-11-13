//! Pure HID++ 2.0 protocol library for Squeak.
//!
//! This crate intentionally has no GUI, udev, or D-Bus dependencies. All I/O goes
//! through a `HidTransport` trait implemented by both Linux `hidraw` and test mocks.

/// Returns the crate name for smoke-test verification.
#[must_use]
pub const fn crate_name() -> &'static str {
    "hidpp"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        assert_eq!(crate_name(), "hidpp");
    }
}

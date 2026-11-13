//! Asynchronous D-Bus proxy client for `dev.squeak.Daemon`.
//!
//! By architectural rule (ADR-001), `squeak-gui` contains zero HID++ protocol
//! logic and communicates exclusively with `squeakd` over D-Bus.

use std::fmt;

use serde::de::DeserializeOwned;
use squeak_core::{
    ButtonsReport, DaemonDeviceSummary, DeviceStatusReport, DpiWriteReport, LedConfigReport,
    LedWriteReport, OnboardModeReport, OnboardModeWriteReport, ProfileBackupReport,
    ProfileSetActiveReport, ProfilesListReport, ReportRateWriteReport, ScrollConfigReport,
};
use zbus::Connection;

/// Well-known user session D-Bus service name.
pub const DBUS_BUS_NAME: &str = "dev.squeak.Daemon";
/// D-Bus object path serving `dev.squeak.Daemon`.
pub const DBUS_OBJECT_PATH: &str = "/dev/squeak/Daemon";

/// Errors returned by [`DaemonClient`].
#[derive(Debug)]
pub enum ClientError {
    /// D-Bus transport or method invocation error.
    Dbus(zbus::Error),
    /// JSON deserialization of daemon payload failed.
    Json(serde_json::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dbus(err) => write!(f, "D-Bus error: {err}"),
            Self::Json(err) => write!(f, "invalid daemon JSON payload: {err}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<zbus::Error> for ClientError {
    fn from(err: zbus::Error) -> Self {
        Self::Dbus(err)
    }
}

impl From<serde_json::Error> for ClientError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

mod proxy_iface {
    #![allow(missing_docs)]
    use zbus::proxy;

    /// Generated `zbus` proxy for `dev.squeak.Daemon`.
    #[proxy(
        interface = "dev.squeak.Daemon",
        default_service = "dev.squeak.Daemon",
        default_path = "/dev/squeak/Daemon"
    )]
    pub trait SqueakDaemon {
        /// Advertised D-Bus API version.
        #[zbus(property)]
        fn api_version(&self) -> zbus::Result<u32>;

        /// Running daemon version.
        #[zbus(property)]
        fn daemon_version(&self) -> zbus::Result<String>;

        /// Lists all tracked devices as JSON.
        fn list_devices(&self) -> zbus::Result<String>;

        /// Reads M3 device status as JSON.
        fn get_device_status(&self, device_key: &str) -> zbus::Result<String>;

        /// Sets DPI (`dry_run = true` previews, `dry_run = false` applies).
        fn set_dpi(
            &self,
            device_key: &str,
            sensor_index: u8,
            dpi: u16,
            dry_run: bool,
        ) -> zbus::Result<String>;

        /// Sets polling rate (`dry_run = true` previews, `dry_run = false` applies).
        fn set_report_rate(&self, device_key: &str, hz: u16, dry_run: bool)
        -> zbus::Result<String>;

        /// Reads RGB LED zone configuration (`0x8070`).
        fn get_led_config(&self, device_key: &str) -> zbus::Result<String>;

        /// Sets RGB LED effect (`0x8070`).
        #[allow(clippy::too_many_arguments)]
        fn set_led_effect(
            &self,
            device_key: &str,
            zone_index: u8,
            effect: &str,
            color_hex: &str,
            period_ms: u16,
            brightness_pct: u8,
            dry_run: bool,
        ) -> zbus::Result<String>;

        /// Reads onboard/host mode (`0x8100`).
        fn get_onboard_mode(&self, device_key: &str) -> zbus::Result<String>;

        /// Switches operating mode (`"host"` / `"onboard"`).
        fn set_onboard_mode(
            &self,
            device_key: &str,
            mode: &str,
            dry_run: bool,
        ) -> zbus::Result<String>;

        /// Lists onboard profiles (`0x8100`).
        fn list_profiles(&self, device_key: &str) -> zbus::Result<String>;

        /// Switches active onboard profile slot (`0x8100`).
        fn set_active_profile(
            &self,
            device_key: &str,
            slot: u16,
            dry_run: bool,
        ) -> zbus::Result<String>;

        /// Backs up onboard profile memory (`0x8100`).
        fn backup_profiles(&self, device_key: &str, output_path: &str) -> zbus::Result<String>;

        /// Reads button capabilities and layout metadata.
        fn get_buttons(&self, device_key: &str) -> zbus::Result<String>;

        /// Reads scroll wheel capabilities and settings.
        fn get_scroll_config(&self, device_key: &str) -> zbus::Result<String>;

        /// Re-scans endpoints.
        fn refresh_devices(&self) -> zbus::Result<u32>;

        /// Signal emitted when devices are added or removed.
        #[zbus(signal)]
        fn devices_changed(&self, added: Vec<String>, removed: Vec<String>) -> zbus::Result<()>;

        /// Signal emitted when settings are applied and verified.
        #[zbus(signal)]
        fn device_settings_applied(&self, device_key: &str, setting: &str) -> zbus::Result<()>;

        /// Signal emitted when a device operation fails.
        #[zbus(signal)]
        fn device_error(&self, device_key: &str, message: &str) -> zbus::Result<()>;
    }
}

pub use proxy_iface::SqueakDaemonProxy;

/// High-level typed async D-Bus client wrapping [`SqueakDaemonProxy`].
#[derive(Clone)]
pub struct DaemonClient {
    proxy: SqueakDaemonProxy<'static>,
}

impl DaemonClient {
    /// Connects to the user session D-Bus and builds a [`SqueakDaemonProxy`].
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Dbus`] if the session bus or daemon proxy cannot be created.
    pub async fn connect() -> Result<Self, ClientError> {
        let conn = Connection::session().await?;
        let proxy = SqueakDaemonProxy::new(&conn).await?;
        Ok(Self { proxy })
    }

    /// Returns a reference to the underlying `zbus` proxy (e.g. for subscribing to signals).
    #[must_use]
    pub const fn proxy(&self) -> &SqueakDaemonProxy<'static> {
        &self.proxy
    }

    /// Lists all tracked HID++ devices.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn list_devices(&self) -> Result<Vec<DaemonDeviceSummary>, ClientError> {
        parse_json(&self.proxy.list_devices().await?)
    }

    /// Reads live M3 status for `device_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn get_device_status(
        &self,
        device_key: &str,
    ) -> Result<DeviceStatusReport, ClientError> {
        parse_json(&self.proxy.get_device_status(device_key).await?)
    }

    /// Sets DPI on `device_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn set_dpi(
        &self,
        device_key: &str,
        sensor_index: u8,
        dpi: u16,
        dry_run: bool,
    ) -> Result<DpiWriteReport, ClientError> {
        parse_json(
            &self
                .proxy
                .set_dpi(device_key, sensor_index, dpi, dry_run)
                .await?,
        )
    }

    /// Sets polling rate on `device_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn set_report_rate(
        &self,
        device_key: &str,
        hz: u16,
        dry_run: bool,
    ) -> Result<ReportRateWriteReport, ClientError> {
        parse_json(&self.proxy.set_report_rate(device_key, hz, dry_run).await?)
    }

    /// Reads RGB LED configuration (`0x8070`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn get_led_config(&self, device_key: &str) -> Result<LedConfigReport, ClientError> {
        parse_json(&self.proxy.get_led_config(device_key).await?)
    }

    /// Applies or previews an RGB LED effect (`0x8070`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_led_effect(
        &self,
        device_key: &str,
        zone_index: u8,
        effect: &str,
        color_hex: &str,
        period_ms: u16,
        brightness_pct: u8,
        dry_run: bool,
    ) -> Result<LedWriteReport, ClientError> {
        parse_json(
            &self
                .proxy
                .set_led_effect(
                    device_key,
                    zone_index,
                    effect,
                    color_hex,
                    period_ms,
                    brightness_pct,
                    dry_run,
                )
                .await?,
        )
    }

    /// Reads onboard/host mode (`0x8100`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn get_onboard_mode(
        &self,
        device_key: &str,
    ) -> Result<OnboardModeReport, ClientError> {
        parse_json(&self.proxy.get_onboard_mode(device_key).await?)
    }

    /// Switches operating mode (`"host"` / `"onboard"`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn set_onboard_mode(
        &self,
        device_key: &str,
        mode: &str,
        dry_run: bool,
    ) -> Result<OnboardModeWriteReport, ClientError> {
        parse_json(
            &self
                .proxy
                .set_onboard_mode(device_key, mode, dry_run)
                .await?,
        )
    }

    /// Lists onboard profiles (`0x8100`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn list_profiles(&self, device_key: &str) -> Result<ProfilesListReport, ClientError> {
        parse_json(&self.proxy.list_profiles(device_key).await?)
    }

    /// Switches active onboard profile slot (`0x8100`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn set_active_profile(
        &self,
        device_key: &str,
        slot: u16,
        dry_run: bool,
    ) -> Result<ProfileSetActiveReport, ClientError> {
        parse_json(
            &self
                .proxy
                .set_active_profile(device_key, slot, dry_run)
                .await?,
        )
    }

    /// Backs up onboard profile memory (`0x8100`).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn backup_profiles(
        &self,
        device_key: &str,
        output_path: &str,
    ) -> Result<ProfileBackupReport, ClientError> {
        parse_json(&self.proxy.backup_profiles(device_key, output_path).await?)
    }

    /// Reads button capabilities and SVG layout metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn get_buttons(&self, device_key: &str) -> Result<ButtonsReport, ClientError> {
        parse_json(&self.proxy.get_buttons(device_key).await?)
    }

    /// Reads scroll wheel capabilities and settings.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus or JSON failure.
    pub async fn get_scroll_config(
        &self,
        device_key: &str,
    ) -> Result<ScrollConfigReport, ClientError> {
        parse_json(&self.proxy.get_scroll_config(device_key).await?)
    }

    /// Triggers endpoint re-scan in `squeakd`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on D-Bus failure.
    pub async fn refresh_devices(&self) -> Result<u32, ClientError> {
        Ok(self.proxy.refresh_devices().await?)
    }
}

fn parse_json<T: DeserializeOwned>(raw: &str) -> Result<T, ClientError> {
    Ok(serde_json::from_str(raw)?)
}

//! Versioned D-Bus service interface (`dev.squeak.Daemon` via `zbus`) and `login1` wake
//! listener.

use std::sync::Arc;

use futures_lite::StreamExt;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::{Connection, MatchRule, MessageStream, interface, message};

use crate::error::DaemonError;
use crate::registry::{DeviceBackend, DeviceRegistry, SyncOutcome, WakeOutcome};

/// Well-known user session D-Bus service name.
pub const DBUS_BUS_NAME: &str = "dev.squeak.Daemon";
/// D-Bus object path serving [`SqueakDaemonInterface`].
pub const DBUS_OBJECT_PATH: &str = "/dev/squeak/Daemon";
/// Versioned D-Bus interface name.
pub const DBUS_INTERFACE_NAME: &str = "dev.squeak.Daemon";
/// Protocol/API revision advertised via the `ApiVersion` property.
pub const DBUS_API_VERSION: u32 = 1;

/// Thread-safe serialized handle to the single-owner [`DeviceRegistry`].
pub type SharedRegistry<B> = Arc<Mutex<DeviceRegistry<B>>>;

/// `zbus` object serving `dev.squeak.Daemon` on `/dev/squeak/Daemon`.
pub struct SqueakDaemonInterface<B: DeviceBackend> {
    registry: SharedRegistry<B>,
}

impl<B: DeviceBackend> SqueakDaemonInterface<B> {
    /// Creates a new D-Bus interface wrapping `registry`.
    #[must_use]
    pub fn new(registry: SharedRegistry<B>) -> Self {
        Self { registry }
    }
}

#[allow(missing_docs)]
#[interface(name = "dev.squeak.Daemon")]
impl<B: DeviceBackend> SqueakDaemonInterface<B> {
    /// Advertised D-Bus API version (`1`).
    #[zbus(property)]
    async fn api_version(&self) -> u32 {
        DBUS_API_VERSION
    }

    /// Running daemon package version.
    #[zbus(property)]
    async fn daemon_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Lists all tracked HID++ devices as JSON `Vec<DaemonDeviceSummary>` (zero serial numbers).
    async fn list_devices(&self) -> zbus::fdo::Result<String> {
        let summaries = {
            let guard = self.registry.lock().await;
            guard.list_devices()
        };
        serde_json::to_string_pretty(&summaries)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Reads live M3 status as JSON `DeviceStatusReport` for `device_key` / selector.
    async fn get_device_status(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.get_status(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Returns the sanitized JSON `DeviceDump` for `device_key` / selector.
    async fn dump_device(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let dump = {
            let mut guard = self.registry.lock().await;
            guard.dump_device(device_key)
        }?;
        serde_json::to_string_pretty(&dump)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Validates and previews (`dry_run = true`) or applies + verifies (`dry_run = false`) DPI.
    async fn set_dpi(
        &self,
        device_key: &str,
        sensor_index: u8,
        dpi: u16,
        dry_run: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let res = {
            let mut guard = self.registry.lock().await;
            guard.set_dpi(device_key, sensor_index, dpi, dry_run)
        };
        match res {
            Ok(report) => {
                if report.applied && report.verified {
                    let setting = format!("dpi:{sensor_index}={dpi}");
                    let _ = Self::device_settings_applied(&emitter, device_key, &setting).await;
                }
                serde_json::to_string_pretty(&report)
                    .map_err(DaemonError::Json)
                    .map_err(Into::into)
            }
            Err(err) => {
                let _ = Self::device_error(&emitter, device_key, &err.to_string()).await;
                Err(err.into())
            }
        }
    }

    /// Validates and previews (`dry_run = true`) or applies + verifies (`dry_run = false`) rate.
    async fn set_report_rate(
        &self,
        device_key: &str,
        hz: u16,
        dry_run: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let res = {
            let mut guard = self.registry.lock().await;
            guard.set_report_rate(device_key, hz, dry_run)
        };
        match res {
            Ok(report) => {
                if report.applied && report.verified {
                    let setting = format!("report_rate={hz}Hz");
                    let _ = Self::device_settings_applied(&emitter, device_key, &setting).await;
                }
                serde_json::to_string_pretty(&report)
                    .map_err(DaemonError::Json)
                    .map_err(Into::into)
            }
            Err(err) => {
                let _ = Self::device_error(&emitter, device_key, &err.to_string()).await;
                Err(err.into())
            }
        }
    }

    /// Reads RGB LED zone configuration (`0x8070`) as JSON `LedConfigReport`.
    async fn get_led_config(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.get_led_config(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Validates and previews (`dry_run = true`) or applies (`dry_run = false`) an LED effect.
    #[allow(clippy::too_many_arguments)]
    async fn set_led_effect(
        &self,
        device_key: &str,
        zone_index: u8,
        effect: &str,
        color_hex: &str,
        period_ms: u16,
        brightness_pct: u8,
        dry_run: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let color_opt = if color_hex.trim().is_empty() {
            None
        } else {
            Some(color_hex)
        };
        let period_opt = if period_ms == 0 {
            None
        } else {
            Some(period_ms)
        };
        let bright_opt = if brightness_pct == 0 {
            None
        } else {
            Some(brightness_pct)
        };
        let res = {
            let mut guard = self.registry.lock().await;
            guard.set_led_effect(
                device_key, zone_index, effect, color_opt, period_opt, bright_opt, dry_run,
            )
        };
        match res {
            Ok(report) => {
                if report.applied && report.verified {
                    let setting = format!("led:{zone_index}={effect}");
                    let _ = Self::device_settings_applied(&emitter, device_key, &setting).await;
                }
                serde_json::to_string_pretty(&report)
                    .map_err(DaemonError::Json)
                    .map_err(Into::into)
            }
            Err(err) => {
                let _ = Self::device_error(&emitter, device_key, &err.to_string()).await;
                Err(err.into())
            }
        }
    }

    /// Reads onboard/host operating mode (`0x8100`) as JSON `OnboardModeReport`.
    async fn get_onboard_mode(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.get_onboard_mode(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Switches operating mode (`"host"` or `"onboard"`) via `0x8100`.
    async fn set_onboard_mode(
        &self,
        device_key: &str,
        mode: &str,
        dry_run: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let res = {
            let mut guard = self.registry.lock().await;
            guard.set_onboard_mode(device_key, mode, dry_run)
        };
        match res {
            Ok(report) => {
                if report.applied && report.verified {
                    let setting = format!("onboard_mode={mode}");
                    let _ = Self::device_settings_applied(&emitter, device_key, &setting).await;
                }
                serde_json::to_string_pretty(&report)
                    .map_err(DaemonError::Json)
                    .map_err(Into::into)
            }
            Err(err) => {
                let _ = Self::device_error(&emitter, device_key, &err.to_string()).await;
                Err(err.into())
            }
        }
    }

    /// Lists onboard profiles (`0x8100`) as JSON `ProfilesListReport`.
    async fn list_profiles(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.list_profiles(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Switches active onboard profile slot (`0x8100`) as JSON `ProfileSetActiveReport`.
    async fn set_active_profile(
        &self,
        device_key: &str,
        slot: u16,
        dry_run: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        let res = {
            let mut guard = self.registry.lock().await;
            guard.set_active_profile(device_key, slot, dry_run)
        };
        match res {
            Ok(report) => {
                if report.applied && report.verified {
                    let setting = format!("profile_slot={slot}");
                    let _ = Self::device_settings_applied(&emitter, device_key, &setting).await;
                }
                serde_json::to_string_pretty(&report)
                    .map_err(DaemonError::Json)
                    .map_err(Into::into)
            }
            Err(err) => {
                let _ = Self::device_error(&emitter, device_key, &err.to_string()).await;
                Err(err.into())
            }
        }
    }

    /// Backs up onboard profile memory (`0x8100`) to default or specified path as JSON `ProfileBackupReport`.
    async fn backup_profiles(
        &self,
        device_key: &str,
        output_path: &str,
    ) -> zbus::fdo::Result<String> {
        let path_opt = if output_path.trim().is_empty() {
            None
        } else {
            Some(std::path::Path::new(output_path))
        };
        let report = {
            let mut guard = self.registry.lock().await;
            guard.backup_profiles(device_key, path_opt)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Reads button capabilities and SVG layout metadata as JSON `ButtonsReport`.
    async fn get_buttons(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.get_buttons(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Reads scroll wheel capabilities and settings as JSON `ScrollConfigReport`.
    async fn get_scroll_config(&self, device_key: &str) -> zbus::fdo::Result<String> {
        let report = {
            let mut guard = self.registry.lock().await;
            guard.get_scroll_config(device_key)
        }?;
        serde_json::to_string_pretty(&report)
            .map_err(DaemonError::Json)
            .map_err(Into::into)
    }

    /// Re-scans endpoints, updates registry, and emits `DevicesChanged` when endpoints change.
    async fn refresh_devices(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<u32> {
        let outcome = {
            let mut guard = self.registry.lock().await;
            guard.sync_devices()
        }?;
        emit_sync_signals(&emitter, &outcome).await;
        Ok(u32::try_from(outcome.total).unwrap_or(u32::MAX))
    }

    /// Invalidates all cached feature tables, re-discovers hardware, and re-applies desired state.
    async fn handle_wake(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<u32> {
        let outcome = {
            let mut guard = self.registry.lock().await;
            guard.invalidate_and_reapply_all()
        }?;
        emit_wake_signals(&emitter, &outcome).await;
        Ok(u32::try_from(outcome.refreshed_devices).unwrap_or(u32::MAX))
    }

    /// Emitted when HID++ endpoints are added or removed.
    #[zbus(signal)]
    async fn devices_changed(
        emitter: &SignalEmitter<'_>,
        added: Vec<String>,
        removed: Vec<String>,
    ) -> zbus::Result<()>;

    /// Emitted after a setting is successfully written and verified on a device.
    #[zbus(signal)]
    async fn device_settings_applied(
        emitter: &SignalEmitter<'_>,
        device_key: &str,
        setting: &str,
    ) -> zbus::Result<()>;

    /// Emitted when an async hotplug, wake, or command operation fails for a device.
    #[zbus(signal)]
    async fn device_error(
        emitter: &SignalEmitter<'_>,
        device_key: &str,
        message: &str,
    ) -> zbus::Result<()>;
}

/// Broadcasts `DevicesChanged` and any auto-applied setting signals from an [`InterfaceRef`].
pub async fn broadcast_sync_outcome<B: DeviceBackend>(
    iface: &InterfaceRef<SqueakDaemonInterface<B>>,
    outcome: &SyncOutcome,
) {
    let emitter = iface.signal_emitter();
    emit_sync_signals(emitter, outcome).await;
}

/// Broadcasts wake re-apply and failure signals from an [`InterfaceRef`].
pub async fn broadcast_wake_outcome<B: DeviceBackend>(
    iface: &InterfaceRef<SqueakDaemonInterface<B>>,
    outcome: &WakeOutcome,
) {
    let emitter = iface.signal_emitter();
    emit_wake_signals(emitter, outcome).await;
}

async fn emit_sync_signals(emitter: &SignalEmitter<'_>, outcome: &SyncOutcome) {
    if !outcome.added.is_empty() || !outcome.removed.is_empty() {
        let _ = SqueakDaemonInterface::<crate::registry::RealHidrawBackend>::devices_changed(
            emitter,
            outcome.added.clone(),
            outcome.removed.clone(),
        )
        .await;
    }
    for (key, setting) in &outcome.auto_applied {
        let _ =
            SqueakDaemonInterface::<crate::registry::RealHidrawBackend>::device_settings_applied(
                emitter, key, setting,
            )
            .await;
    }
}

async fn emit_wake_signals(emitter: &SignalEmitter<'_>, outcome: &WakeOutcome) {
    for (key, setting) in &outcome.reapplied {
        let _ =
            SqueakDaemonInterface::<crate::registry::RealHidrawBackend>::device_settings_applied(
                emitter, key, setting,
            )
            .await;
    }
    for (key, err) in &outcome.failures {
        let _ = SqueakDaemonInterface::<crate::registry::RealHidrawBackend>::device_error(
            emitter, key, err,
        )
        .await;
    }
}

/// Listens to `org.freedesktop.login1.Manager.PrepareForSleep(false)` on the system D-Bus and
/// triggers [`DeviceRegistry::invalidate_and_reapply_all`] whenever the host wakes from suspend.
///
/// Fails gracefully (returning `None`) when no system D-Bus or `systemd-logind` is present.
pub async fn spawn_login1_wake_watcher<B: DeviceBackend>(
    registry: SharedRegistry<B>,
    iface: InterfaceRef<SqueakDaemonInterface<B>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let sys_conn = match Connection::system().await {
        Ok(conn) => conn,
        Err(err) => {
            debug!(%err, "system D-Bus unavailable; login1 suspend/resume listener disabled");
            return None;
        }
    };

    let rule = match build_login1_match_rule() {
        Ok(r) => r,
        Err(err) => {
            warn!(%err, "failed building login1 PrepareForSleep match rule");
            return None;
        }
    };

    let mut stream: MessageStream =
        match MessageStream::for_match_rule(rule, &sys_conn, Some(8)).await {
            Ok(s) => s,
            Err(err) => {
                debug!(%err, "cannot subscribe to login1 PrepareForSleep; wake watcher disabled");
                return None;
            }
        };

    let handle = tokio::spawn(async move {
        let _keep_conn_alive = sys_conn;
        while let Some(msg_res) = stream.next().await {
            let Ok(msg) = msg_res else {
                continue;
            };
            let Ok(going_to_sleep) = msg.body().deserialize::<bool>() else {
                continue;
            };
            if !going_to_sleep {
                info!(
                    "system resumed from sleep (PrepareForSleep=false); re-applying device state"
                );
                let res = {
                    let mut guard = registry.lock().await;
                    guard.invalidate_and_reapply_all()
                };
                match res {
                    Ok(outcome) => broadcast_wake_outcome(&iface, &outcome).await,
                    Err(err) => warn!(%err, "wake refresh failed"),
                }
            }
        }
    });

    Some(handle)
}

fn build_login1_match_rule() -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface("org.freedesktop.login1.Manager")?
        .member("PrepareForSleep")?
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbus_constants_and_rule_builder_are_valid() {
        assert_eq!(DBUS_BUS_NAME, "dev.squeak.Daemon");
        assert_eq!(DBUS_OBJECT_PATH, "/dev/squeak/Daemon");
        assert_eq!(DBUS_INTERFACE_NAME, "dev.squeak.Daemon");
        assert_eq!(DBUS_API_VERSION, 1);

        let rule = build_login1_match_rule().unwrap();
        let rule_str = rule.to_string();
        assert!(rule_str.contains("org.freedesktop.login1.Manager"));
        assert!(rule_str.contains("PrepareForSleep"));
    }
}

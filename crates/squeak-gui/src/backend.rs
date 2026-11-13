//! Background Tokio worker bridging the GTK main thread and `squeakd` over D-Bus.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use futures_lite::StreamExt;
use squeak_core::{
    ButtonsReport, DaemonDeviceSummary, DeviceStatusReport, DpiWriteReport, LedConfigReport,
    LedWriteReport, OnboardModeReport, OnboardModeWriteReport, ProfileBackupReport,
    ProfileSetActiveReport, ProfilesListReport, ReportRateWriteReport, ScrollConfigReport,
};

use crate::dbus_client::{ClientError, DaemonClient};
use crate::state::ErrorViewState;

/// Full snapshot of all capability-gated reports for the currently selected device.
#[derive(Debug, Clone)]
pub struct DeviceSnapshot {
    /// Device summary from `list_devices`.
    pub summary: DaemonDeviceSummary,
    /// M3 status (`0x0003`, `0x0005`, battery, `0x2201`, `0x8060`/`0x8061`).
    pub status: Option<DeviceStatusReport>,
    /// Button layout & `0x1B04`/`0x8110`/`0x8100` capabilities.
    pub buttons: Option<ButtonsReport>,
    /// RGB lighting (`0x8070`), if supported.
    pub led: Option<LedConfigReport>,
    /// Onboard/host mode (`0x8100`), if supported.
    pub onboard_mode: Option<OnboardModeReport>,
    /// Onboard profile slots (`0x8100`), if supported.
    pub profiles: Option<ProfilesListReport>,
    /// Scroll wheel (`0x2110` / `0x2121`), if supported.
    pub scroll: Option<ScrollConfigReport>,
}

/// Commands dispatched from the GTK UI thread to the background Tokio worker.
#[derive(Debug, Clone)]
pub enum UiCommand {
    /// Refresh device list and load the selected (or first) device.
    RefreshAll {
        /// Optional preferred device key to keep selected.
        selected_key: Option<String>,
    },
    /// Load all reports for a specific `device_key`.
    LoadDevice {
        /// Target device key.
        device_key: String,
    },
    /// Apply sensor DPI via `squeakd`.
    SetDpi {
        /// Target device key.
        device_key: String,
        /// Sensor index.
        sensor_index: u8,
        /// Target DPI.
        dpi: u16,
    },
    /// Apply polling rate via `squeakd`.
    SetReportRate {
        /// Target device key.
        device_key: String,
        /// Polling rate in Hz.
        hz: u16,
    },
    /// Apply RGB LED effect via `squeakd`.
    SetLedEffect {
        /// Target device key.
        device_key: String,
        /// Zone index.
        zone_index: u8,
        /// Effect name (`"off"`, `"static"`, `"breathing"`, `"cycle"`).
        effect: String,
        /// Hex color (`"#RRGGBB"` or empty).
        color_hex: String,
        /// Period in ms.
        period_ms: u16,
        /// Brightness percentage (`1..=100`).
        brightness_pct: u8,
    },
    /// Switch onboard/host operating mode via `squeakd`.
    SetOnboardMode {
        /// Target device key.
        device_key: String,
        /// `"host"` or `"onboard"`.
        mode: String,
    },
    /// Switch active onboard profile slot via `squeakd`.
    SetActiveProfile {
        /// Target device key.
        device_key: String,
        /// Slot `1..=5`.
        slot: u16,
    },
    /// Backup onboard profiles to default directory.
    BackupProfiles {
        /// Target device key.
        device_key: String,
    },
    /// Run `pkexec squeak setup-udev` and then refresh devices.
    InstallUdevRule,
}

/// Events delivered from the background Tokio worker to the GTK UI thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Device list updated (empty if no devices found).
    DevicesLoaded {
        /// All discovered devices.
        devices: Vec<DaemonDeviceSummary>,
        /// Snapshot of the selected device, if accessible.
        snapshot: Option<Box<DeviceSnapshot>>,
    },
    /// Full snapshot loaded for a selected device.
    DeviceSnapshotLoaded(Box<DeviceSnapshot>),
    /// D-Bus or daemon connection error.
    Error(ErrorViewState),
    /// DPI write completed.
    DpiApplied(DpiWriteReport),
    /// Report rate write completed.
    ReportRateApplied(ReportRateWriteReport),
    /// LED effect write completed.
    LedApplied(LedWriteReport),
    /// Operating mode switched.
    OnboardModeApplied(OnboardModeWriteReport),
    /// Active profile switched.
    ActiveProfileApplied(ProfileSetActiveReport),
    /// Profile backup written.
    BackupCreated(ProfileBackupReport),
    /// Udev rule installation finished (`Ok` or error string).
    UdevInstallFinished(Result<(), String>),
    /// Async D-Bus signal notification from `squeakd`.
    ToastMessage(String),
}

/// Handle for sending [`UiCommand`]s to the background Tokio runtime.
#[derive(Clone)]
pub struct BackendHandle {
    tx: Sender<UiCommand>,
}

impl BackendHandle {
    /// Spawns the background Tokio thread and returns `(BackendHandle, Receiver<UiEvent>)`.
    ///
    /// # Panics
    ///
    /// Never panics; if Tokio runtime creation fails it reports an [`UiEvent::Error`].
    #[must_use]
    pub fn spawn() -> (Self, Receiver<UiEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel::<UiCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>();

        thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = evt_tx.send(UiEvent::Error(ErrorViewState::Other {
                        detail: format!("Failed to start async runtime: {err}"),
                    }));
                    return;
                }
            };

            rt.block_on(async move {
                run_worker(cmd_rx, evt_tx).await;
            });
        });

        (Self { tx: cmd_tx }, evt_rx)
    }

    /// Sends a command to the background worker without blocking the UI thread.
    pub fn send(&self, cmd: UiCommand) {
        let _ = self.tx.send(cmd);
    }
}

async fn run_worker(cmd_rx: Receiver<UiCommand>, evt_tx: Sender<UiEvent>) {
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<UiCommand>();
    thread::spawn(move || {
        while let Ok(cmd) = cmd_rx.recv() {
            if async_tx.send(cmd).is_err() {
                break;
            }
        }
    });

    let mut client: Option<DaemonClient> = None;

    while let Some(cmd) = async_rx.recv().await {
        if matches!(cmd, UiCommand::InstallUdevRule) {
            let res = run_pkexec_setup_udev().await;
            let _ = evt_tx.send(UiEvent::UdevInstallFinished(res));
            continue;
        }

        let active_client = match ensure_connected(&mut client, &evt_tx).await {
            Some(c) => c,
            None => continue,
        };

        handle_command(&active_client, cmd, &evt_tx).await;
    }
}

async fn ensure_connected(
    slot: &mut Option<DaemonClient>,
    evt_tx: &Sender<UiEvent>,
) -> Option<DaemonClient> {
    if let Some(c) = slot {
        return Some(c.clone());
    }
    match DaemonClient::connect().await {
        Ok(c) => {
            spawn_signal_forwarder(c.clone(), evt_tx.clone());
            *slot = Some(c.clone());
            Some(c)
        }
        Err(err) => {
            let _ = evt_tx.send(UiEvent::Error(classify_client_error(&err)));
            None
        }
    }
}

fn spawn_signal_forwarder(client: DaemonClient, evt_tx: Sender<UiEvent>) {
    tokio::spawn(async move {
        if let Ok(mut stream) = client.proxy().receive_devices_changed().await {
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args()
                    && (!args.added.is_empty() || !args.removed.is_empty())
                {
                    let _ = evt_tx.send(UiEvent::ToastMessage(format!(
                        "Devices updated (+{} / -{})",
                        args.added.len(),
                        args.removed.len()
                    )));
                }
            }
        }
    });
}

async fn handle_command(client: &DaemonClient, cmd: UiCommand, evt_tx: &Sender<UiEvent>) {
    match cmd {
        UiCommand::RefreshAll { selected_key } => {
            let _ = client.refresh_devices().await;
            match client.list_devices().await {
                Ok(devices) => {
                    let target = selected_key
                        .and_then(|k| devices.iter().find(|d| d.device_key == k).cloned())
                        .or_else(|| devices.first().cloned());
                    let snapshot = match target {
                        Some(summary) => Some(Box::new(load_snapshot(client, summary).await)),
                        None => None,
                    };
                    let _ = evt_tx.send(UiEvent::DevicesLoaded { devices, snapshot });
                }
                Err(err) => {
                    let _ = evt_tx.send(UiEvent::Error(classify_client_error(&err)));
                }
            }
        }
        UiCommand::LoadDevice { device_key } => match client.list_devices().await {
            Ok(devices) => {
                if let Some(summary) = devices.into_iter().find(|d| d.device_key == device_key) {
                    let snap = load_snapshot(client, summary).await;
                    let _ = evt_tx.send(UiEvent::DeviceSnapshotLoaded(Box::new(snap)));
                }
            }
            Err(err) => {
                let _ = evt_tx.send(UiEvent::Error(classify_client_error(&err)));
            }
        },
        UiCommand::SetDpi {
            device_key,
            sensor_index,
            dpi,
        } => match client.set_dpi(&device_key, sensor_index, dpi, false).await {
            Ok(rep) => {
                let _ = evt_tx.send(UiEvent::DpiApplied(rep));
            }
            Err(err) => {
                let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
            }
        },
        UiCommand::SetReportRate { device_key, hz } => {
            match client.set_report_rate(&device_key, hz, false).await {
                Ok(rep) => {
                    let _ = evt_tx.send(UiEvent::ReportRateApplied(rep));
                }
                Err(err) => {
                    if client
                        .set_onboard_mode(&device_key, "host", false)
                        .await
                        .is_ok()
                        && let Ok(rep) = client.set_report_rate(&device_key, hz, false).await
                    {
                        let _ = evt_tx.send(UiEvent::ReportRateApplied(rep));
                        return;
                    }
                    let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
                }
            }
        }
        UiCommand::SetLedEffect {
            device_key,
            zone_index,
            effect,
            color_hex,
            period_ms,
            brightness_pct,
        } => {
            match client
                .set_led_effect(
                    &device_key,
                    zone_index,
                    &effect,
                    &color_hex,
                    period_ms,
                    brightness_pct,
                    false,
                )
                .await
            {
                Ok(rep) => {
                    let _ = evt_tx.send(UiEvent::LedApplied(rep));
                }
                Err(err) => {
                    if client
                        .set_onboard_mode(&device_key, "host", false)
                        .await
                        .is_ok()
                        && let Ok(rep) = client
                            .set_led_effect(
                                &device_key,
                                zone_index,
                                &effect,
                                &color_hex,
                                period_ms,
                                brightness_pct,
                                false,
                            )
                            .await
                    {
                        let _ = evt_tx.send(UiEvent::LedApplied(rep));
                        return;
                    }
                    let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
                }
            }
        }
        UiCommand::SetOnboardMode { device_key, mode } => {
            match client.set_onboard_mode(&device_key, &mode, false).await {
                Ok(rep) => {
                    let _ = evt_tx.send(UiEvent::OnboardModeApplied(rep));
                }
                Err(err) => {
                    let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
                }
            }
        }
        UiCommand::SetActiveProfile { device_key, slot } => {
            match client.set_active_profile(&device_key, slot, false).await {
                Ok(rep) => {
                    let _ = evt_tx.send(UiEvent::ActiveProfileApplied(rep));
                }
                Err(err) => {
                    let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
                }
            }
        }
        UiCommand::BackupProfiles { device_key } => {
            match client.backup_profiles(&device_key, "").await {
                Ok(rep) => {
                    let _ = evt_tx.send(UiEvent::BackupCreated(rep));
                }
                Err(err) => {
                    let _ = evt_tx.send(UiEvent::ToastMessage(err.to_string()));
                }
            }
        }
        UiCommand::InstallUdevRule => {}
    }
}

async fn load_snapshot(client: &DaemonClient, summary: DaemonDeviceSummary) -> DeviceSnapshot {
    let key = summary.device_key.clone();
    let status = client.get_device_status(&key).await.ok();
    let buttons = client.get_buttons(&key).await.ok();
    let led = client.get_led_config(&key).await.ok();
    let onboard_mode = client.get_onboard_mode(&key).await.ok();
    let profiles = client.list_profiles(&key).await.ok();
    let scroll = client.get_scroll_config(&key).await.ok();

    DeviceSnapshot {
        summary,
        status,
        buttons,
        led,
        onboard_mode,
        profiles,
        scroll,
    }
}

async fn run_pkexec_setup_udev() -> Result<(), String> {
    let output = tokio::process::Command::new("pkexec")
        .arg("squeak")
        .arg("setup-udev")
        .output()
        .await
        .map_err(|e| format!("Failed to invoke pkexec squeak setup-udev: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("udev installation failed: {stderr}"))
    }
}

fn classify_client_error(err: &ClientError) -> ErrorViewState {
    let msg = err.to_string();
    if msg.contains("ServiceUnknown") || msg.contains("org.freedesktop.DBus.Error") {
        ErrorViewState::DaemonNotRunning { detail: msg }
    } else {
        ErrorViewState::Other { detail: msg }
    }
}

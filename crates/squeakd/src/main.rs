//! Squeak background daemon (`squeakd`) binary entrypoint.
//!
//! Single-owner HID++ `hidraw` manager exposing `dev.squeak.Daemon` over the user
//! session D-Bus, handling netlink `hidraw` hotplug and `login1` suspend/resume wake events.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;
use squeak_core::{DeviceDatabase, load_profiles_from_dir, xdg_profiles_dir};
use squeakd::dbus::{
    DBUS_BUS_NAME, DBUS_OBJECT_PATH, SqueakDaemonInterface, broadcast_sync_outcome,
    spawn_login1_wake_watcher,
};
use squeakd::error::DaemonError;
use squeakd::hotplug::spawn_netlink_hotplug_watcher;
use squeakd::registry::{DeviceRegistry, RealHidrawBackend};
use squeakd::window_watcher::{ActiveWindowProvider, AutoWindowProvider, WindowMatchTracker};
use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

// Dependencies used by squeakd library or binary dependencies that trigger unused-crate-dependencies.
use futures_lite as _;
use hidpp as _;
use rustix as _;
use serde as _;
use serde_json as _;
use thiserror as _;

const HOTPLUG_DEBOUNCE_MS: u64 = 120;

/// Command-line flags for `squeakd`.
#[derive(Debug, Parser)]
#[command(
    name = "squeakd",
    version,
    about = "Single-owner HID++ hidraw daemon exposing dev.squeak.Daemon over D-Bus"
)]
struct DaemonCli {
    /// Emit debug-level diagnostic logs (still strictly excluding serial numbers).
    #[arg(short, long)]
    verbose: bool,

    /// Disable the kernel NETLINK_KOBJECT_UEVENT hidraw hotplug listener.
    #[arg(long)]
    no_hotplug: bool,

    /// Disable systemd-logind PrepareForSleep wake monitoring.
    #[arg(long)]
    no_logind: bool,
}

#[tokio::main]
async fn main() {
    let cli = DaemonCli::parse();
    init_tracing(cli.verbose);

    if let Err(err) = run_daemon(cli).await {
        error!(%err, "squeakd exited with error");
        std::process::exit(1);
    }
}

async fn run_daemon(cli: DaemonCli) -> Result<(), DaemonError> {
    let db = DeviceDatabase::builtin()?;
    let mut registry = DeviceRegistry::new(RealHidrawBackend, db);
    let initial = registry.sync_devices()?;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        devices = initial.total,
        "squeakd initialized device registry"
    );

    let shared = Arc::new(Mutex::new(registry));
    let iface_svc = SqueakDaemonInterface::new(Arc::clone(&shared));

    let conn = zbus::connection::Builder::session()?
        .name(DBUS_BUS_NAME)?
        .serve_at(DBUS_OBJECT_PATH, iface_svc)?
        .build()
        .await?;
    info!(
        bus = DBUS_BUS_NAME,
        path = DBUS_OBJECT_PATH,
        "registered D-Bus service"
    );

    let iface_ref = conn
        .object_server()
        .interface::<_, SqueakDaemonInterface<RealHidrawBackend>>(DBUS_OBJECT_PATH)
        .await?;

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let (uevent_tx, mut uevent_rx) = mpsc::channel(32);

    let hotplug_handle = if cli.no_hotplug {
        None
    } else {
        spawn_netlink_hotplug_watcher(uevent_tx, Arc::clone(&shutdown_flag))
    };

    let wake_handle = if cli.no_logind {
        None
    } else {
        spawn_login1_wake_watcher(Arc::clone(&shared), iface_ref.clone()).await
    };

    let hotplug_task = tokio::spawn({
        let shared = Arc::clone(&shared);
        let iface_ref = iface_ref.clone();
        async move {
            while let Some(ev) = uevent_rx.recv().await {
                debug!(?ev.action, devname = ?ev.devname, "debouncing hidraw hotplug sync");
                sleep(Duration::from_millis(HOTPLUG_DEBOUNCE_MS)).await;
                while uevent_rx.try_recv().is_ok() {}

                let res = {
                    let mut guard = shared.lock().await;
                    guard.sync_devices()
                };
                match res {
                    Ok(outcome) => broadcast_sync_outcome(&iface_ref, &outcome).await,
                    Err(err) => warn!(%err, "hotplug device sync failed"),
                }
            }
        }
    });

    let window_task = tokio::spawn({
        let shared = Arc::clone(&shared);
        let shutdown_flag = Arc::clone(&shutdown_flag);
        async move {
            let provider = AutoWindowProvider;
            let mut tracker = WindowMatchTracker::new();
            let profiles_dir = xdg_profiles_dir();
            while !shutdown_flag.load(Ordering::Relaxed) {
                sleep(Duration::from_millis(750)).await;
                if shutdown_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(profiles) = load_profiles_from_dir(&profiles_dir) else {
                    continue;
                };
                if !profiles.iter().any(|p| !p.match_apps.is_empty()) {
                    continue;
                }
                let Some(win) = provider.active_window() else {
                    continue;
                };

                let mut guard = shared.lock().await;
                let devices = guard.list_devices();
                let mut matched_for_any = None;
                for dev in &devices {
                    let vid = u16::from_str_radix(dev.vendor_id.trim_start_matches("0x"), 16).ok();
                    let pid = u16::from_str_radix(dev.product_id.trim_start_matches("0x"), 16).ok();
                    if let Some(matched) = tracker.evaluate(&win, &profiles, vid, pid) {
                        matched_for_any = Some((dev.device_key.clone(), matched.clone()));
                        break;
                    }
                }
                if let Some((device_key, profile)) = matched_for_any {
                    match guard.apply_software_profile(&device_key, &profile, false) {
                        Ok(_) => info!(
                            device = %device_key,
                            profile = %profile.name,
                            app_id = %win.app_id,
                            "auto-switched software profile for active window"
                        ),
                        Err(err) => warn!(
                            device = %device_key,
                            profile = %profile.name,
                            %err,
                            "failed auto-switching software profile"
                        ),
                    }
                }
            }
        }
    });

    wait_for_shutdown_signal().await;
    info!("shutdown signal received; stopping squeakd cleanly");

    shutdown_flag.store(true, Ordering::Relaxed);
    if let Some(handle) = wake_handle {
        handle.abort();
    }
    hotplug_task.abort();
    window_task.abort();
    if let Some(handle) = hotplug_handle {
        let _ = handle.await;
    }
    drop(conn);
    Ok(())
}

async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    let mut term_stream =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(sig) => Some(sig),
            Err(err) => {
                warn!(%err, "failed to install SIGTERM handler");
                None
            }
        };

    tokio::select! {
        () = ctrl_c => {}
        () = async {
            if let Some(ref mut stream) = term_stream {
                let _ = stream.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }
}

fn init_tracing(verbose: bool) {
    let default_level = if verbose { "debug" } else { "info" };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    #[test]
    fn daemon_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

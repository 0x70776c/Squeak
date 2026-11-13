//! Single-owner device registry, runtime `FeatureTable` cache lifecycle, and wake/hotplug
//! re-apply orchestration for `squeakd`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hidpp::{
    DeviceIndex, FeatureCache, FeatureId, HidDeviceInfo, HidTransport, HidppError, HidppSession,
    HidrawTransport, ProtocolError, TransportError, discover_hidpp_devices,
};
use serde::Serialize;
use squeak_core::{
    ButtonsReport, DaemonDeviceSummary, DefinitionMatchKind, DeviceDatabase, DeviceDump,
    DeviceStatusReport, DpiWriteReport, LedConfigReport, LedWriteReport, OnboardModeReport,
    OnboardModeWriteReport, ProbeStatus, ProfileBackupReport, ProfileSetActiveReport,
    ProfilesListReport, ReportRateWriteReport, ScrollConfigReport, SoftwareProfile,
    execute_dpi_write, execute_host_mode_write, execute_led_write, execute_profile_backup,
    execute_report_rate_write, execute_set_active_profile, parse_led_effect_spec,
    read_buttons_report, read_device_status, read_led_config, read_onboard_mode,
    read_profiles_summary, read_scroll_config,
};
use tracing::{debug, info, warn};

use crate::error::DaemonError;

/// Abstract hardware backend allowing `MockTransport`-driven unit & D-Bus tests without real hardware.
pub trait DeviceBackend: Send + Sync + 'static {
    /// Concrete transport type managed by the daemon.
    type Transport: HidTransport + Send;

    /// Enumerates candidate HID++ device endpoints currently attached to the system.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if sysfs or endpoint enumeration fails.
    fn enumerate_endpoints(&self) -> Result<Vec<HidDeviceInfo>, TransportError>;

    /// Opens a single-owner transport handle to `info`.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] if opening the device fails (e.g. permission denied).
    fn open_transport(&self, info: &HidDeviceInfo) -> Result<Self::Transport, TransportError>;
}

/// Production Linux `hidraw` backend using sysfs discovery and `HidrawTransport`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealHidrawBackend;

impl DeviceBackend for RealHidrawBackend {
    type Transport = HidrawTransport;

    fn enumerate_endpoints(&self) -> Result<Vec<HidDeviceInfo>, TransportError> {
        discover_hidpp_devices()
    }

    fn open_transport(&self, info: &HidDeviceInfo) -> Result<Self::Transport, TransportError> {
        HidrawTransport::open(info.clone())
    }
}

/// Result returned by [`DeviceRegistry::sync_devices`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Newly added `device_key`s.
    pub added: Vec<String>,
    /// Removed `device_key`s.
    pub removed: Vec<String>,
    /// Settings applied automatically upon reconnect (`(device_key, setting_description)`).
    pub auto_applied: Vec<(String, String)>,
    /// Total devices currently tracked.
    pub total: usize,
}

/// Result returned by [`DeviceRegistry::invalidate_and_reapply_all`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WakeOutcome {
    /// Number of accessible devices re-discovered after cache invalidation.
    pub refreshed_devices: usize,
    /// Settings re-applied cleanly (`(device_key, setting_description)`).
    pub reapplied: Vec<(String, String)>,
    /// Non-fatal errors encountered per device during wake re-apply (`(device_key, error)`).
    pub failures: Vec<(String, String)>,
}

/// Summary returned by [`DeviceRegistry::apply_software_profile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SoftwareProfileApplyReport {
    /// Software profile name.
    pub profile_name: String,
    /// Target device key.
    pub device_key: String,
    /// Whether this was a dry-run preview without hardware writes.
    pub dry_run: bool,
    /// Operating mode applied (`"host"` or `"onboard"`), if configured.
    pub onboard_mode_applied: Option<String>,
    /// DPI applied to sensor 0, if configured.
    pub dpi_applied: Option<u16>,
    /// Polling report rate applied in Hz, if configured.
    pub report_rate_hz_applied: Option<u16>,
    /// LED zone indices applied, if configured.
    pub led_zones_applied: Vec<u8>,
}

/// Serial-free signature used to remember volatile host preferences across replug/wake.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DesiredSignature {
    bus: String,
    vendor_id: u16,
    product_id: u16,
    device_index: u8,
}

impl DesiredSignature {
    fn from_info(info: &HidDeviceInfo, device_index: DeviceIndex) -> Self {
        Self {
            bus: info.bus.to_string(),
            vendor_id: info.vendor_id,
            product_id: info.product_id,
            device_index: device_index.as_byte(),
        }
    }
}

/// Cached volatile host settings preserved across suspend/resume and reconnects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DesiredSettings {
    dpi: Option<(u8, u16)>,
    report_rate_hz: Option<u16>,
}

struct ManagedDevice<T: HidTransport> {
    device_key: String,
    info: HidDeviceInfo,
    device_index: DeviceIndex,
    cache: FeatureCache,
    session: Option<HidppSession<T>>,
    probe_status: ProbeStatus,
    hint: Option<String>,
}

/// Single-owner registry managing connected HID++ endpoints, cached feature tables, and
/// reconnect/wake re-apply state.
pub struct DeviceRegistry<B: DeviceBackend> {
    backend: B,
    db: DeviceDatabase,
    devices: BTreeMap<String, ManagedDevice<B::Transport>>,
    desired_by_sig: BTreeMap<DesiredSignature, DesiredSettings>,
}

impl<B: DeviceBackend> DeviceRegistry<B> {
    /// Creates a new empty registry backed by `backend` and `db`.
    #[must_use]
    pub fn new(backend: B, db: DeviceDatabase) -> Self {
        Self {
            backend,
            db,
            devices: BTreeMap::new(),
            desired_by_sig: BTreeMap::new(),
        }
    }

    /// Synchronizes the registry against [`DeviceBackend::enumerate_endpoints`], probing newly
    /// attached endpoints, pruning unplugged ones, and re-applying cached desired settings on
    /// reconnect.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Transport`] if endpoint enumeration itself fails.
    pub fn sync_devices(&mut self) -> Result<SyncOutcome, DaemonError> {
        let endpoints = self.backend.enumerate_endpoints()?;
        let device_index = DeviceIndex::DIRECT;

        let mut desired_keys = BTreeSet::new();
        let mut endpoint_by_key = BTreeMap::new();
        for info in endpoints {
            let key = build_device_key(&info, device_index);
            desired_keys.insert(key.clone());
            endpoint_by_key.insert(key, info);
        }

        let mut outcome = SyncOutcome::default();

        // Prune unplugged keys cleanly while retaining `desired_by_sig` for reconnect re-apply.
        let existing_keys: Vec<String> = self.devices.keys().cloned().collect();
        for key in existing_keys {
            if !desired_keys.contains(&key) {
                self.devices.remove(&key);
                outcome.removed.push(key);
            }
        }

        // Probe newly added or previously non-accessible endpoints.
        for (key, info) in endpoint_by_key {
            let needs_probe = match self.devices.get(&key) {
                None => true,
                Some(dev) => dev.cache.get(dev.device_index).is_none() || dev.session.is_none(),
            };
            if !needs_probe {
                continue;
            }

            let is_new = !self.devices.contains_key(&key);
            let managed = probe_managed_device(&self.backend, key.clone(), info, device_index);
            if is_new {
                info!(
                    device_key = %managed.device_key,
                    vid = format_args!("0x{:04x}", managed.info.vendor_id),
                    pid = format_args!("0x{:04x}", managed.info.product_id),
                    status = ?managed.probe_status,
                    features = managed
                        .cache
                        .get(managed.device_index)
                        .map_or(0, |t| t.entries().len()),
                    "registered hidraw endpoint"
                );
                outcome.added.push(key.clone());
            }
            self.devices.insert(key.clone(), managed);

            if is_new {
                let applied = self.reapply_for_key(&key);
                for setting in applied.0 {
                    outcome.auto_applied.push((key.clone(), setting));
                }
            }
        }

        outcome.total = self.devices.len();
        Ok(outcome)
    }

    /// Invalidates all cached feature tables and open sessions (e.g. on wake from suspend),
    /// re-discovers every connected endpoint, and re-applies stored desired settings.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::Transport`] if endpoint enumeration fails.
    pub fn invalidate_and_reapply_all(&mut self) -> Result<WakeOutcome, DaemonError> {
        info!("invalidating all cached feature tables and sessions for wake/refresh");
        for dev in self.devices.values_mut() {
            dev.session = None;
            dev.cache.invalidate_all();
        }
        let _sync = self.sync_devices()?;

        let mut outcome = WakeOutcome::default();
        let keys: Vec<String> = self.devices.keys().cloned().collect();
        for key in keys {
            let Some(dev) = self.devices.get(&key) else {
                continue;
            };
            if dev.probe_status == ProbeStatus::Accessible
                && dev.cache.get(dev.device_index).is_some()
            {
                outcome.refreshed_devices += 1;
            }
            let (applied, errors) = self.reapply_for_key(&key);
            for setting in applied {
                outcome.reapplied.push((key.clone(), setting));
            }
            for err in errors {
                outcome.failures.push((key.clone(), err));
            }
        }

        Ok(outcome)
    }

    /// Lists all tracked devices as sanitized [`DaemonDeviceSummary`] objects (no serial numbers).
    #[must_use]
    pub fn list_devices(&self) -> Vec<DaemonDeviceSummary> {
        self.devices
            .values()
            .map(|dev| {
                let resolved = self.db.resolve(dev.info.vendor_id, dev.info.product_id);
                let definition_match = if resolved.is_known() {
                    DefinitionMatchKind::Known
                } else {
                    DefinitionMatchKind::Generic
                };
                let sig = DesiredSignature::from_info(&dev.info, dev.device_index);
                let desired = self.desired_by_sig.get(&sig).copied().unwrap_or_default();
                let (protocol_version, feature_count) = match dev.cache.get(dev.device_index) {
                    Some(t) => (Some(t.protocol_version().to_string()), t.entries().len()),
                    None => (None, 0),
                };

                DaemonDeviceSummary {
                    device_key: dev.device_key.clone(),
                    path: dev.info.path.display().to_string(),
                    bus: dev.info.bus.to_string(),
                    vendor_id: format!("0x{:04x}", dev.info.vendor_id),
                    product_id: format!("0x{:04x}", dev.info.product_id),
                    hid_name: dev.info.name.clone(),
                    device_index: format!("0x{:02X}", dev.device_index.as_byte()),
                    definition_match,
                    matched_name: resolved.display_name(dev.info.name.as_deref()),
                    kind: resolved.kind(),
                    protocol_version,
                    probe_status: dev.probe_status.clone(),
                    feature_count,
                    desired_dpi: desired.dpi.map(|(_, d)| d),
                    desired_report_rate_hz: desired.report_rate_hz,
                    hint: dev.hint.clone(),
                }
            })
            .collect()
    }

    /// Reads M3 device status for `selector` through the single-owner daemon session.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `selector` is unknown/ambiguous or probing fails.
    pub fn get_status(&mut self, selector: &str) -> Result<DeviceStatusReport, DaemonError> {
        let key = self.resolve_selector(selector)?;
        self.ensure_ready(&key)?;
        let dev = self
            .devices
            .get_mut(&key)
            .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
        let table = dev
            .cache
            .get(dev.device_index)
            .ok_or_else(|| {
                DaemonError::InvalidArgument(format!("device {key} feature table not ready"))
            })?
            .clone();
        let session = dev.session.as_mut().ok_or_else(|| {
            DaemonError::InvalidArgument(format!("device {key} session not open"))
        })?;
        Ok(read_device_status(
            session,
            &dev.info,
            &table,
            &self.db,
            env!("CARGO_PKG_VERSION"),
        ))
    }

    /// Produces a sanitized [`DeviceDump`] for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `selector` cannot be resolved or device discovery failed.
    pub fn dump_device(&mut self, selector: &str) -> Result<DeviceDump, DaemonError> {
        let key = self.resolve_selector(selector)?;
        self.ensure_ready(&key)?;
        let dev = self
            .devices
            .get(&key)
            .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
        let table = dev.cache.get(dev.device_index).ok_or_else(|| {
            DaemonError::InvalidArgument(format!("device {key} feature table not ready"))
        })?;
        let resolved = self.db.resolve(dev.info.vendor_id, dev.info.product_id);
        Ok(DeviceDump::from_discovery(
            env!("CARGO_PKG_VERSION"),
            &dev.info,
            table,
            resolved,
        ))
    }

    /// Validates, previews (`dry_run = true`), or applies + verifies (`dry_run = false`) a DPI
    /// change on `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] on selector, capability, transport, or readback errors.
    pub fn set_dpi(
        &mut self,
        selector: &str,
        sensor_index: u8,
        dpi: u16,
        dry_run: bool,
    ) -> Result<DpiWriteReport, DaemonError> {
        let key = self.resolve_selector(selector)?;
        self.ensure_ready(&key)?;

        let (sig, result) = {
            let dev = self
                .devices
                .get_mut(&key)
                .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
            let sig = DesiredSignature::from_info(&dev.info, dev.device_index);
            let table = dev
                .cache
                .get(dev.device_index)
                .ok_or_else(|| {
                    DaemonError::InvalidArgument(format!("device {key} feature table not ready"))
                })?
                .clone();
            let session = dev.session.as_mut().ok_or_else(|| {
                DaemonError::InvalidArgument(format!("device {key} session not open"))
            })?;
            let res = execute_dpi_write(session, &table, sensor_index, dpi, !dry_run);
            if matches!(
                res,
                Err(HidppError::Transport(
                    TransportError::Io { .. } | TransportError::Disconnected
                ))
            ) {
                dev.cache.invalidate_all();
                dev.session = None;
            }
            (sig, res)
        };

        let report = result?;
        if report.applied && report.verified {
            self.desired_by_sig.entry(sig).or_default().dpi = Some((sensor_index, dpi));
        }
        Ok(report)
    }

    /// Validates, previews (`dry_run = true`), or applies + verifies (`dry_run = false`) a
    /// polling report-rate change on `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] on selector, capability, transport, or readback errors.
    pub fn set_report_rate(
        &mut self,
        selector: &str,
        hz: u16,
        dry_run: bool,
    ) -> Result<ReportRateWriteReport, DaemonError> {
        let key = self.resolve_selector(selector)?;
        self.ensure_ready(&key)?;

        let (sig, result) = {
            let dev = self
                .devices
                .get_mut(&key)
                .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
            let sig = DesiredSignature::from_info(&dev.info, dev.device_index);
            let table = dev
                .cache
                .get(dev.device_index)
                .ok_or_else(|| {
                    DaemonError::InvalidArgument(format!("device {key} feature table not ready"))
                })?
                .clone();
            let session = dev.session.as_mut().ok_or_else(|| {
                DaemonError::InvalidArgument(format!("device {key} session not open"))
            })?;
            let res = execute_report_rate_write(session, &table, hz, !dry_run);
            if matches!(
                res,
                Err(HidppError::Transport(
                    TransportError::Io { .. } | TransportError::Disconnected
                ))
            ) {
                dev.cache.invalidate_all();
                dev.session = None;
            }
            (sig, res)
        };

        let report = result?;
        if report.applied && report.verified {
            self.desired_by_sig.entry(sig).or_default().report_rate_hz = Some(hz);
        }
        Ok(report)
    }

    fn with_ready_session<R>(
        &mut self,
        selector: &str,
        op: impl FnOnce(
            &mut HidppSession<B::Transport>,
            &hidpp::FeatureTable,
            &HidDeviceInfo,
            &DeviceDatabase,
        ) -> Result<R, HidppError>,
    ) -> Result<R, DaemonError> {
        let key = self.resolve_selector(selector)?;
        self.ensure_ready(&key)?;
        let dev = self
            .devices
            .get_mut(&key)
            .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
        let table = dev
            .cache
            .get(dev.device_index)
            .ok_or_else(|| {
                DaemonError::InvalidArgument(format!("device {key} feature table not ready"))
            })?
            .clone();
        let session = dev.session.as_mut().ok_or_else(|| {
            DaemonError::InvalidArgument(format!("device {key} session not open"))
        })?;
        let res = op(session, &table, &dev.info, &self.db);
        if matches!(
            res,
            Err(HidppError::Transport(
                TransportError::Io { .. } | TransportError::Disconnected
            ))
        ) {
            dev.cache.invalidate_all();
            dev.session = None;
        }
        res.map_err(DaemonError::from)
    }

    /// Reads RGB LED zone configuration (`0x8070`) for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `0x8070` is unsupported or device communication fails.
    pub fn get_led_config(&mut self, selector: &str) -> Result<LedConfigReport, DaemonError> {
        self.with_ready_session(selector, |session, table, _, _| {
            read_led_config(session, table)?.ok_or(HidppError::Protocol(
                ProtocolError::FeatureNotSupported {
                    feature_id: FeatureId::COLOR_LED_EFFECTS,
                },
            ))
        })
    }

    /// Previews or applies an RGB LED effect (`0x8070`) on `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if parameters are invalid or `0x8070` fails.
    #[allow(clippy::too_many_arguments)]
    pub fn set_led_effect(
        &mut self,
        selector: &str,
        zone_index: u8,
        effect: &str,
        color_hex: Option<&str>,
        period_ms: Option<u16>,
        brightness_pct: Option<u8>,
        dry_run: bool,
    ) -> Result<LedWriteReport, DaemonError> {
        let parsed = parse_led_effect_spec(effect, color_hex, period_ms, brightness_pct)
            .map_err(|e| DaemonError::InvalidArgument(e.to_string()))?;
        self.with_ready_session(selector, |session, table, _, _| {
            execute_led_write(session, table, zone_index, parsed, !dry_run)
        })
    }

    /// Reads onboard/host operating mode (`0x8100`) for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `0x8100` is unsupported or device communication fails.
    pub fn get_onboard_mode(&mut self, selector: &str) -> Result<OnboardModeReport, DaemonError> {
        self.with_ready_session(selector, |session, table, _, _| {
            read_onboard_mode(session, table)?.ok_or(HidppError::Protocol(
                ProtocolError::FeatureNotSupported {
                    feature_id: FeatureId::ONBOARD_PROFILES,
                },
            ))
        })
    }

    /// Switches onboard/host operating mode (`0x8100`) on `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `mode` is invalid or `0x8100` fails.
    pub fn set_onboard_mode(
        &mut self,
        selector: &str,
        mode: &str,
        dry_run: bool,
    ) -> Result<OnboardModeWriteReport, DaemonError> {
        let target_host_mode = match mode.trim().to_ascii_lowercase().as_str() {
            "host" | "software" => true,
            "onboard" | "memory" | "hardware" => false,
            other => {
                return Err(DaemonError::InvalidArgument(format!(
                    "invalid onboard mode '{other}' (expected 'host' or 'onboard')"
                )));
            }
        };
        self.with_ready_session(selector, |session, table, _, _| {
            execute_host_mode_write(session, table, target_host_mode, !dry_run)
        })
    }

    /// Lists onboard profiles (`0x8100`) for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `0x8100` is unsupported or reading sectors fails.
    pub fn list_profiles(&mut self, selector: &str) -> Result<ProfilesListReport, DaemonError> {
        self.with_ready_session(selector, |session, table, _, _| {
            read_profiles_summary(session, table)
        })
    }

    /// Switches active onboard profile slot (`0x8100`) on `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if slot is out of range or readback fails.
    pub fn set_active_profile(
        &mut self,
        selector: &str,
        slot: u16,
        dry_run: bool,
    ) -> Result<ProfileSetActiveReport, DaemonError> {
        self.with_ready_session(selector, |session, table, _, _| {
            execute_set_active_profile(session, table, slot, !dry_run)
        })
    }

    /// Backs up onboard profile memory sectors (`0x8100`) for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if reading sectors or writing the backup file fails.
    pub fn backup_profiles(
        &mut self,
        selector: &str,
        output_path: Option<&Path>,
    ) -> Result<ProfileBackupReport, DaemonError> {
        self.with_ready_session(selector, |session, table, info, _| {
            execute_profile_backup(session, info, table, output_path)
        })
    }

    /// Reads button capabilities and SVG layout metadata for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `selector` cannot be resolved or device discovery failed.
    pub fn get_buttons(&mut self, selector: &str) -> Result<ButtonsReport, DaemonError> {
        self.with_ready_session(selector, |session, table, info, db| {
            Ok(read_buttons_report(session, info, table, db))
        })
    }

    /// Reads scroll wheel (`0x2110` / `0x2121`) capabilities and status for `selector`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `selector` cannot be resolved or device discovery failed.
    pub fn get_scroll_config(&mut self, selector: &str) -> Result<ScrollConfigReport, DaemonError> {
        self.with_ready_session(selector, |session, table, _, _| {
            Ok(read_scroll_config(session, table))
        })
    }

    /// Applies (or dry-runs) a [`SoftwareProfile`] to `selector`.
    ///
    /// Applies `onboard_mode`, `dpi` (sensor 0), `report_rate_hz`, and per-zone `led`
    /// effects in sequence using the single-owner device session.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] if `selector` cannot be resolved or any device write fails.
    pub fn apply_software_profile(
        &mut self,
        selector: &str,
        profile: &SoftwareProfile,
        dry_run: bool,
    ) -> Result<SoftwareProfileApplyReport, DaemonError> {
        let key = self.resolve_selector(selector)?;

        let onboard_mode_applied = if let Some(ref mode) = profile.onboard_mode {
            self.set_onboard_mode(&key, mode, dry_run)?;
            Some(mode.clone())
        } else {
            None
        };

        let dpi_applied = if let Some(dpi) = profile.dpi {
            self.set_dpi(&key, 0, dpi, dry_run)?;
            Some(dpi)
        } else {
            None
        };

        let report_rate_hz_applied = if let Some(hz) = profile.report_rate_hz {
            self.set_report_rate(&key, hz, dry_run)?;
            Some(hz)
        } else {
            None
        };

        let mut led_zones_applied = Vec::with_capacity(profile.led.len());
        for zone in &profile.led {
            self.set_led_effect(
                &key,
                zone.zone_index,
                &zone.effect,
                zone.color_hex.as_deref(),
                zone.period_ms,
                zone.brightness_pct,
                dry_run,
            )?;
            led_zones_applied.push(zone.zone_index);
        }

        Ok(SoftwareProfileApplyReport {
            profile_name: profile.name.clone(),
            device_key: key,
            dry_run,
            onboard_mode_applied,
            dpi_applied,
            report_rate_hz_applied,
            led_zones_applied,
        })
    }

    fn resolve_selector(&self, selector: &str) -> Result<String, DaemonError> {
        let clean = selector.trim();
        if clean.is_empty() {
            return match self.devices.len() {
                0 => Err(DaemonError::DeviceNotFound(selector.to_owned())),
                1 => {
                    let only = self
                        .devices
                        .keys()
                        .next()
                        .ok_or_else(|| DaemonError::DeviceNotFound(selector.to_owned()))?;
                    Ok(only.clone())
                }
                _ => Err(DaemonError::AmbiguousDevice(selector.to_owned())),
            };
        }

        if self.devices.contains_key(clean) {
            return Ok(clean.to_owned());
        }

        let vid_pid = parse_vid_pid(clean);
        let mut matches = Vec::new();
        for (key, dev) in &self.devices {
            let basename = dev.info.path.file_name().and_then(|n| n.to_str());
            let path_matches = dev.info.path.to_str() == Some(clean) || basename == Some(clean);
            let vp_matches = vid_pid == Some((dev.info.vendor_id, dev.info.product_id));
            if path_matches || vp_matches {
                matches.push(key.clone());
            }
        }

        match matches.len() {
            0 => Err(DaemonError::DeviceNotFound(clean.to_owned())),
            1 => Ok(matches.remove(0)),
            _ => Err(DaemonError::AmbiguousDevice(clean.to_owned())),
        }
    }

    fn ensure_ready(&mut self, key: &str) -> Result<(), DaemonError> {
        let needs_reprobe = match self.devices.get(key) {
            Some(dev) => dev.cache.get(dev.device_index).is_none() || dev.session.is_none(),
            None => return Err(DaemonError::DeviceNotFound(key.to_owned())),
        };
        if !needs_reprobe {
            return Ok(());
        }

        let Some(existing) = self.devices.get(key) else {
            return Err(DaemonError::DeviceNotFound(key.to_owned()));
        };
        let refreshed = probe_managed_device(
            &self.backend,
            key.to_owned(),
            existing.info.clone(),
            existing.device_index,
        );
        let status = refreshed.probe_status.clone();
        let hint = refreshed.hint.clone();
        self.devices.insert(key.to_owned(), refreshed);

        match status {
            ProbeStatus::Accessible => Ok(()),
            ProbeStatus::PermissionDenied => {
                Err(DaemonError::Transport(TransportError::PermissionDenied {
                    path: self
                        .devices
                        .get(key)
                        .map_or_else(Default::default, |d| d.info.path.clone()),
                }))
            }
            ProbeStatus::LegacyHidpp10 => {
                Err(DaemonError::InvalidArgument(hint.unwrap_or_else(|| {
                    "Legacy HID++ 1.0 device is not supported.".to_owned()
                })))
            }
            ProbeStatus::TimeoutOrAsleep => Err(DaemonError::Transport(TransportError::Timeout)),
            ProbeStatus::Error { message } => {
                Err(DaemonError::InvalidArgument(hint.unwrap_or(message)))
            }
            other => {
                Err(DaemonError::InvalidArgument(hint.unwrap_or_else(|| {
                    format!("device not accessible: {other:?}")
                })))
            }
        }
    }

    fn reapply_for_key(&mut self, key: &str) -> (Vec<String>, Vec<String>) {
        let mut applied = Vec::new();
        let mut errors = Vec::new();

        let Some((sig, ready)) = self.devices.get(key).map(|dev| {
            (
                DesiredSignature::from_info(&dev.info, dev.device_index),
                dev.cache.get(dev.device_index).is_some() && dev.session.is_some(),
            )
        }) else {
            return (applied, errors);
        };
        if !ready {
            return (applied, errors);
        }

        let Some(desired) = self.desired_by_sig.get(&sig).copied() else {
            return (applied, errors);
        };

        if let Some((sensor, dpi)) = desired.dpi {
            match self.set_dpi(key, sensor, dpi, false) {
                Ok(_) => {
                    let msg = format!("dpi:{sensor}={dpi}");
                    debug!(device_key = %key, setting = %msg, "re-applied DPI setting");
                    applied.push(msg);
                }
                Err(err) => {
                    warn!(device_key = %key, %err, "failed re-applying DPI");
                    errors.push(format!("dpi: {err}"));
                }
            }
        }

        if let Some(hz) = desired.report_rate_hz {
            match self.set_report_rate(key, hz, false) {
                Ok(_) => {
                    let msg = format!("report_rate={hz}Hz");
                    debug!(device_key = %key, setting = %msg, "re-applied report rate setting");
                    applied.push(msg);
                }
                Err(err) => {
                    warn!(device_key = %key, %err, "failed re-applying report rate");
                    errors.push(format!("report_rate: {err}"));
                }
            }
        }

        (applied, errors)
    }
}

fn probe_managed_device<B: DeviceBackend>(
    backend: &B,
    device_key: String,
    info: HidDeviceInfo,
    device_index: DeviceIndex,
) -> ManagedDevice<B::Transport> {
    let mut cache = FeatureCache::new();
    match backend.open_transport(&info) {
        Ok(transport) => {
            let mut session = HidppSession::new(transport);
            match cache.get_or_discover(&mut session, device_index) {
                Ok(_table) => ManagedDevice {
                    device_key,
                    info,
                    device_index,
                    cache,
                    session: Some(session),
                    probe_status: ProbeStatus::Accessible,
                    hint: None,
                },
                Err(HidppError::Protocol(ProtocolError::LegacyDeviceNotSupported { .. })) => {
                    ManagedDevice {
                        device_key,
                        info,
                        device_index,
                        cache,
                        session: None,
                        probe_status: ProbeStatus::LegacyHidpp10,
                        hint: Some("Legacy HID++ 1.0 hardware is out of scope.".to_owned()),
                    }
                }
                Err(HidppError::Transport(TransportError::Timeout))
                | Err(HidppError::Protocol(ProtocolError::ReceiverDeviceOffline { .. })) => {
                    ManagedDevice {
                        device_key,
                        info,
                        device_index,
                        cache,
                        session: None,
                        probe_status: ProbeStatus::TimeoutOrAsleep,
                        hint: Some(
                            "Device timed out or is asleep; will retry on wake/refresh.".to_owned(),
                        ),
                    }
                }
                Err(err) => ManagedDevice {
                    device_key,
                    info,
                    device_index,
                    cache,
                    session: None,
                    probe_status: ProbeStatus::Error {
                        message: err.to_string(),
                    },
                    hint: None,
                },
            }
        }
        Err(TransportError::PermissionDenied { .. }) => ManagedDevice {
            device_key,
            info,
            device_index,
            cache,
            session: None,
            probe_status: ProbeStatus::PermissionDenied,
            hint: Some("Permission denied; run `squeak setup-udev`.".to_owned()),
        },
        Err(err) => ManagedDevice {
            device_key,
            info,
            device_index,
            cache,
            session: None,
            probe_status: ProbeStatus::Error {
                message: err.to_string(),
            },
            hint: None,
        },
    }
}

fn build_device_key(info: &HidDeviceInfo, device_index: DeviceIndex) -> String {
    let node = info
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("hidraw");
    format!(
        "{node}-{:04x}-{:04x}-{:02x}",
        info.vendor_id,
        info.product_id,
        device_index.as_byte()
    )
}

fn parse_vid_pid(spec: &str) -> Option<(u16, u16)> {
    let (v, p) = spec.split_once(':')?;
    let parse_part = |part: &str| -> Option<u16> {
        let t = part.trim();
        let clean = t
            .strip_prefix("0x")
            .or_else(|| t.strip_prefix("0X"))
            .unwrap_or(t);
        u16::from_str_radix(clean, 16).ok()
    };
    Some((parse_part(v)?, parse_part(p)?))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use hidpp::{FeatureIndex, FunctionId, HidppFrame, MockTransport, ReportId, SwId};

    use super::*;

    #[derive(Clone, Default)]
    pub struct MockBackend {
        pub endpoints: Arc<Mutex<Vec<HidDeviceInfo>>>,
        pub queued_transports: Arc<Mutex<BTreeMap<PathBuf, VecDeque<MockTransport>>>>,
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn set_endpoints(&self, endpoints: Vec<HidDeviceInfo>) {
            *self.endpoints.lock().unwrap() = endpoints;
        }

        pub fn push_transport(&self, path: PathBuf, mock: MockTransport) {
            self.queued_transports
                .lock()
                .unwrap()
                .entry(path)
                .or_default()
                .push_back(mock);
        }
    }

    impl DeviceBackend for MockBackend {
        type Transport = MockTransport;

        fn enumerate_endpoints(&self) -> Result<Vec<HidDeviceInfo>, TransportError> {
            Ok(self.endpoints.lock().unwrap().clone())
        }

        fn open_transport(&self, info: &HidDeviceInfo) -> Result<Self::Transport, TransportError> {
            let mut map = self.queued_transports.lock().unwrap();
            let queue = map
                .get_mut(&info.path)
                .ok_or_else(|| TransportError::DeviceNotFound {
                    path: info.path.clone(),
                })?;
            queue
                .pop_front()
                .ok_or_else(|| TransportError::DeviceNotFound {
                    path: info.path.clone(),
                })
        }
    }

    pub fn fn_id(id: u8) -> FunctionId {
        FunctionId::new(id).unwrap()
    }

    pub fn push_g502_discovery(mock: &mut MockTransport) {
        // ping -> 4.2
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x02, 0x53],
            )
            .unwrap(),
        );
        // getFeature(0x0001) -> idx 1, v0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00, 0x00],
            )
            .unwrap(),
        );
        // getCount() -> 2 features (0x0001 v0, 0x2201 v1)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[2, 0, 0],
            )
            .unwrap(),
        );
        // idx 1: 0x0001 v0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x00, 0x00],
            )
            .unwrap(),
        );
        // idx 2: 0x2201 v1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(1),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x22, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
    }

    pub fn push_dpi_set_sequence(mock: &mut MockTransport, current_dpi: u16, target_dpi: u16) {
        let [c_hi, c_lo] = current_dpi.to_be_bytes();
        let [t_hi, t_lo] = target_dpi.to_be_bytes();
        // Pre-check: getSensorCount, getSensorDpiList, getSensorDpi
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, c_hi, c_lo, 0x03, 0xE8],
            )
            .unwrap(),
        );
        // set_sensor_dpi_verified: getSensorCount, getSensorDpiList, set echo, readback
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(3),
                SwId::DEFAULT_REQUEST,
                &[0, t_hi, t_lo],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, t_hi, t_lo, 0x03, 0xE8],
            )
            .unwrap(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hidpp::{FeatureIndex, HidppFrame, MockTransport, ReportId, SwId};

    use super::test_support::{MockBackend, fn_id, push_dpi_set_sequence, push_g502_discovery};
    use super::*;

    #[test]
    fn registry_discovers_applies_invalidates_on_wake_and_reapplies_on_hotplug() {
        let backend = MockBackend::new();
        let mut dev6 = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        dev6.path = PathBuf::from("/dev/hidraw6");
        backend.set_endpoints(vec![dev6.clone()]);

        // 1. Initial connection + set DPI 1200 -> 1600
        let mut m1 = MockTransport::new(dev6.clone());
        push_g502_discovery(&mut m1);
        push_dpi_set_sequence(&mut m1, 1200, 1600);
        backend.push_transport(dev6.path.clone(), m1);

        let db = DeviceDatabase::builtin().unwrap();
        let mut registry = DeviceRegistry::new(backend.clone(), db);
        let sync1 = registry.sync_devices().unwrap();
        assert_eq!(sync1.added, vec!["hidraw6-046d-c08b-ff"]);
        assert_eq!(sync1.total, 1);

        let list1 = registry.list_devices();
        assert_eq!(list1[0].feature_count, 3);
        assert_eq!(list1[0].desired_dpi, None);

        // Apply 1600 DPI
        let dpi_res = registry.set_dpi("046d:c08b", 0, 1600, false).unwrap();
        assert!(dpi_res.applied);
        assert!(dpi_res.verified);
        assert_eq!(registry.list_devices()[0].desired_dpi, Some(1600));

        // 2. Wake from suspend -> invalidates cached table & re-opens + re-applies 1600 DPI!
        let mut m_wake = MockTransport::new(dev6.clone());
        push_g502_discovery(&mut m_wake);
        // Note: on wake the mouse might have reset to default 1000 DPI; daemon restores 1600!
        push_dpi_set_sequence(&mut m_wake, 1000, 1600);
        backend.push_transport(dev6.path.clone(), m_wake);

        let wake = registry.invalidate_and_reapply_all().unwrap();
        assert_eq!(wake.refreshed_devices, 1);
        assert_eq!(
            wake.reapplied,
            vec![("hidraw6-046d-c08b-ff".to_owned(), "dpi:0=1600".to_owned())]
        );
        assert!(wake.failures.is_empty());

        // 3. Hotplug unplug + replug on a different node (/dev/hidraw7) preserves desired_dpi & re-applies!
        backend.set_endpoints(vec![]);
        let sync_unplug = registry.sync_devices().unwrap();
        assert_eq!(sync_unplug.removed, vec!["hidraw6-046d-c08b-ff"]);
        assert_eq!(sync_unplug.total, 0);

        let mut dev7 = dev6;
        dev7.path = PathBuf::from("/dev/hidraw7");
        backend.set_endpoints(vec![dev7.clone()]);
        let mut m_replug = MockTransport::new(dev7.clone());
        push_g502_discovery(&mut m_replug);
        push_dpi_set_sequence(&mut m_replug, 1000, 1600);
        // Also queue a status read after replug (0x2201: count, list, get -> 1600)
        m_replug.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[1],
            )
            .unwrap(),
        );
        m_replug.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0, 0x00, 0x64, 0xE0, 0x32, 0x64, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        m_replug.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(2),
                fn_id(2),
                SwId::DEFAULT_REQUEST,
                &[0, 0x06, 0x40, 0x03, 0xE8],
            )
            .unwrap(),
        );
        backend.push_transport(dev7.path, m_replug);

        let sync_replug = registry.sync_devices().unwrap();
        assert_eq!(sync_replug.added, vec!["hidraw7-046d-c08b-ff"]);
        assert_eq!(
            sync_replug.auto_applied,
            vec![("hidraw7-046d-c08b-ff".to_owned(), "dpi:0=1600".to_owned())]
        );

        let status = registry.get_status("hidraw7").unwrap();
        assert_eq!(status.dpi.unwrap().sensors[0].current_dpi, 1600);
    }
}

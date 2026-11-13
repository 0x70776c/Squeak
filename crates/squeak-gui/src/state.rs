//! UI view-model state, capability gating, undo stack, and error classification.

use squeak_core::{
    ButtonsReport, DaemonDeviceSummary, DeviceStatusReport, LedConfigReport, ProbeStatus,
    ProfilesListReport, ScrollConfigReport,
};

/// Last reversible setting change recorded for the "Undo" action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoAction {
    /// Revert sensor DPI to `previous_dpi`.
    Dpi {
        /// Target device key.
        device_key: String,
        /// Sensor index.
        sensor_index: u8,
        /// Previous DPI before the change.
        previous_dpi: u16,
    },
    /// Revert polling rate to `previous_hz`.
    ReportRate {
        /// Target device key.
        device_key: String,
        /// Previous polling rate in Hz.
        previous_hz: u16,
    },
    /// Revert operating mode (`"host"` / `"onboard"`).
    OnboardMode {
        /// Target device key.
        device_key: String,
        /// Previous operating mode string.
        previous_mode: String,
    },
    /// Revert active onboard profile slot.
    ActiveProfile {
        /// Target device key.
        device_key: String,
        /// Previous active slot (`1..=5`).
        previous_slot: u16,
    },
}

/// Single-slot undo history for the most recent setting change.
#[derive(Debug, Default)]
pub struct UndoStack {
    last: Option<UndoAction>,
}

impl UndoStack {
    /// Records the most recent reversible action.
    pub fn push(&mut self, action: UndoAction) {
        self.last = Some(action);
    }

    /// Pops the last recorded action for execution.
    pub fn pop(&mut self) -> Option<UndoAction> {
        self.last.take()
    }

    /// Returns `true` when an undoable action is available.
    #[must_use]
    pub const fn can_undo(&self) -> bool {
        self.last.is_some()
    }
}

/// Actionable error classification rendered via `AdwStatusPage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorViewState {
    /// `/dev/hidraw*` lacks `uaccess` permissions -> offer "Install udev rule" (`pkexec squeak setup-udev`).
    NoHidrawPermission {
        /// Human-readable device label.
        device_name: String,
    },
    /// `squeakd` session D-Bus service is unreachable -> offer retry.
    DaemonNotRunning {
        /// Underlying D-Bus error detail.
        detail: String,
    },
    /// Device is asleep, wireless receiver slot is offline, or unplugged -> offer retry.
    DeviceAsleepOrDisconnected {
        /// Human-readable device label.
        device_name: String,
    },
    /// Feature is not supported by this device's runtime feature table.
    UnsupportedFeature {
        /// Feature or page label.
        feature_name: String,
    },
    /// Fallback error message.
    Other {
        /// Error detail.
        detail: String,
    },
}

/// Classifies a [`DaemonDeviceSummary`] probe status into an [`ErrorViewState`] if inaccessible.
#[must_use]
pub fn classify_device_probe(summary: &DaemonDeviceSummary) -> Option<ErrorViewState> {
    match &summary.probe_status {
        ProbeStatus::Accessible => None,
        ProbeStatus::PermissionDenied => Some(ErrorViewState::NoHidrawPermission {
            device_name: summary.matched_name.clone(),
        }),
        ProbeStatus::TimeoutOrAsleep => Some(ErrorViewState::DeviceAsleepOrDisconnected {
            device_name: summary.matched_name.clone(),
        }),
        ProbeStatus::LegacyHidpp10 => Some(ErrorViewState::Other {
            detail: summary
                .hint
                .clone()
                .unwrap_or_else(|| "Legacy HID++ 1.0 device is not supported".to_owned()),
        }),
        ProbeStatus::Error { message } => Some(ErrorViewState::Other {
            detail: message.clone(),
        }),
        _ => Some(ErrorViewState::Other {
            detail: summary
                .hint
                .clone()
                .unwrap_or_else(|| "Device is currently unavailable".to_owned()),
        }),
    }
}

/// Capability flags controlling which sidebar pages are enabled for the selected device.
#[derive(Debug, Clone, Default)]
pub struct DeviceCapabilities {
    /// Device supports `ADJUSTABLE_DPI` (`0x2201`) or `REPORT_RATE` (`0x8060`/`0x8061`).
    pub has_dpi_or_rate: bool,
    /// Device has button metadata or `0x1B04`/`0x8110`/`0x8100`.
    pub has_buttons: bool,
    /// Device supports `COLOR_LED_EFFECTS` (`0x8070`).
    pub has_lighting: bool,
    /// Device supports `ONBOARD_PROFILES` (`0x8100`).
    pub has_profiles: bool,
    /// Device supports `SMART_SHIFT` (`0x2110`) or `HIRES_WHEEL` (`0x2121`).
    pub has_scroll: bool,
}

impl DeviceCapabilities {
    /// Computes visible page flags from live daemon reports.
    #[must_use]
    pub fn from_reports(
        status: Option<&DeviceStatusReport>,
        buttons: Option<&ButtonsReport>,
        led: Option<&LedConfigReport>,
        profiles: Option<&ProfilesListReport>,
        scroll: Option<&ScrollConfigReport>,
    ) -> Self {
        let has_dpi_or_rate = status
            .map(|s| s.dpi.is_some() || s.report_rate.is_some())
            .unwrap_or(false);
        let has_buttons = buttons
            .map(|b| {
                !b.buttons.is_empty()
                    || b.supports_reprog_v4
                    || b.supports_button_spy
                    || b.supports_onboard_profiles
            })
            .unwrap_or(false);
        let has_lighting = led.map(|l| !l.zones.is_empty()).unwrap_or(false);
        let has_profiles = profiles.is_some();
        let has_scroll = scroll
            .map(|sc| sc.smart_shift_supported || sc.hires_wheel_supported)
            .unwrap_or(false);

        Self {
            has_dpi_or_rate,
            has_buttons,
            has_lighting,
            has_profiles,
            has_scroll,
        }
    }
}

#[cfg(test)]
mod tests {
    use squeak_core::{DefinitionMatchKind, DeviceKind};

    use super::*;

    #[test]
    fn undo_stack_pushes_and_pops_last_action() {
        let mut stack = UndoStack::default();
        assert!(!stack.can_undo());
        stack.push(UndoAction::Dpi {
            device_key: "dev0".to_owned(),
            sensor_index: 0,
            previous_dpi: 1200,
        });
        assert!(stack.can_undo());
        assert_eq!(
            stack.pop(),
            Some(UndoAction::Dpi {
                device_key: "dev0".to_owned(),
                sensor_index: 0,
                previous_dpi: 1200,
            })
        );
        assert!(!stack.can_undo());
    }

    #[test]
    fn classifies_permission_denied_as_udev_prompt() {
        let summary = DaemonDeviceSummary {
            device_key: "hidraw6".to_owned(),
            path: "/dev/hidraw6".to_owned(),
            bus: "USB".to_owned(),
            vendor_id: "0x046d".to_owned(),
            product_id: "0xc08b".to_owned(),
            hid_name: None,
            device_index: "0xFF".to_owned(),
            definition_match: DefinitionMatchKind::Known,
            matched_name: "G502 HERO".to_owned(),
            kind: DeviceKind::Mouse,
            protocol_version: None,
            probe_status: ProbeStatus::PermissionDenied,
            feature_count: 0,
            desired_dpi: None,
            desired_report_rate_hz: None,
            hint: None,
        };
        assert_eq!(
            classify_device_probe(&summary),
            Some(ErrorViewState::NoHidrawPermission {
                device_name: "G502 HERO".to_owned()
            })
        );
    }
}

//! Unprivileged `hidraw` udev rule generation and `pkexec` installation helper.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use hidpp::{HidrawTransport, TransportError, discover_hidpp_devices};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// System destination path for Squeak's `uaccess` udev rule (before systemd `73-seat-late.rules`).
pub const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/70-squeak.rules";

/// Contents written to [`UDEV_RULE_PATH`].
///
/// Covers both USB (`ATTRS{idVendor}=="046d"`) and HID bus USB/Bluetooth receiver nodes
/// (`SUBSYSTEMS=="hid", KERNELS=="000[35]:0000046D:*"`) using desktop-session `uaccess` ACLs
/// instead of broad `0666` permissions.
pub const UDEV_RULE_CONTENT: &str = "\
# Squeak udev rule: grant active local user access to HID++ hidraw nodes (VID 0x046d).
# Ordered at 70- so TAG+=\"uaccess\" is applied prior to systemd's 73-seat-late.rules.
KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"046d\", TAG+=\"uaccess\"
KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", SUBSYSTEMS==\"hid\", KERNELS==\"000[35]:0000046D:*\", TAG+=\"uaccess\"
";

/// Errors raised during `squeak setup-udev`.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum UdevSetupError {
    /// Failed discovering connected hidraw endpoints under `/sys/class/hidraw`.
    #[error("failed scanning hidraw devices: {0}")]
    Discovery(#[from] TransportError),

    /// Could not spawn `pkexec` (for example polkit is unavailable or not installed).
    #[error(
        "failed launching `pkexec` ({source}); install rule manually as root at {UDEV_RULE_PATH}"
    )]
    PkexecUnavailable {
        /// Underlying OS spawn error.
        #[source]
        source: std::io::Error,
    },

    /// `pkexec` returned a non-zero exit status (user cancelled auth or command failed).
    #[error("`pkexec` installation failed ({reason})")]
    PkexecFailed {
        /// Human-readable exit detail.
        reason: String,
    },
}

/// Access verification result for one discovered `hidraw` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAccessCheck {
    /// Device node path (e.g. `/dev/hidraw1`).
    pub path: String,
    /// Vendor ID formatted in hex.
    pub vendor_id: String,
    /// Product ID formatted in hex.
    pub product_id: String,
    /// True if current user can open `path` with `O_RDWR`.
    pub writable: bool,
    /// Short outcome detail (`"accessible"`, `"permission denied"`, etc.).
    pub status: String,
}

/// Summary of `setup-udev` plan or execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupUdevReport {
    /// Whether `--dry-run` was requested.
    pub dry_run: bool,
    /// Target rule path on disk.
    pub rule_path: String,
    /// Exact udev rule contents.
    pub rule_content: String,
    /// Exact privileged shell actions needed/executed via `pkexec`.
    pub commands: Vec<String>,
    /// True when the existing rule file already matches [`UDEV_RULE_CONTENT`].
    pub rule_already_matches: bool,
    /// True if installation executed and succeeded during this invocation.
    pub applied: bool,
    /// Openability checks against connected HID++ `hidraw` nodes.
    pub verified_devices: Vec<DeviceAccessCheck>,
}

/// Evaluates whether the rule file at `rule_path` already equals `expected`.
#[must_use]
pub fn rule_matches_disk(rule_path: &Path, expected: &str) -> bool {
    fs::read_to_string(rule_path)
        .map(|existing| existing.trim() == expected.trim())
        .unwrap_or(false)
}

/// Planned privileged shell commands shown on `--dry-run`.
#[must_use]
pub fn planned_commands() -> Vec<String> {
    vec![
        "install -d -m 0755 /etc/udev/rules.d".to_owned(),
        format!("write {UDEV_RULE_PATH} (mode 0644, atomic temp file; restorecon if available)"),
        "udevadm control --reload".to_owned(),
        "udevadm trigger --subsystem-match=hidraw".to_owned(),
        "udevadm settle --timeout=5".to_owned(),
    ]
}

/// Builds a [`SetupUdevReport`] or performs `pkexec` installation when `dry_run` is false.
///
/// # Errors
///
/// Returns [`UdevSetupError`] if `hidraw` discovery or `pkexec` execution fails.
pub fn run_setup_udev(dry_run: bool) -> Result<SetupUdevReport, UdevSetupError> {
    let rule_path = Path::new(UDEV_RULE_PATH);
    let already_matches = rule_matches_disk(rule_path, UDEV_RULE_CONTENT);

    let applied = if dry_run {
        false
    } else {
        install_via_pkexec(UDEV_RULE_CONTENT)?;
        true
    };

    let verified_devices = check_discovered_access()?;

    Ok(SetupUdevReport {
        dry_run,
        rule_path: UDEV_RULE_PATH.to_owned(),
        rule_content: UDEV_RULE_CONTENT.to_owned(),
        commands: planned_commands(),
        rule_already_matches: already_matches,
        applied,
        verified_devices,
    })
}

fn build_pkexec_args(rule_content: &str) -> [String; 5] {
    // Pass rule_content strictly as positional parameter `$1` so no shell interpolation can occur,
    // keep stdin inherited for interactive polkit text prompts (`pkttyagent`), and create the
    // temp file inside `/etc/udev/rules.d` so SELinux context stays consistent on Fedora.
    let script = format!(
        "set -e\n\
         install -d -m 0755 /etc/udev/rules.d\n\
         tmp=$(mktemp /etc/udev/rules.d/.70-squeak.rules.XXXXXX)\n\
         trap 'rm -f \"$tmp\"' EXIT\n\
         printf '%s' \"$1\" > \"$tmp\"\n\
         chmod 0644 \"$tmp\"\n\
         mv -f \"$tmp\" {UDEV_RULE_PATH}\n\
         if command -v restorecon >/dev/null 2>&1; then restorecon {UDEV_RULE_PATH} || true; fi\n\
         udevadm control --reload\n\
         udevadm trigger --subsystem-match=hidraw\n\
         udevadm settle --timeout=5\n"
    );

    [
        "sh".to_owned(),
        "-c".to_owned(),
        script,
        "squeak-setup-udev".to_owned(),
        rule_content.to_owned(),
    ]
}

fn install_via_pkexec(rule_content: &str) -> Result<(), UdevSetupError> {
    let args = build_pkexec_args(rule_content);
    let output = Command::new("pkexec")
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| UdevSetupError::PkexecUnavailable { source })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let detail = match output.status.code() {
        Some(126) => "authentication dismissed by user (exit 126)".to_owned(),
        Some(127) => "pkexec authentication failed or helper missing (exit 127)".to_owned(),
        Some(code) if stderr.is_empty() => format!("exit status {code}"),
        Some(code) => format!("exit status {code}: {stderr}"),
        None => "terminated by signal".to_owned(),
    };
    Err(UdevSetupError::PkexecFailed { reason: detail })
}

fn check_discovered_access() -> Result<Vec<DeviceAccessCheck>, UdevSetupError> {
    let devices = discover_hidpp_devices()?;
    let mut checks = Vec::with_capacity(devices.len());
    for info in devices {
        let path_str = info.path.display().to_string();
        let vendor_id = format!("0x{:04x}", info.vendor_id);
        let product_id = format!("0x{:04x}", info.product_id);
        let (writable, status) = match HidrawTransport::open(info) {
            Ok(_) => (true, "accessible".to_owned()),
            Err(TransportError::PermissionDenied { .. }) => (false, "permission denied".to_owned()),
            Err(err) => (false, err.to_string()),
        };
        checks.push(DeviceAccessCheck {
            path: path_str,
            vendor_id,
            product_id,
            writable,
            status,
        });
    }
    Ok(checks)
}

/// Formats a [`SetupUdevReport`] as human-readable text.
#[must_use]
pub fn format_setup_udev_human(report: &SetupUdevReport) -> String {
    let mut out = String::new();
    let mode = if report.dry_run {
        "DRY RUN (no system files modified)"
    } else if report.applied {
        "APPLIED"
    } else {
        "SKIPPED"
    };

    let _ = writeln!(out, "setup-udev status: {mode}");
    let _ = writeln!(out, "Target file:       {}", report.rule_path);
    let _ = writeln!(
        out,
        "Existing file:     {}",
        if report.rule_already_matches {
            "already matches"
        } else {
            "missing or differs"
        }
    );
    let _ = writeln!(out, "\nRule content:");
    for line in report.rule_content.lines() {
        let _ = writeln!(out, "  {line}");
    }
    let _ = writeln!(out, "\nPrivileged actions:");
    for cmd in &report.commands {
        let _ = writeln!(out, "  - {cmd}");
    }
    let _ = writeln!(out, "\nConnected HID++ node access:");
    if report.verified_devices.is_empty() {
        let _ = writeln!(out, "  (no HID++ hidraw devices currently attached)");
    } else {
        for dev in &report.verified_devices {
            let _ = writeln!(
                out,
                "  - {} ({}:{}) -> {}",
                dev.path, dev.vendor_id, dev.product_id, dev.status
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn udev_rule_contains_usb_and_hid_bus_uaccess_matches() {
        assert!(UDEV_RULE_CONTENT.contains("KERNEL==\"hidraw*\""));
        assert!(UDEV_RULE_CONTENT.contains("ATTRS{idVendor}==\"046d\""));
        assert!(UDEV_RULE_CONTENT.contains("SUBSYSTEMS==\"hid\""));
        assert!(UDEV_RULE_CONTENT.contains("TAG+=\"uaccess\""));
        assert!(UDEV_RULE_PATH.ends_with("/70-squeak.rules"));
    }

    #[test]
    fn rule_matches_disk_compares_normalized_contents() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "squeak-udev-rule-test-{}-{id}.rules",
            std::process::id()
        ));

        assert!(!rule_matches_disk(&path, UDEV_RULE_CONTENT));
        fs::write(&path, format!("{UDEV_RULE_CONTENT}\n")).unwrap();
        assert!(rule_matches_disk(&path, UDEV_RULE_CONTENT));
        fs::write(&path, "outdated").unwrap();
        assert!(!rule_matches_disk(&path, UDEV_RULE_CONTENT));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn dry_run_setup_udev_succeeds_without_pkexec_and_serializes() {
        let report = run_setup_udev(true).unwrap();
        assert!(report.dry_run);
        assert!(!report.applied);
        assert_eq!(report.rule_path, UDEV_RULE_PATH);

        let json = serde_json::to_string_pretty(&report).unwrap();
        let parsed: SetupUdevReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);

        let human = format_setup_udev_human(&report);
        assert!(human.contains("DRY RUN"));
        assert!(human.contains("udevadm control --reload"));
        assert!(human.contains("udevadm settle --timeout=5"));
    }

    #[test]
    fn pkexec_args_pass_rule_content_verbatim_as_positional_arg() {
        let payload = "ATTRS{idVendor}==\"046d\", RUN+=\"$(id)\" `echo` $HOME\n";
        let args = build_pkexec_args(payload);
        assert_eq!(args[0], "sh");
        assert_eq!(args[1], "-c");
        assert!(args[2].contains("printf '%s' \"$1\" > \"$tmp\""));
        assert!(args[2].contains("udevadm control --reload"));
        assert!(args[2].contains("udevadm settle --timeout=5"));
        assert_eq!(args[3], "squeak-setup-udev");
        assert_eq!(args[4], payload);
    }
}

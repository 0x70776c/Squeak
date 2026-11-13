//! Command-line argument definitions (`clap`) for the `squeak` binary.

use clap::{Args, Parser, Subcommand};

/// Top-level CLI entrypoint for `squeak`.
#[derive(Debug, Parser)]
#[command(
    name = "squeak",
    version,
    about = "Open-source HID++ 2.0 peripheral control & diagnostics CLI"
)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout instead of human tables.
    #[arg(long, global = true)]
    pub json: bool,

    /// Enable verbose diagnostic logging on stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Command to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Supported `squeak` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List connected HID++ devices detected under `/sys/class/hidraw`.
    List(ListArgs),

    /// Show protocol summary and discovered capability overview for one device.
    Info(TargetArgs),

    /// Read live M3 capabilities and state (name, firmware, battery, DPI, report rate).
    Status(TargetArgs),

    /// Inspect or update adjustable sensor DPI (`0x2201`).
    Dpi(DpiArgs),

    /// Inspect or update polling report rate (`0x8060` / `0x8061`).
    #[command(name = "report-rate")]
    ReportRate(ReportRateArgs),

    /// Dump the complete runtime feature table for a device (without serial numbers).
    #[command(name = "dump-device")]
    DumpDevice(TargetArgs),

    /// Install `/etc/udev/rules.d/70-squeak.rules` via `pkexec` for unprivileged hidraw access.
    #[command(name = "setup-udev")]
    SetupUdev(SetupUdevArgs),
}

/// Arguments for `squeak list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Target HID++ device index to probe (`direct` / `0xFF`, or receiver slot `1..=6`).
    #[arg(long, default_value = "direct")]
    pub device_index: String,
}

/// Common device selection arguments for `info` and `dump-device`.
#[derive(Debug, Args)]
pub struct TargetArgs {
    /// Device selector (`/dev/hidraw1`, `hidraw1`, or `vid:pid` like `046d:c08b`).
    /// Optional when exactly one HID++ device is connected.
    #[arg(short, long)]
    pub device: Option<String>,

    /// Target HID++ device index (`direct` / `0xFF`, or receiver slot `1..=6`).
    #[arg(long, default_value = "direct")]
    pub device_index: String,
}

/// Arguments for `squeak setup-udev`.
#[derive(Debug, Args)]
pub struct SetupUdevArgs {
    /// Print planned rule file, commands, and permission checks without modifying system files.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments for `squeak dpi`.
#[derive(Debug, Args)]
pub struct DpiArgs {
    /// Action to perform on `ADJUSTABLE_DPI` (`0x2201`).
    #[command(subcommand)]
    pub action: DpiCommand,
}

/// Subcommands under `squeak dpi`.
#[derive(Debug, Subcommand)]
pub enum DpiCommand {
    /// Read active DPI, default DPI, and supported DPI values/ranges.
    Get(DpiGetArgs),

    /// Set active DPI for a sensor (dry-run by default; pass `--apply` to write to hardware).
    Set(DpiSetArgs),
}

/// Arguments for `squeak dpi get`.
#[derive(Debug, Args)]
pub struct DpiGetArgs {
    /// Optional sensor index to filter (`0..sensor_count`). Reads all sensors when omitted.
    #[arg(long)]
    pub sensor: Option<u8>,

    /// Target device selection flags.
    #[command(flatten)]
    pub target: TargetArgs,
}

/// Arguments for `squeak dpi set`.
#[derive(Debug, Args)]
pub struct DpiSetArgs {
    /// Target DPI value (validated against the device's advertised DPI capabilities).
    pub dpi: u16,

    /// Sensor index to configure (`0` for standard single-sensor mice).
    #[arg(long, default_value_t = 0)]
    pub sensor: u8,

    /// Actually transmit the write command to hardware and verify readback.
    /// When omitted, `dpi set` performs capability validation and prints a `--dry-run` plan only.
    #[arg(long, conflicts_with = "dry_run")]
    pub apply: bool,

    /// Explicitly preview the validated write plan without sending any mutating frame (default).
    #[arg(long)]
    pub dry_run: bool,

    /// Target device selection flags.
    #[command(flatten)]
    pub target: TargetArgs,
}

/// Arguments for `squeak report-rate`.
#[derive(Debug, Args)]
pub struct ReportRateArgs {
    /// Action to perform on `REPORT_RATE` (`0x8060`) / `EXTENDED_REPORT_RATE` (`0x8061`).
    #[command(subcommand)]
    pub action: ReportRateCommand,
}

/// Subcommands under `squeak report-rate`.
#[derive(Debug, Subcommand)]
pub enum ReportRateCommand {
    /// Read current and supported polling rates in Hz.
    Get(TargetArgs),

    /// Set polling rate in Hz (dry-run by default; pass `--apply` to write to hardware).
    Set(ReportRateSetArgs),
}

/// Arguments for `squeak report-rate set`.
#[derive(Debug, Args)]
pub struct ReportRateSetArgs {
    /// Target polling rate in Hz (e.g. `125`, `250`, `500`, `1000`, `2000`, `4000`, `8000`).
    pub hz: u16,

    /// Actually transmit the write command to hardware and verify readback.
    /// When omitted, `report-rate set` performs capability validation and prints a `--dry-run` plan only.
    #[arg(long, conflicts_with = "dry_run")]
    pub apply: bool,

    /// Explicitly preview the validated write plan without sending any mutating frame (default).
    #[arg(long)]
    pub dry_run: bool,

    /// Target device selection flags.
    #[command(flatten)]
    pub target: TargetArgs,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_all_subcommands_with_global_json_flag() {
        let list = Cli::try_parse_from(["squeak", "list", "--json"]).unwrap();
        assert!(list.json);
        assert!(matches!(list.command, Command::List(_)));

        let dump = Cli::try_parse_from([
            "squeak",
            "--json",
            "dump-device",
            "--device",
            "046d:c08b",
            "--device-index",
            "direct",
        ])
        .unwrap();
        assert!(dump.json);
        match dump.command {
            Command::DumpDevice(args) => {
                assert_eq!(args.device.as_deref(), Some("046d:c08b"));
                assert_eq!(args.device_index, "direct");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let udev = Cli::try_parse_from(["squeak", "setup-udev", "--dry-run"]).unwrap();
        assert!(matches!(
            udev.command,
            Command::SetupUdev(SetupUdevArgs { dry_run: true })
        ));

        let status = Cli::try_parse_from(["squeak", "status", "--device", "046d:c08b"]).unwrap();
        assert!(matches!(status.command, Command::Status(_)));

        let dpi_set =
            Cli::try_parse_from(["squeak", "dpi", "set", "1600", "--sensor", "0"]).unwrap();
        match dpi_set.command {
            Command::Dpi(DpiArgs {
                action: DpiCommand::Set(args),
            }) => {
                assert_eq!(args.dpi, 1600);
                assert_eq!(args.sensor, 0);
                assert!(!args.apply);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert!(
            Cli::try_parse_from(["squeak", "dpi", "set", "1600", "--apply", "--dry-run"]).is_err()
        );

        let rr_set =
            Cli::try_parse_from(["squeak", "report-rate", "set", "500", "--apply"]).unwrap();
        match rr_set.command {
            Command::ReportRate(ReportRateArgs {
                action: ReportRateCommand::Set(args),
            }) => {
                assert_eq!(args.hz, 500);
                assert!(args.apply);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}

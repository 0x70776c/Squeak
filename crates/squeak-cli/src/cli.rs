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
    }
}

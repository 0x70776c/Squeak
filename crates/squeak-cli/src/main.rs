//! Squeak command-line interface (`squeak`) entrypoint.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod m3;
mod udev;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::Cli;

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match commands::execute(cli) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

fn init_tracing(verbose: bool) {
    let default_level = if verbose { "debug" } else { "warn" };
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
    fn cli_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

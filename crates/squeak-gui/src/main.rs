//! Squeak GTK4/libadwaita GUI (`squeak-gui`).
//!
//! By architectural rule (ADR-001), `squeak-gui` contains zero protocol logic
//! and communicates exclusively with `squeakd` over D-Bus.

#![forbid(unsafe_code)]

pub mod dbus_client;
pub mod i18n;
pub mod state;
pub mod svg_diagram;

use futures_lite as _;
use serde as _;
use serde_json as _;
use tokio as _;
use tracing as _;

fn main() {
    println!(
        "squeak-gui {} (D-Bus client ready)",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn gui_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

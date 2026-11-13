//! Squeak GTK4/libadwaita GUI (`squeak-gui`).
//!
//! By architectural rule (ADR-001), `squeak-gui` contains zero protocol logic
//! and communicates exclusively with `squeakd` over D-Bus.

#![forbid(unsafe_code)]

pub mod backend;
pub mod dbus_client;
pub mod i18n;
pub mod state;
pub mod svg_diagram;
pub mod ui;

use serde as _;

fn main() -> gtk4::glib::ExitCode {
    ui::launch_gui()
}

#[cfg(test)]
mod tests {
    #[test]
    fn gui_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

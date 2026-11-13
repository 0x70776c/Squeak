//! Squeak GTK4/libadwaita GUI (`squeak-gui`) scaffold.
//!
//! By architectural rule (ADR-001), `squeak-gui` contains zero protocol logic
//! and communicates exclusively with `squeakd` over D-Bus.

use squeak_core as _;

fn main() {
    println!("squeak-gui {} (scaffold)", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn gui_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}

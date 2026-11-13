//! Squeak background daemon (`squeakd`) library.
//!
//! Owns connected HID++ `hidraw` devices, manages hotplug and runtime `FeatureTable`
//! invalidation/re-apply, and exposes `dev.squeak.Daemon` over D-Bus.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

pub mod dbus;
pub mod error;
pub mod hotplug;
pub mod registry;

// Silence unused-crate-dependencies lint when compiling as a library,
// as these dependencies are consumed by the squeakd binary.
use clap as _;
use serde as _;
use tracing_subscriber as _;

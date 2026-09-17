//! Basalt host: serves one locked-in drive over the LAN.
//!
//! The pieces, roughly in order of how much damage a bug in each would do:
//!
//! - [`vault`] turns untrusted relative paths into real ones, and is the
//!   security boundary of the entire system.
//! - [`registry`] decides who may connect at all.
//! - [`uploads`] makes sure a failed transfer can never replace a good file.
//! - [`server`] is the loop that wires those together onto a socket.
//! - [`config`] is what survives a restart, the host's identity above all.
//!
//! The rest exist for the host's own window: [`drives`] lists what could be
//! served, [`rates`] turns [`traffic`]'s counters into speeds over a measured
//! interval, [`autostart`] is the Windows startup entry, and [`ui`] holds the
//! shapes that window receives — here rather than in the Tauri shell, which no
//! test ever runs.

pub mod autostart;
pub mod config;
pub mod drives;
pub mod error;
pub mod media;
pub mod rates;
pub mod registry;
pub mod server;
pub mod space;
pub mod traffic;
pub mod ui;
pub mod uploads;
pub mod vault;
pub mod watch;

pub use config::HostConfig;
pub use drives::Drive;
pub use error::{HostError, Result};
pub use media::Library;
pub use rates::{Rate, Rates};
pub use registry::{Device, PairingRequest, Registry};
pub use server::{Host, bind, serve};
pub use traffic::{DeviceTraffic, Traffic};
pub use vault::Vault;
pub use watch::Watch;

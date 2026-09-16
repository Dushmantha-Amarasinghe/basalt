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

pub mod config;
pub mod error;
pub mod registry;
pub mod server;
pub mod space;
pub mod uploads;
pub mod vault;

pub use config::HostConfig;
pub use error::{HostError, Result};
pub use registry::{Device, Registry};
pub use server::{Host, bind, serve};
pub use vault::Vault;

//! MissionCtrl core.
//!
//! Platform-independent backend shared by the Android and iOS apps:
//! SSH transport ([`transport`]), the herdr socket API ([`herdr`]), the
//! herdr-core API ([`hc`]), the
//! per-host runtime with reconnect and live state ([`host`]), and the
//! multi-host aggregation ([`fleet`]), and the live terminal view of one
//! pane ([`terminal`]).

pub mod detect;
pub mod error;
pub mod fleet;
pub mod hc;
pub mod herdr;
pub mod host;
pub mod keys;
pub mod profile;
pub mod server;
pub mod terminal;
pub mod transport;

pub use detect::{Detected, Presence, detect_backends, probe as probe_backend};
pub use error::{Error, Result};
pub use fleet::{Fleet, FleetAgent, FleetView};
pub use host::{ConnectionState, Connector, ErrorInfo, HostHandle, HostView};
pub use profile::{AuthRef, Backend, HostProfile};
pub use server::ServerClient;

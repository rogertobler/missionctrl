//! MissionCtrl core.
//!
//! Platform-independent backend shared by the Android and iOS apps:
//! SSH transport ([`transport`]), the herdr socket API ([`herdr`]), the
//! per-host runtime with reconnect and live state ([`host`]), and the
//! multi-host aggregation ([`fleet`]), and the live terminal view of one
//! pane ([`terminal`]).

pub mod error;
pub mod fleet;
pub mod herdr;
pub mod host;
pub mod keys;
pub mod profile;
pub mod terminal;
pub mod transport;

pub use error::{Error, Result};
pub use fleet::{Fleet, FleetAgent, FleetView};
pub use host::{ConnectionState, Connector, ErrorInfo, HostHandle, HostView};
pub use profile::{AuthRef, HostProfile};

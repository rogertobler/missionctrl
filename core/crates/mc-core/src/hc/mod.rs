//! herdr-core API: JSON-RPC 2.0 over herdr-core's Unix socket.
//!
//! herdr-core (`herdr-core`/`hc`, Apache-2.0) is the server of the Herdr
//! desktop app. It knows only a flat list of terminals; spaces, tabs and the
//! split layout belong to the desktop app, which stores them in the core as
//! the document `herdr-app`. [`snapshot::build`] maps both onto the same
//! [`SessionSnapshot`](crate::herdr::model::SessionSnapshot) the herdr TUI
//! delivers, so the rest of MissionCtrl does not need to know which server it
//! talks to.
//!
//! Sources: `schema/hc.openrpc.json` and `schema/README.md` in herdr-core
//! (protocol generation 1, hc 0.1.20), `docs/labels.md` and `src/mirror.rs`,
//! `src/layout.rs` in herdr-app (0.1.15).

pub mod attach;
mod client;
pub mod framed;
pub mod layout;
pub mod model;
pub mod snapshot;
pub mod write;

pub use client::{
    CoreClient, CoreSubscription, Endpoint, GENERATION, Notification, SeenMarks, connect_command,
    find_core_binary, socket_candidates,
};

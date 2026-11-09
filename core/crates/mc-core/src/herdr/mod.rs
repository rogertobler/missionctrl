//! herdr socket API: JSON requests, snapshots, and event subscriptions.

mod client;
pub mod model;

pub use client::{HerdrClient, Subscription, socket_candidates};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A saved server. Contains no secrets: private keys and passphrases are
/// provided separately at connect time (see [`crate::transport::SshAuth`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostProfile {
    /// Stable id chosen by the app, unique across profiles.
    pub id: String,
    /// Display name, e.g. "build-server".
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    /// Named herdr session (`herdr --session <name>`); `None` = default session.
    #[serde(default)]
    pub session: Option<String>,
    /// Absolute API socket path on the server. Overrides discovery.
    #[serde(default)]
    pub socket_path: Option<String>,
}

fn default_port() -> u16 {
    22
}

/// How a profile authenticates. Secrets themselves are never serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthRef {
    /// Use an ssh-agent socket (desktop development).
    Agent {
        socket: PathBuf,
        /// Only offer agent keys whose comment or fingerprint contains this.
        #[serde(default)]
        key_filter: Option<String>,
    },
    /// Private key file in OpenSSH format.
    KeyFile { path: PathBuf },
}

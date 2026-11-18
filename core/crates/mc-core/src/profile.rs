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
    /// Which herdr server to talk to. Profiles saved without it use
    /// [`Backend::Auto`].
    #[serde(default)]
    pub backend: Backend,
    /// On herdr-core: the app may change the Herdr app's layout (create,
    /// rename, move, close workspaces and tabs). Off unless turned on.
    #[serde(default)]
    pub edit_layout: bool,
}

/// The herdr server a profile talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// Detect it: the herdr TUI's server, or herdr-core when the TUI is
    /// surely not there and herdr-core already runs. Never starts a server
    /// (see `host::select_backend`).
    #[default]
    Auto,
    /// The herdr TUI's own server (`herdr server`).
    Tui,
    /// herdr-core (`herdr-core`/`hc`), the server of the Herdr desktop app.
    /// Started through `herdr-core connect` when it is installed but not
    /// running.
    Core,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Tui => "tui",
            Backend::Core => "core",
        }
    }

    /// Parses [`Self::as_str`]; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Backend::Auto),
            "tui" => Some(Backend::Tui),
            "core" => Some(Backend::Core),
            _ => None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_without_backend_is_auto() {
        let p: HostProfile =
            serde_json::from_str(r#"{"id":"a","name":"a","host":"example.com","user":"u"}"#)
                .unwrap();
        assert_eq!(p.backend, Backend::Auto);
        let p: HostProfile = serde_json::from_str(
            r#"{"id":"a","name":"a","host":"example.com","user":"u","backend":"core"}"#,
        )
        .unwrap();
        assert_eq!(p.backend, Backend::Core);
        for b in [Backend::Auto, Backend::Tui, Backend::Core] {
            assert_eq!(Backend::parse(b.as_str()), Some(b));
        }
    }
}

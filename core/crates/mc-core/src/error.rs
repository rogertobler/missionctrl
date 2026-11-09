use std::io;

/// Errors surfaced by the core. Variants are typed so a UI can react to them
/// (e.g. ask the user to trust a host key) without parsing messages.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection failed: {0}")]
    Connect(String),

    #[error("connection timed out")]
    Timeout,

    #[error("unknown host key for {host}:{port} ({fingerprint})")]
    HostKeyUnknown {
        host: String,
        port: u16,
        fingerprint: String,
        /// OpenSSH public key line, so the caller can persist a trust decision.
        openssh: String,
    },

    #[error("HOST KEY CHANGED for {host}:{port} (now {fingerprint})")]
    HostKeyChanged {
        host: String,
        port: u16,
        fingerprint: String,
    },

    #[error("authentication failed for user {user}")]
    AuthFailed { user: String },

    #[error("ssh agent: {0}")]
    Agent(String),

    #[error("ssh: {0}")]
    Ssh(#[from] russh::Error),

    #[error("herdr socket not reachable at {path}: {reason}")]
    SocketUnavailable { path: String, reason: String },

    #[error("herdr error {code}: {message}")]
    Herdr { code: String, message: String },

    #[error("herdr protocol: {0}")]
    Protocol(String),

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("host is not connected")]
    NotConnected,

    /// A directory given for a new workspace or tab does not exist on the server.
    #[error("no such directory: {path}")]
    NoSuchDirectory { path: String },
}

impl Error {
    /// herdr error code, if this is an error response from the server.
    pub fn herdr_code(&self) -> Option<&str> {
        match self {
            Error::Herdr { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

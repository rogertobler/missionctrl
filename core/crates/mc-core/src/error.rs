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

    /// The action is not available for this server's backend (yet).
    #[error("not supported for {backend}: {what}")]
    Unsupported { backend: String, what: String },

    /// The server speaks a protocol this build cannot use (e.g. a newer
    /// herdr-core generation); MissionCtrl needs an update.
    #[error("incompatible server: {what}")]
    Incompatible { what: String },

    /// A directory given for a new workspace or tab does not exist on the server.
    #[error("no such directory: {path}")]
    NoSuchDirectory { path: String },

    /// A change to the Herdr app's layout (herdr-core) could not be made, or
    /// only partly.
    #[error("layout {problem}: {detail}")]
    Layout {
        problem: LayoutProblem,
        detail: String,
    },
}

/// Why a change to the Herdr app's layout did not (fully) happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProblem {
    /// The layout is in a shape this build does not write.
    Unreadable,
    /// It kept changing elsewhere while MissionCtrl tried to write it.
    Conflict,
    /// Some of it happened: terminals still running, or new content
    /// appeared meanwhile and was left alone.
    Partial,
    /// Closing would end a terminal shown elsewhere (herdr-core ends
    /// children with their parent).
    ChildElsewhere,
    /// It holds terminals of another machine.
    OtherMachine,
    /// No answer: whether it happened is not known; reload and look.
    Unclear,
    /// The workspace or tab is not there any more.
    Gone,
    /// An earlier start for the same place is not settled yet.
    Busy,
    /// A terminal shown in the tab is labelled for a tab that is not in
    /// the layout (the desktop may still be writing it, or nobody adopts
    /// it): closing waits until it has a place.
    Unplaced,
    /// The copy of the layout kept before the first change could not be
    /// written: nothing was changed.
    Backup,
    /// The change would make the layout larger than herdr-core accepts:
    /// nothing was changed.
    TooLarge,
    /// The loose terminal to close stands in a workspace by now.
    Placed,
    /// The loose terminal to close is labelled for a workspace without
    /// tabs, which the Herdr app may be adopting it into: nothing ended.
    Claimed,
}

impl LayoutProblem {
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutProblem::Unreadable => "unreadable",
            LayoutProblem::Conflict => "conflict",
            LayoutProblem::Partial => "partial",
            LayoutProblem::ChildElsewhere => "child_elsewhere",
            LayoutProblem::OtherMachine => "other_machine",
            LayoutProblem::Unclear => "unclear",
            LayoutProblem::Gone => "gone",
            LayoutProblem::Busy => "busy",
            LayoutProblem::Unplaced => "unplaced",
            LayoutProblem::Backup => "backup",
            LayoutProblem::TooLarge => "too_large",
            LayoutProblem::Placed => "placed",
            LayoutProblem::Claimed => "claimed",
        }
    }
}

impl std::fmt::Display for LayoutProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A [`Error::Layout`].
pub fn layout(problem: LayoutProblem, detail: impl Into<String>) -> Error {
    Error::Layout {
        problem,
        detail: detail.into(),
    }
}

impl Error {
    /// herdr error code, if this is an error response from the server.
    pub fn herdr_code(&self) -> Option<&str> {
        match self {
            Error::Herdr { code, .. } => Some(code),
            _ => None,
        }
    }

    /// The server is surely not there: its socket refuses or does not exist,
    /// or a different server answered the handshake (an error reply or an
    /// answer in another shape). Not absent: timeouts, a connection closed
    /// half-way, SSH trouble (they pass), and an incompatible server (it is
    /// there, MissionCtrl needs an update).
    pub fn is_absent(&self) -> bool {
        matches!(
            self,
            Error::SocketUnavailable { .. } | Error::Herdr { .. } | Error::Json(_)
        )
    }
}

/// Of the errors from several places a server could be, the one to report:
/// one that hints at a server being there (a timeout, an incompatible
/// answer) wins over "nothing here", so a hanging server is never taken for
/// a missing one.
pub(crate) fn telling(errors: Vec<Error>) -> Option<Error> {
    let mut absent = None;
    for e in errors {
        if !e.is_absent() {
            return Some(e);
        }
        absent = Some(e);
    }
    absent
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

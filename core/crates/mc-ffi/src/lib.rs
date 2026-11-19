//! UniFFI bindings: the MissionCtrl core as seen from Kotlin and Swift.
//!
//! The app owns persistence (servers, keys, trusted host keys) and passes it
//! in; the core owns connections and live state and reports every change
//! through [`FleetObserver`]. Blocking methods must be called from a
//! background thread, never from the observer callback.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use mc_core::hc::CoreClient;
use mc_core::hc::layout::Anchor;
use mc_core::hc::write::{self as layout_write, LayoutState, NewTab, Start, Target};
use mc_core::herdr::HerdrClient;
use mc_core::herdr::model::{AgentStatus, PaneRead};
use mc_core::keys::{
    ImportError, PrivateKey, TrustStore, encrypt_private_key, generate_ed25519, import_private_key,
    is_hardware_key, parse_private_key, parse_public_key,
};
use mc_core::terminal::{
    Color as TermColorCore, Ended, Screen, ScrollDirection, TerminalSession, TerminalSink,
};
use mc_core::transport::{SshAuth, SshTarget, SshTransport, Transport, Zeroizing};
use mc_core::{
    Backend, ConnectionState, Connector, ErrorInfo, Fleet, FleetView, HostProfile, ServerClient,
};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum McError {
    #[error("{reason}")]
    Failed { reason: String },
    #[error("passphrase required")]
    PassphraseRequired,
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// The server is unknown or not connected right now.
    #[error("server is not connected")]
    NotConnected,
    #[error("timed out")]
    Timeout,
    /// The directory for a new workspace or tab does not exist on the server.
    #[error("no such directory: {path}")]
    NoSuchDirectory { path: String },
    /// The server's host key is not trusted yet: show it, and after the user
    /// trusts it ([`MissionCtrl::trust_host_key`]) try again.
    #[error("unknown host key for {host}:{port}")]
    HostKeyUnknown {
        host: String,
        port: u16,
        fingerprint: String,
        openssh: String,
    },
    /// The server's host key differs from the trusted one.
    #[error("host key changed for {host}:{port}")]
    HostKeyChanged {
        host: String,
        port: u16,
        fingerprint: String,
    },
    #[error("authentication failed for user {user}")]
    AuthFailed { user: String },
    /// A workspace or tab change on herdr-core did not (fully) happen.
    /// `problem`: one of `LayoutProblem::as_str` (unreadable, conflict,
    /// partial, child_elsewhere, other_machine, unclear, gone, busy,
    /// unplaced, backup, too_large, placed, claimed); `detail` for logs.
    #[error("layout {problem}: {detail}")]
    Layout { problem: String, detail: String },
}

impl From<mc_core::Error> for McError {
    fn from(e: mc_core::Error) -> Self {
        match e {
            mc_core::Error::NotConnected => McError::NotConnected,
            mc_core::Error::Timeout => McError::Timeout,
            mc_core::Error::NoSuchDirectory { path } => McError::NoSuchDirectory { path },
            mc_core::Error::HostKeyUnknown {
                host,
                port,
                fingerprint,
                openssh,
            } => McError::HostKeyUnknown {
                host,
                port,
                fingerprint,
                openssh,
            },
            mc_core::Error::HostKeyChanged {
                host,
                port,
                fingerprint,
            } => McError::HostKeyChanged {
                host,
                port,
                fingerprint,
            },
            mc_core::Error::AuthFailed { user } => McError::AuthFailed { user },
            mc_core::Error::Layout { problem, detail } => McError::Layout {
                problem: problem.as_str().to_owned(),
                detail,
            },
            e => McError::Failed {
                reason: e.to_string(),
            },
        }
    }
}

impl From<ImportError> for McError {
    fn from(e: ImportError) -> Self {
        match e {
            ImportError::PassphraseRequired => McError::PassphraseRequired,
            ImportError::WrongPassphrase => McError::WrongPassphrase,
            ImportError::Invalid(reason) => McError::Failed { reason },
        }
    }
}

/// A saved backend name; unknown or missing means automatic detection.
fn backend_of(name: Option<&str>) -> Backend {
    name.and_then(Backend::parse).unwrap_or_default()
}

fn failed(reason: impl Into<String>) -> McError {
    McError::Failed {
        reason: reason.into(),
    }
}

/// Route core logs to the platform log (logcat tag `mc-core` on Android).
/// Safe to call more than once.
#[uniffi::export]
pub fn init_logging() {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            })
            .with_tag("mc-core"),
    );
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

#[derive(uniffi::Record, Clone)]
pub struct KeyMaterial {
    /// Unencrypted OpenSSH private key. Store it encrypted (Android Keystore).
    pub private_openssh: String,
    /// Line for `~/.ssh/authorized_keys`.
    pub public_openssh: String,
    pub fingerprint: String,
    /// e.g. `ssh-ed25519`
    pub algorithm: String,
    pub comment: String,
    /// FIDO key (`sk-…`): needs the security key to sign (not supported yet).
    pub hardware: bool,
}

/// Generate a new Ed25519 client key.
#[uniffi::export]
pub fn generate_client_key(comment: String) -> Result<KeyMaterial, McError> {
    let key = generate_ed25519(&comment)?;
    Ok(KeyMaterial {
        private_openssh: key.private_openssh,
        public_openssh: key.public_openssh,
        fingerprint: key.fingerprint,
        algorithm: "ssh-ed25519".into(),
        comment,
        hardware: false,
    })
}

/// Import a private key (OpenSSH, PEM, PuTTY) from file contents or the
/// clipboard. Fails with `PassphraseRequired` / `WrongPassphrase` so the UI
/// can ask.
#[uniffi::export]
pub fn import_client_key(text: String, passphrase: Option<String>) -> Result<KeyMaterial, McError> {
    let key = import_private_key(&text, passphrase.as_deref())?;
    Ok(KeyMaterial {
        private_openssh: key.private_openssh,
        public_openssh: key.public_openssh,
        fingerprint: key.fingerprint,
        algorithm: key.algorithm,
        comment: key.comment,
        hardware: key.hardware,
    })
}

/// A private key as an OpenSSH file encrypted with `passphrase`, for a backup
/// that `import_client_key` (or `ssh`) reads again.
#[uniffi::export]
pub fn export_client_key(private_openssh: String, passphrase: String) -> Result<String, McError> {
    Ok(encrypt_private_key(&private_openssh, &passphrase)?)
}

// ---------------------------------------------------------------------------
// State records
// ---------------------------------------------------------------------------

#[derive(uniffi::Record, Clone)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Named herdr session, or `None` for the default session.
    pub session: Option<String>,
    pub auth: ServerAuth,
    /// Which herdr server to use: "auto" (default), "tui" or "core".
    #[uniffi(default)]
    pub backend: Option<String>,
    /// Absolute path of the server's socket when it is not where herdr or
    /// herdr-core put it by default; `None` finds it.
    #[uniffi(default)]
    pub socket_path: Option<String>,
    /// herdr-core only: workspaces and tabs may be created, renamed, moved
    /// and closed (they live in the Herdr app's layout, which MissionCtrl
    /// then writes). Off: those actions fail with `NotSupported`.
    #[uniffi(default)]
    pub edit_layout: Option<bool>,
}

/// How a server authenticates.
#[derive(uniffi::Enum, Clone)]
pub enum ServerAuth {
    /// A key loaded with `add_key`.
    Key { key_id: String },
    /// Username and password (also used for keyboard-interactive). The app
    /// stores the password encrypted and passes it here.
    Password { password: String },
}

// Debug without the password, so it never ends up in a log.
impl std::fmt::Debug for ServerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerAuth::Key { key_id } => f.debug_struct("Key").field("key_id", key_id).finish(),
            ServerAuth::Password { .. } => f.write_str("Password { .. }"),
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum Connection {
    Connecting,
    Online,
    Reconnecting {
        attempt: u32,
        error: String,
        retry_in_ms: u64,
        /// No herdr server answered where the profile looks (none running,
        /// or herdr-core not installed), as opposed to one that failed.
        #[uniffi(default)]
        absent: bool,
    },
    /// First contact: the user must confirm the fingerprint.
    HostKeyUnknown {
        host: String,
        port: u16,
        fingerprint: String,
        openssh: String,
    },
    /// The server presents a different key than the trusted one.
    HostKeyChanged {
        host: String,
        port: u16,
        fingerprint: String,
    },
    /// The server rejected the client key.
    AuthFailed {
        user: String,
    },
    /// The server needs a newer MissionCtrl (a newer herdr-core generation).
    /// No automatic retry.
    Incompatible {
        what: String,
    },
    Failed {
        message: String,
    },
    Offline,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Blocked,
    Done,
    Working,
    Idle,
    Unknown,
}

impl From<AgentStatus> for Status {
    fn from(s: AgentStatus) -> Self {
        match s {
            AgentStatus::Blocked => Status::Blocked,
            AgentStatus::Done => Status::Done,
            AgentStatus::Working => Status::Working,
            AgentStatus::Idle => Status::Idle,
            AgentStatus::Unknown => Status::Unknown,
        }
    }
}

/// A herdr server found on a machine ([`MissionCtrl::detect_backends`]).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DetectedBackend {
    /// "tui" or "core".
    pub backend: String,
    /// It answers now; false: herdr-core installed, not running (a profile
    /// set to "core" starts it), or `unclear`.
    pub running: bool,
    /// The probe gave no clear answer (timeout, broken connection): this
    /// herdr may or may not be there. Never save on such a result.
    #[uniffi(default)]
    pub unclear: bool,
}

#[derive(uniffi::Record, Clone)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub connection: Connection,
    pub herdr_version: Option<String>,
    /// Shown state may be outdated (not connected right now).
    pub stale: bool,
    pub workspaces: u32,
    pub panes: u32,
    /// The server this connection talks to: "tui" or "core"; `None` until connected.
    #[uniffi(default)]
    pub backend: Option<String>,
    /// Every herdr server that answered at the last connect ("tui", "core").
    /// Both with backend "auto": the app may ask which one to use.
    #[uniffi(default)]
    pub available_backends: Vec<String>,
}

#[derive(uniffi::Record, Clone)]
pub struct AgentRow {
    pub server_id: String,
    pub server_name: String,
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    /// Best display name (custom name, display name, or type).
    pub name: String,
    /// Agent type reported by herdr, e.g. `claude`, `codex`.
    pub agent_type: Option<String>,
    /// Name given to the agent in herdr, if any.
    pub custom_name: Option<String>,
    pub status: Status,
    /// Integration-provided label such as "refactoring", else the status.
    pub status_label: String,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    /// Working directory of the agent's pane.
    pub cwd: Option<String>,
    pub stale: bool,
}

#[derive(uniffi::Record, Clone)]
pub struct TabRow {
    pub tab_id: String,
    pub label: String,
    pub status: Status,
    pub panes: u32,
    /// First pane of the tab, to open it in the detail view.
    pub first_pane_id: Option<String>,
    /// Working directory of that pane.
    pub cwd: Option<String>,
    /// Agent type running in the tab, if any.
    pub agent_type: Option<String>,
}

#[derive(uniffi::Record, Clone)]
pub struct WorkspaceRow {
    pub server_id: String,
    pub server_name: String,
    pub workspace_id: String,
    pub label: String,
    pub status: Status,
    pub tabs: Vec<TabRow>,
    pub stale: bool,
    /// Working directory of the workspace's first pane (default for new tabs).
    pub cwd: Option<String>,
}

/// Result of [`MissionCtrl::read_image`].
#[derive(uniffi::Enum, Clone, Debug)]
pub enum ImageRead {
    /// The file and its stamp, to pass as `known` next time.
    Data { data: Vec<u8>, stamp: String },
    /// Still the same as the `known` stamp; nothing sent.
    Unchanged,
    /// No readable file at that path (any more).
    Missing,
    /// Larger than the app fetches; size in bytes.
    TooLarge { size: u64 },
}

/// What a new tab runs.
#[derive(uniffi::Enum, Clone, Debug)]
pub enum TabStart {
    Shell,
    /// A managed agent, `kind` e.g. `claude` or `codex`.
    Agent {
        kind: String,
    },
    /// A shell command, typed and submitted.
    Command {
        command: String,
    },
}

#[derive(uniffi::Record, Clone)]
pub struct FleetState {
    pub servers: Vec<ServerRow>,
    /// All workspaces of all servers, in server order then herdr order.
    pub workspaces: Vec<WorkspaceRow>,
    /// Most urgent first.
    pub agents: Vec<AgentRow>,
}

/// Git facts about a working directory on a server (path line).
#[derive(uniffi::Record, Clone)]
pub struct DirInfo {
    pub dir: String,
    pub branch: Option<String>,
    pub repo: Option<String>,
}

#[derive(uniffi::Record, Clone)]
pub struct DirReport {
    /// Home directory of the login, to shorten paths to `~/…`.
    pub home: String,
    pub dirs: Vec<DirInfo>,
}

#[derive(uniffi::Record, Clone)]
pub struct PaneText {
    pub text: String,
    /// Unchanged revision means unchanged content.
    pub revision: u64,
    pub truncated: bool,
}

impl From<PaneRead> for PaneText {
    fn from(read: PaneRead) -> Self {
        Self {
            text: read.text,
            revision: read.revision,
            truncated: read.truncated,
        }
    }
}

fn connection(state: &ConnectionState) -> Connection {
    match state {
        ConnectionState::Connecting => Connection::Connecting,
        ConnectionState::Online => Connection::Online,
        ConnectionState::Reconnecting {
            attempt,
            error,
            retry_in_ms,
            absent,
        } => Connection::Reconnecting {
            attempt: *attempt,
            error: error.clone(),
            retry_in_ms: *retry_in_ms,
            absent: *absent,
        },
        ConnectionState::ActionRequired { error } => match error {
            ErrorInfo::HostKeyUnknown {
                host,
                port,
                fingerprint,
                openssh,
            } => Connection::HostKeyUnknown {
                host: host.clone(),
                port: *port,
                fingerprint: fingerprint.clone(),
                openssh: openssh.clone(),
            },
            ErrorInfo::HostKeyChanged {
                host,
                port,
                fingerprint,
            } => Connection::HostKeyChanged {
                host: host.clone(),
                port: *port,
                fingerprint: fingerprint.clone(),
            },
            ErrorInfo::AuthFailed { user } => Connection::AuthFailed { user: user.clone() },
            ErrorInfo::Incompatible { what } => Connection::Incompatible { what: what.clone() },
            ErrorInfo::Other { message } => Connection::Failed {
                message: message.clone(),
            },
        },
        ConnectionState::Offline => Connection::Offline,
    }
}

fn fleet_state(view: &FleetView) -> FleetState {
    let servers = view
        .hosts
        .iter()
        .map(|h| ServerRow {
            id: h.profile.id.clone(),
            name: h.profile.name.clone(),
            connection: connection(&h.connection),
            herdr_version: h.server_version.clone(),
            stale: h.stale,
            workspaces: h.snapshot.as_ref().map_or(0, |s| s.workspaces.len() as u32),
            panes: h.snapshot.as_ref().map_or(0, |s| s.panes.len() as u32),
            backend: h.backend.map(|b| b.as_str().to_owned()),
            available_backends: h
                .available_backends
                .iter()
                .map(|b| b.as_str().to_owned())
                .collect(),
        })
        .collect();

    let mut workspaces = Vec::new();
    for host in &view.hosts {
        let Some(snapshot) = &host.snapshot else {
            continue;
        };
        // herdr lists workspaces and tabs in the order it shows them. A moved
        // tab keeps its `number` (herdr 0.9.5), so that is no sort key.
        for ws in &snapshot.workspaces {
            let tabs: Vec<_> = snapshot
                .tabs
                .iter()
                .filter(|t| t.workspace_id == ws.workspace_id)
                .collect();
            let first_pane = |tab_id: &str| snapshot.panes.iter().find(|p| p.tab_id == tab_id);
            workspaces.push(WorkspaceRow {
                cwd: snapshot
                    .panes
                    .iter()
                    .find(|p| p.workspace_id == ws.workspace_id)
                    .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone())),
                server_id: host.profile.id.clone(),
                server_name: host.profile.name.clone(),
                workspace_id: ws.workspace_id.clone(),
                label: ws.label.clone(),
                status: ws.agent_status.into(),
                stale: host.stale,
                tabs: tabs
                    .into_iter()
                    .map(|t| TabRow {
                        tab_id: t.tab_id.clone(),
                        label: t.label.clone(),
                        status: t.agent_status.into(),
                        panes: t.pane_count,
                        first_pane_id: first_pane(&t.tab_id).map(|p| p.pane_id.clone()),
                        cwd: first_pane(&t.tab_id)
                            .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone())),
                        agent_type: snapshot
                            .panes
                            .iter()
                            .filter(|p| p.tab_id == t.tab_id)
                            .find_map(|p| p.agent.clone()),
                    })
                    .collect(),
            });
        }
    }

    let agents = view
        .agents
        .iter()
        .map(|a| AgentRow {
            server_id: a.host_id.clone(),
            server_name: a.host_name.clone(),
            pane_id: a.agent.pane_id.clone(),
            workspace_id: a.agent.workspace_id.clone(),
            tab_id: a.agent.tab_id.clone(),
            name: a.agent.display_name().to_owned(),
            agent_type: a.agent.agent.clone(),
            custom_name: a.agent.name.clone(),
            status: a.agent.agent_status.into(),
            status_label: a.agent.status_label(),
            title: a
                .agent
                .title
                .clone()
                .or_else(|| a.agent.terminal_title_stripped.clone()),
            workspace: a.workspace_label.clone(),
            tab: a.tab_label.clone(),
            cwd: a.agent.cwd.clone(),
            stale: a.stale,
        })
        .collect();

    FleetState {
        servers,
        workspaces,
        agents,
    }
}

// ---------------------------------------------------------------------------
// Fleet
// ---------------------------------------------------------------------------

/// Implemented by the app; called on a background thread after every change.
#[uniffi::export(with_foreign)]
pub trait FleetObserver: Send + Sync {
    fn on_update(&self, state: FleetState);
}

#[derive(Default)]
struct Keys {
    /// key id → private key
    keys: HashMap<String, Arc<PrivateKey>>,
    /// server id → how it authenticates
    servers: HashMap<String, ServerAuth>,
}

impl Keys {
    fn auth_for(&self, server_id: &str) -> Option<SshAuth> {
        match self.servers.get(server_id)? {
            ServerAuth::Key { key_id } => self.keys.get(key_id).cloned().map(SshAuth::Key),
            ServerAuth::Password { password } => {
                Some(SshAuth::Password(Zeroizing::new(password.clone())))
            }
        }
    }
}

/// A herdr-core client with the layout state that serializes its writes.
type CoreLayout = (Arc<CoreClient>, Arc<LayoutState>);

#[derive(uniffi::Object)]
pub struct MissionCtrl {
    runtime: tokio::runtime::Runtime,
    fleet: Fleet,
    trust: Arc<TrustStore>,
    keys: Arc<Mutex<Keys>>,
    /// Opens transports, also for checks outside the fleet.
    connector: Connector,
}

#[uniffi::export]
impl MissionCtrl {
    /// Create the core. Load keys and trusted host keys, then add servers.
    #[uniffi::constructor]
    pub fn new(observer: Arc<dyn FleetObserver>) -> Result<Arc<Self>, McError> {
        let trust = Arc::new(TrustStore::new());
        let keys = Arc::new(Mutex::new(Keys::default()));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("mc-core")
            .build()
            .map_err(|e| failed(e.to_string()))?;

        let verifier = trust.clone();
        let key_store = keys.clone();
        let connector: Connector = Arc::new(move |profile: HostProfile| {
            let verifier = verifier.clone();
            let auth = key_store
                .lock()
                .expect("key store lock")
                .auth_for(&profile.id);
            Box::pin(async move {
                let auth = auth.ok_or_else(|| {
                    mc_core::Error::Connect("no SSH key or password for this server".into())
                })?;
                let target = SshTarget {
                    host: profile.host.clone(),
                    port: profile.port,
                    user: profile.user.clone(),
                };
                let transport = SshTransport::connect(target, auth, verifier).await?;
                Ok(Transport::Ssh(transport))
            })
        });

        let fleet = {
            let _guard = runtime.enter();
            Fleet::new(connector.clone())
        };
        let mut updates = fleet.subscribe();
        runtime.spawn(async move {
            while updates.changed().await.is_ok() {
                let state = fleet_state(&updates.borrow_and_update());
                // An exception in the app's callback arrives as a panic; it
                // must not end the updates for good.
                let delivered =
                    std::panic::catch_unwind(AssertUnwindSafe(|| observer.on_update(state)));
                if delivered.is_err() {
                    tracing::error!("fleet observer failed");
                }
            }
        });
        Ok(Arc::new(Self {
            runtime,
            fleet,
            trust,
            keys,
            connector,
        }))
    }

    // -- Keys and trust --------------------------------------------------------

    /// Make a private key available under `key_id` for servers to use.
    pub fn add_key(&self, key_id: String, private_openssh: String) -> Result<(), McError> {
        let key = parse_private_key(&private_openssh)?;
        if is_hardware_key(&key) {
            return Err(failed(
                "FIDO keys need the security key and are not supported yet",
            ));
        }
        self.keys
            .lock()
            .expect("key store lock")
            .keys
            .insert(key_id, Arc::new(key));
        Ok(())
    }

    pub fn remove_key(&self, key_id: String) {
        self.keys
            .lock()
            .expect("key store lock")
            .keys
            .remove(&key_id);
    }

    /// Mark a host key as trusted (on startup for stored keys, or after the
    /// user confirmed a fingerprint). Call [`retry`](Self::retry) afterwards.
    pub fn trust_host_key(
        &self,
        host: String,
        port: u16,
        public_openssh: String,
    ) -> Result<(), McError> {
        self.trust
            .trust(&host, port, parse_public_key(&public_openssh)?);
        Ok(())
    }

    /// Forget all trusted keys of a host (e.g. before accepting a changed key).
    /// Forget the trusted keys of `host:port` and drop its live connections, so
    /// the next connection asks again.
    pub fn forget_host_keys(&self, host: String, port: u16) {
        self.trust.forget(&host, port);
        for h in self
            .fleet
            .hosts()
            .into_iter()
            .filter(|h| h.is_at(&host, port))
        {
            h.reconnect();
        }
    }

    // -- Servers ----------------------------------------------------------------

    /// Add or replace a server and start connecting.
    pub fn add_server(&self, config: ServerConfig) {
        self.keys
            .lock()
            .expect("key store lock")
            .servers
            .insert(config.id.clone(), config.auth.clone());
        let _guard = self.runtime.enter();
        self.fleet.add_host(HostProfile {
            id: config.id,
            name: config.name,
            host: config.host,
            port: config.port,
            user: config.user,
            session: config.session,
            socket_path: config.socket_path.filter(|p| !p.trim().is_empty()),
            backend: backend_of(config.backend.as_deref()),
            edit_layout: config.edit_layout == Some(true),
        });
    }

    /// Which herdr servers a server has: "tui", "core", both (TUI first) or
    /// none. "core" also when herdr-core is installed but not running
    /// (`running` false): only a profile set to "core" starts it, "auto"
    /// never does. The form must ask before saving such a server as "core"
    /// (saving it starts a core the user may not want, e.g. while their TUI
    /// is only down for a moment). For the
    /// add-server form, before the server is added; nothing is started. An
    /// untrusted host key fails with [`McError::HostKeyUnknown`] /
    /// [`McError::HostKeyChanged`], like a connection.
    pub async fn detect_backends(
        &self,
        config: ServerConfig,
    ) -> Result<Vec<DetectedBackend>, McError> {
        // The probe's credentials go in under an id of their own and come out
        // again before anything is awaited: the connector reads them when it
        // is called. The servers' own entries are never touched, so a
        // concurrent add, a second probe or a cancelled one cannot mix them up.
        static PROBES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let probe_id = format!(
            "detect:{}",
            PROBES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let profile = HostProfile {
            id: probe_id.clone(),
            name: config.name,
            host: config.host,
            port: config.port,
            user: config.user,
            session: config.session,
            socket_path: config.socket_path.filter(|p| !p.trim().is_empty()),
            backend: Backend::Auto,
            edit_layout: false,
        };
        let transport = {
            let mut keys = self.keys.lock().expect("key store lock");
            keys.servers.insert(probe_id.clone(), config.auth);
            drop(keys);
            let transport = (self.connector)(profile.clone());
            self.keys
                .lock()
                .expect("key store lock")
                .servers
                .remove(&probe_id);
            transport
        };
        let found = self
            .run(async move {
                let transport = Arc::new(transport.await?);
                let found = mc_core::detect_backends(
                    &transport,
                    profile.session.as_deref(),
                    profile.socket_path.as_deref(),
                )
                .await;
                transport.close().await;
                Ok(found)
            })
            .await?;
        Ok(found
            .into_iter()
            .map(|d| DetectedBackend {
                backend: d.backend.as_str().to_owned(),
                running: d.running,
                unclear: d.unclear,
            })
            .collect())
    }

    pub fn remove_server(&self, id: String) {
        self.keys
            .lock()
            .expect("key store lock")
            .servers
            .remove(&id);
        let _guard = self.runtime.enter();
        self.fleet.remove_host(&id);
    }

    /// Reconnect a server now (after trusting its key or fixing access).
    pub fn retry(&self, id: String) {
        if let Ok(host) = self.fleet.host(&id) {
            host.retry();
        }
    }

    /// Close a server's connection and connect anew, choosing its backend
    /// again: for "auto" the herdr TUI comes first, so this switches back to
    /// it when it runs again. [`Self::retry`] only re-syncs a live connection.
    pub fn reconnect(&self, id: String) {
        if let Ok(host) = self.fleet.host(&id) {
            host.reconnect();
        }
    }

    /// Reconnect every server that is not online, e.g. when the app returns
    /// to the foreground. Online servers just refresh.
    pub fn retry_all(&self) {
        for host in self.fleet.hosts() {
            if host.is_online() {
                host.refresh();
            } else {
                host.retry();
            }
        }
    }

    /// Current state, e.g. for the first frame before the observer fires.
    pub fn state(&self) -> FleetState {
        fleet_state(&self.fleet.view())
    }

    // -- Pane and workspace actions (async: Kotlin `suspend`) ----------------

    /// Last `lines` lines of a pane including scrollback. With `ansi` the
    /// text keeps SGR colour codes for the terminal view.
    pub async fn read_pane(
        &self,
        server_id: String,
        pane_id: String,
        lines: u32,
        ansi: bool,
    ) -> Result<PaneText, McError> {
        let client = self.server(&server_id)?;
        let read = self
            .run(async move { client.read_pane_as(&pane_id, lines, ansi).await })
            .await?;
        Ok(read.into())
    }

    /// What a pane shows right now (no scrollback). Never makes herdr scroll a
    /// resting full-screen agent, so it is safe to poll.
    pub async fn read_screen(
        &self,
        server_id: String,
        pane_id: String,
        ansi: bool,
    ) -> Result<PaneText, McError> {
        let client = self.server(&server_id)?;
        let read = self
            .run(async move { client.read_visible(&pane_id, ansi).await })
            .await?;
        Ok(read.into())
    }

    /// Type text into a pane, optionally pressing Enter.
    pub async fn send_text(
        &self,
        server_id: String,
        pane_id: String,
        text: String,
        enter: bool,
    ) -> Result<(), McError> {
        let client = self.server(&server_id)?;
        self.run(async move {
            if enter {
                client.send_line(&pane_id, &text).await
            } else {
                client.send_text(&pane_id, &text).await
            }
        })
        .await
    }

    /// Send herdr key combos such as `up`, `down`, `enter`, `esc`, `ctrl+c`.
    pub async fn send_keys(
        &self,
        server_id: String,
        pane_id: String,
        keys: Vec<String>,
    ) -> Result<(), McError> {
        let client = self.server(&server_id)?;
        self.run(async move {
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            client.send_keys(&pane_id, &keys).await
        })
        .await
    }

    /// The pane is on screen now with the app in front: a look starts, also
    /// while the server is not connected (its turns are then marked once it
    /// is). Synchronous, so it is ordered with [`Self::forget_seen`].
    pub fn show_seen(&self, server_id: String, pane_id: String) {
        if let Ok(host) = self.fleet.host(&server_id) {
            host.show_seen(&pane_id);
        }
    }

    /// The user has looked at this pane's latest result. On herdr-core this
    /// clears "done" for every client (shared label `herdr.seen`); on the herdr
    /// TUI, which tracks it itself, it does nothing. Only within a look
    /// started with [`Self::show_seen`]: after the pane left the screen it
    /// does nothing.
    pub async fn mark_seen(&self, server_id: String, pane_id: String) -> Result<(), McError> {
        let Some(generation) = self.fleet.host(&server_id)?.seen_look(&pane_id) else {
            return Ok(());
        };
        let client = self.server(&server_id)?;
        // herdr-core's `labels` event brings the new state; the herdr TUI has
        // nothing to mark.
        self.run(async move { client.mark_look(&pane_id, generation).await })
            .await
    }

    /// The pane left the screen (another page, the app in the background):
    /// on herdr-core the next look marks its turn seen again.
    pub fn forget_seen(&self, server_id: String, pane_id: String) {
        if let Ok(host) = self.fleet.host(&server_id) {
            host.forget_seen(&pane_id);
        }
    }

    pub async fn rename_workspace(
        &self,
        server_id: String,
        workspace_id: String,
        label: String,
    ) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            self.run(async move {
                layout_write::rename_space(&core, &state, &workspace_id, &label).await
            })
            .await?;
            self.refresh(&server_id);
            return Ok(());
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.rename_workspace(&workspace_id, &label).await })
            .await
    }

    pub async fn rename_tab(
        &self,
        server_id: String,
        tab_id: String,
        label: String,
    ) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            self.run(async move { layout_write::rename_tab(&core, &state, &tab_id, &label).await })
                .await?;
            self.refresh(&server_id);
            return Ok(());
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.rename_tab(&tab_id, &label).await })
            .await
    }

    /// Home directory and Git branch/repository of each directory in `dirs`.
    pub async fn dir_info(
        &self,
        server_id: String,
        dirs: Vec<String>,
    ) -> Result<DirReport, McError> {
        let client = self.server(&server_id)?;
        let report = self
            .run(async move { client.transport().dir_info(&dirs).await })
            .await?;
        Ok(DirReport {
            home: report.home,
            dirs: report
                .dirs
                .into_iter()
                .map(|d| DirInfo {
                    dir: d.dir,
                    branch: d.branch,
                    repo: d.repo,
                })
                .collect(),
        })
    }

    /// A plan an agent wrote, from the path on its screen (`~/.claude…/plans/<name>.md`).
    pub async fn read_plan(&self, server_id: String, path: String) -> Result<String, McError> {
        let client = self.server(&server_id)?;
        self.run(async move { client.transport().read_plan(&path).await })
            .await
    }

    /// An image an agent named on its screen (absolute, `~/…`, or relative to `cwd`, the
    /// pane's working directory); larger files and other paths are not read. With `known`
    /// (the stamp of the copy the app has), an unchanged file is not sent again.
    pub async fn read_image(
        &self,
        server_id: String,
        path: String,
        cwd: Option<String>,
        known: Option<String>,
    ) -> Result<ImageRead, McError> {
        let client = self.server(&server_id)?;
        let file = self
            .run(async move {
                client
                    .transport()
                    .read_image(&path, cwd.as_deref(), known.as_deref())
                    .await
            })
            .await?;
        use mc_core::transport::ImageFile;
        Ok(match file {
            ImageFile::Data { data, stamp } => ImageRead::Data { data, stamp },
            ImageFile::Unchanged => ImageRead::Unchanged,
            ImageFile::Missing => ImageRead::Missing,
            ImageFile::TooLarge(size) => ImageRead::TooLarge { size },
        })
    }

    /// Store a file (e.g. a pasted screenshot) on the server under
    /// `~/.cache/missionctrl/uploads/` and return its absolute path.
    pub async fn upload_file(
        &self,
        server_id: String,
        file_name: String,
        data: Vec<u8>,
    ) -> Result<String, McError> {
        let client = self.server(&server_id)?;
        self.run(async move { client.transport().upload(&file_name, &data).await })
            .await
    }

    /// Create a tab (without changing focus on other clients) and start
    /// `start` in it. Returns the new pane id.
    pub async fn create_tab(
        &self,
        server_id: String,
        workspace_id: String,
        label: Option<String>,
        cwd: Option<String>,
        start: TabStart,
    ) -> Result<String, McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let tab_name = nonempty(&label)
                .map(str::to_owned)
                .or_else(|| start_label(&start))
                .unwrap_or_default();
            return self
                .core_create(
                    &server_id,
                    core,
                    state,
                    Some(workspace_id),
                    String::new(),
                    tab_name,
                    cwd,
                    start,
                )
                .await;
        }
        let client = self.client(&server_id)?;
        let pane = self
            .run(async move {
                let cwd = client
                    .transport()
                    .resolve_dir(cwd.as_deref().unwrap_or(""))
                    .await?;
                let label = nonempty(&label)
                    .map(str::to_owned)
                    .or_else(|| start_label(&start));
                let (_, pane) = client
                    .create_tab(&workspace_id, label.as_deref(), Some(&cwd))
                    .await?;
                run_start(&client, &pane, &start).await?;
                Ok(pane)
            })
            .await?;
        self.refresh(&server_id);
        Ok(pane)
    }

    /// Create a workspace (without changing focus) and start `start` in it.
    /// Returns the new pane id.
    pub async fn create_workspace(
        &self,
        server_id: String,
        label: Option<String>,
        cwd: Option<String>,
        start: TabStart,
    ) -> Result<String, McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let space_name = nonempty(&label).map(str::to_owned).unwrap_or_default();
            let tab_name = start_label(&start).unwrap_or_default();
            return self
                .core_create(
                    &server_id, core, state, None, space_name, tab_name, cwd, start,
                )
                .await;
        }
        let client = self.client(&server_id)?;
        let pane = self
            .run(async move {
                let cwd = client
                    .transport()
                    .resolve_dir(cwd.as_deref().unwrap_or(""))
                    .await?;
                let (_, tab, pane) = client
                    .create_workspace(nonempty(&label), Some(&cwd))
                    .await?;
                // herdr takes no name for the first tab and numbers it "1": name it like a
                // new tab without a name. Only cosmetic, so a failure does not stop the start.
                if let Some(tab_label) = start_label(&start) {
                    let _ = client.rename_tab(&tab, &tab_label).await;
                }
                run_start(&client, &pane, &start).await?;
                Ok(pane)
            })
            .await?;
        self.refresh(&server_id);
        Ok(pane)
    }

    /// Move a tab to `index` (0-based) within its workspace, on the server.
    /// herdr-core takes `before_id` instead: the tab it goes before in the
    /// list as shown (`None`: to the end), so a change made elsewhere
    /// meanwhile does not shift the target.
    pub async fn move_tab(
        &self,
        server_id: String,
        tab_id: String,
        index: u32,
        before_id: Option<String>,
    ) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let anchor = anchor_of(before_id);
            self.run(async move { layout_write::move_tab(&core, &state, &tab_id, &anchor).await })
                .await?;
            self.refresh(&server_id);
            return Ok(());
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.move_tab(&tab_id, index).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    /// Move a workspace to `index` (0-based) in the list, on the server
    /// (herdr-core: before `before_id`, as for [`Self::move_tab`]).
    pub async fn move_workspace(
        &self,
        server_id: String,
        workspace_id: String,
        index: u32,
        before_id: Option<String>,
    ) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let anchor = anchor_of(before_id);
            self.run(async move {
                layout_write::move_space(&core, &state, &workspace_id, &anchor).await
            })
            .await?;
            self.refresh(&server_id);
            return Ok(());
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.move_workspace(&workspace_id, index).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    pub async fn close_tab(&self, server_id: String, tab_id: String) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let result = self
                .run(async move { layout_write::close(&core, &state, Target::Tab(&tab_id)).await })
                .await;
            self.refresh(&server_id);
            return result;
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.close_tab(&tab_id).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    pub async fn close_workspace(
        &self,
        server_id: String,
        workspace_id: String,
    ) -> Result<(), McError> {
        if let Some((core, state)) = self.core_layout(&server_id)? {
            let result = self
                .run(async move {
                    layout_write::close(&core, &state, Target::Space(&workspace_id)).await
                })
                .await;
            self.refresh(&server_id);
            return result;
        }
        let client = self.client(&server_id)?;
        self.run(async move { client.close_workspace(&workspace_id).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    /// herdr-core: end a terminal that stands in no workspace (the
    /// "Terminals" group). Needs layout editing like the other changes.
    pub async fn close_terminal(&self, server_id: String, pane_id: String) -> Result<(), McError> {
        let Some((core, state)) = self.core_layout(&server_id)? else {
            return Err(failed("only herdr-core has loose terminals"));
        };
        let result = self
            .run(async move { layout_write::close_terminal(&core, &state, &pane_id).await })
            .await;
        self.refresh(&server_id);
        result
    }

    /// Control `pane_id` as a live terminal of `cols × rows` (TUI mode).
    /// While it is open herdr gives the pane this size; closing it (or
    /// dropping the handle) gives the size back.
    pub async fn open_terminal(
        &self,
        server_id: String,
        pane_id: String,
        cols: u16,
        rows: u16,
        observer: Arc<dyn PaneTerminalObserver>,
    ) -> Result<Arc<PaneTerminal>, McError> {
        let sink = Arc::new(TerminalForwarder(observer));
        // Unlike other calls, an opening the app gave up on stops: it would
        // take the terminal's size only to let go of it again. The observer
        // then hears an end all the same, so the app can wait for every
        // opening it started to be over before it opens the next one. The
        // guard exists before anything can fail and moves into the task, so
        // the end is also heard when the task is dropped before its first poll.
        let mut ends = EndIfUnopened(Some(sink.clone()));
        let client = self.server(&server_id)?;
        let work = self.runtime.spawn(async move {
            let opened = client.open_terminal(&pane_id, cols, rows, sink).await;
            if opened.is_ok() {
                // An open session reports its own end.
                ends.disarm();
            }
            opened
        });
        let session = AbortOnDrop(work)
            .await
            .map_err(|e| failed(e.to_string()))?
            .map_err(McError::from)?;
        Ok(Arc::new(PaneTerminal { session }))
    }

    pub fn shutdown(&self) {
        self.fleet.stop_all();
    }
}

// ---------------------------------------------------------------------------
// Live terminal of one pane (TUI mode)
// ---------------------------------------------------------------------------

/// Colours as one number: `-1` default, `0..=255` palette index,
/// `0x1000000 | rgb` true colour.
fn term_color(c: TermColorCore) -> i32 {
    match c {
        TermColorCore::Default => -1,
        TermColorCore::Indexed(i) => i as i32,
        TermColorCore::Rgb(r, g, b) => 0x100_0000 | (r as i32) << 16 | (g as i32) << 8 | b as i32,
    }
}

#[derive(uniffi::Record, Clone)]
pub struct TermRun {
    pub col: u16,
    /// Cells covered; a wide character is a run of its own with width 2.
    pub width: u16,
    pub text: String,
    pub fg: i32,
    pub bg: i32,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(uniffi::Record, Clone)]
pub struct TermLine {
    /// Blank cells with the default style are left out.
    pub runs: Vec<TermRun>,
}

#[derive(uniffi::Record, Clone)]
pub struct TermScreen {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<TermLine>,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    pub seq: u64,
    /// herdr-core: the app only watches (the desktop's size); the next input
    /// takes the terminal over. Always false on the herdr TUI.
    #[uniffi(default)]
    pub watching: bool,
    /// The program asked for application cursor keys (DECCKM).
    #[uniffi(default)]
    pub app_cursor: bool,
    /// herdr-core: a named key is on its way to herdr; input after it waits.
    #[uniffi(default)]
    pub keys_pending: bool,
    /// herdr-core: counts named keys herdr refused (one more per failure).
    #[uniffi(default)]
    pub key_errors: u64,
    /// herdr-core: why the last named key was refused.
    #[uniffi(default)]
    pub key_error: Option<String>,
    /// herdr-core: counts the times typed input was refused because too much
    /// was still waiting to go out (one more per refusal).
    #[uniffi(default)]
    pub input_refused: u64,
}

impl From<Screen> for TermScreen {
    fn from(s: Screen) -> Self {
        TermScreen {
            cols: s.cols,
            rows: s.rows,
            lines: s
                .lines
                .into_iter()
                .map(|runs| TermLine {
                    runs: runs
                        .into_iter()
                        .map(|r| TermRun {
                            col: r.col,
                            width: r.width,
                            text: r.text,
                            fg: term_color(r.style.fg),
                            bg: term_color(r.style.bg),
                            bold: r.style.bold,
                            dim: r.style.dim,
                            italic: r.style.italic,
                            underline: r.style.underline,
                            inverse: r.style.inverse,
                        })
                        .collect(),
                })
                .collect(),
            cursor_row: s.cursor_row,
            cursor_col: s.cursor_col,
            cursor_visible: s.cursor_visible,
            seq: s.seq,
            watching: s.watching,
            app_cursor: s.app_cursor,
            keys_pending: s.keys_pending,
            key_errors: s.key_errors,
            key_error: s.key_error,
            input_refused: s.input_refused,
        }
    }
}

/// Implemented by the app; called on a background thread, at most about 30
/// times a second.
#[uniffi::export(with_foreign)]
pub trait PaneTerminalObserver: Send + Sync {
    fn on_screen(&self, screen: TermScreen);
    /// The session is over. `reason` is herdr's or the server's message, or
    /// `None` when it was released normally.
    fn on_ended(&self, reason: Option<String>);
}

struct TerminalForwarder(Arc<dyn PaneTerminalObserver>);

/// Tells the observer its opening is over when dropped before the session
/// opened (failed, or aborted because the app gave up on it).
struct EndIfUnopened(Option<Arc<TerminalForwarder>>);

impl EndIfUnopened {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for EndIfUnopened {
    fn drop(&mut self) {
        if let Some(sink) = self.0.take() {
            sink.ended(Ended::Released);
        }
    }
}

/// A task that is aborted when the call waiting for it is dropped.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl TerminalSink for TerminalForwarder {
    fn screen(&self, screen: Screen) {
        let screen = screen.into();
        if std::panic::catch_unwind(AssertUnwindSafe(|| self.0.on_screen(screen))).is_err() {
            tracing::error!("terminal observer failed");
        }
    }

    fn ended(&self, why: Ended) {
        let reason = match why {
            Ended::Released => None,
            Ended::Closed(reason) => Some(reason),
        };
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| self.0.on_ended(reason)));
    }
}

/// An open terminal session. Dropping it releases the pane.
#[derive(uniffi::Object)]
pub struct PaneTerminal {
    session: TerminalSession,
}

#[uniffi::export]
impl PaneTerminal {
    /// Bytes as a terminal sends them: UTF-8 text, `\r` for Enter, `\u007f`
    /// for Backspace, escape sequences for arrows and function keys.
    pub fn input(&self, bytes: Vec<u8>) {
        self.session.input(bytes);
    }

    /// Key names without fixed bytes (`ctrl+enter`, `esc` …), encoded by
    /// herdr-core for the program's keyboard mode after the input before
    /// them. herdr-core only; the TUI backend drops them.
    pub fn keys(&self, names: Vec<String>) {
        self.session.keys(names);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.session.resize(cols, rows);
    }

    /// Like a mouse wheel: the application gets it if it asked for the
    /// mouse, otherwise herdr scrolls the pane's history.
    pub fn scroll(&self, up: bool, lines: u16) {
        let direction = if up {
            ScrollDirection::Up
        } else {
            ScrollDirection::Down
        };
        self.session.scroll(direction, lines);
    }

    /// Give the pane back to herdr (its size returns to herdr's layout).
    /// Named `release` because Kotlin objects already have `close()`.
    pub fn release(&self) {
        self.session.release();
    }
}

/// Where a herdr-core move goes: before `before_id`, or to the end.
fn anchor_of(before_id: Option<String>) -> Anchor {
    match before_id.filter(|id| !id.is_empty()) {
        Some(id) => Anchor::Before(id),
        None => Anchor::End,
    }
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// Name for a tab nobody named: what runs in it ("Claude", "Codex", "Shell");
/// none for a command, there herdr's number stays.
fn start_label(start: &TabStart) -> Option<String> {
    match start {
        TabStart::Shell => Some("Shell".to_owned()),
        TabStart::Agent { kind } => {
            let mut chars = kind.trim().chars();
            let first = chars.next()?;
            Some(first.to_uppercase().chain(chars).collect())
        }
        TabStart::Command { .. } => None,
    }
}

async fn run_start(
    client: &HerdrClient,
    pane: &str,
    start: &TabStart,
) -> Result<(), mc_core::Error> {
    match start {
        TabStart::Shell => Ok(()),
        TabStart::Agent { kind } => client.start_agent_free_name(pane, kind).await.map(drop),
        TabStart::Command { command } => {
            client.send_text(pane, command).await?;
            client.send_keys(pane, &["enter"]).await
        }
    }
}

impl MissionCtrl {
    fn refresh(&self, server_id: &str) {
        if let Ok(host) = self.fleet.host(server_id) {
            host.refresh();
        }
    }

    /// Runs `work` on the core's own runtime (where the connections live)
    /// and waits for it from whatever executor polls the app's call.
    async fn run<T, F>(&self, work: F) -> Result<T, McError>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, mc_core::Error>> + Send + 'static,
    {
        self.runtime
            .spawn(work)
            .await
            .map_err(|e| failed(e.to_string()))?
            .map_err(McError::from)
    }

    /// For a herdr-core server: its client and layout state ([`CoreLayout`]), when layout
    /// editing is on (else an error before anything happens). `None` for a
    /// herdr TUI server.
    fn core_layout(&self, server_id: &str) -> Result<Option<CoreLayout>, McError> {
        let host = self.fleet.host(server_id)?;
        let core = match host.server()? {
            ServerClient::Tui(_) => return Ok(None),
            ServerClient::Core(core) => core,
        };
        // The core refuses too; checking here fails before anything is read.
        let layout = host.layout();
        if !layout.allowed() {
            return Err(McError::from(mc_core::Error::Unsupported {
                backend: "herdr-core".into(),
                what: "changing the layout (turn on layout editing for this server)".into(),
            }));
        }
        Ok(Some((core, layout)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn core_create(
        &self,
        server_id: &str,
        core: Arc<CoreClient>,
        state: Arc<LayoutState>,
        space: Option<String>,
        space_name: String,
        tab_name: String,
        cwd: Option<String>,
        start: TabStart,
    ) -> Result<String, McError> {
        let created = self
            .run(async move {
                let cwd = core
                    .transport()
                    .resolve_dir(cwd.as_deref().unwrap_or(""))
                    .await?;
                let start = match start {
                    TabStart::Shell => Start::Shell,
                    TabStart::Agent { kind } => Start::Program(vec![kind]),
                    TabStart::Command { command } => Start::Line(command),
                };
                layout_write::create(
                    &core,
                    &state,
                    NewTab {
                        space,
                        space_name,
                        tab_name,
                        cwd,
                        start,
                        size: layout_write::DEFAULT_SIZE,
                    },
                )
                .await
            })
            .await;
        self.refresh(server_id);
        Ok(created?.address)
    }

    fn client(&self, server_id: &str) -> Result<Arc<HerdrClient>, McError> {
        Ok(self.fleet.host(server_id)?.client()?)
    }

    fn server(&self, server_id: &str) -> Result<ServerClient, McError> {
        Ok(self.fleet.host(server_id)?.server()?)
    }
}

// ---------------------------------------------------------------------------
// Speech recognition on the device
// ---------------------------------------------------------------------------

/// A whisper.cpp model loaded from a ggml file on the device. Loading and
/// recognition block; call them from a background thread.
#[derive(uniffi::Object)]
pub struct SpeechModel {
    inner: mc_voice::Whisper,
}

#[uniffi::export]
impl SpeechModel {
    #[uniffi::constructor]
    pub fn load(path: String) -> Result<Arc<Self>, McError> {
        let inner = mc_voice::Whisper::load(&path).map_err(|e| failed(e.to_string()))?;
        Ok(Arc::new(Self { inner }))
    }

    /// Text of `pcm` (16 kHz mono, little-endian i16). `language` like `de-CH`
    /// (empty = detect), `prompt` = words to expect.
    pub fn transcribe(
        &self,
        pcm: Vec<u8>,
        language: String,
        prompt: String,
    ) -> Result<String, McError> {
        self.inner
            .transcribe(&pcm, &language, &prompt)
            .map_err(|e| failed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_the_server_form_handles_stay_apart() {
        let unknown = McError::from(mc_core::Error::HostKeyUnknown {
            host: "example.com".into(),
            port: 22,
            fingerprint: "SHA256:abc".into(),
            openssh: "ssh-ed25519 AAAA".into(),
        });
        assert!(matches!(unknown, McError::HostKeyUnknown { port: 22, .. }));
        let changed = McError::from(mc_core::Error::HostKeyChanged {
            host: "example.com".into(),
            port: 22,
            fingerprint: "SHA256:def".into(),
        });
        assert!(matches!(changed, McError::HostKeyChanged { .. }));
        let auth = McError::from(mc_core::Error::AuthFailed { user: "dev".into() });
        assert!(matches!(auth, McError::AuthFailed { .. }));
        assert!(matches!(
            McError::from(mc_core::Error::Timeout),
            McError::Timeout
        ));
    }

    #[test]
    fn unnamed_tab_is_named_after_what_runs_in_it() {
        let agent = |kind: &str| TabStart::Agent {
            kind: kind.to_owned(),
        };
        assert_eq!(start_label(&agent("claude")).as_deref(), Some("Claude"));
        assert_eq!(start_label(&agent("codex")).as_deref(), Some("Codex"));
        assert_eq!(start_label(&agent(" ")), None);
        assert_eq!(start_label(&TabStart::Shell).as_deref(), Some("Shell"));
        let command = TabStart::Command {
            command: "htop".to_owned(),
        };
        assert_eq!(start_label(&command), None);
    }
}

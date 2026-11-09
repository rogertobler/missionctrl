//! UniFFI bindings: the MissionCtrl core as seen from Kotlin and Swift.
//!
//! The app owns persistence (servers, keys, trusted host keys) and passes it
//! in; the core owns connections and live state and reports every change
//! through [`FleetObserver`]. Blocking methods must be called from a
//! background thread, never from the observer callback.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

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
use mc_core::{ConnectionState, Connector, ErrorInfo, Fleet, FleetView, HostProfile};

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
}

impl From<mc_core::Error> for McError {
    fn from(e: mc_core::Error) -> Self {
        match e {
            mc_core::Error::NotConnected => McError::NotConnected,
            mc_core::Error::Timeout => McError::Timeout,
            mc_core::Error::NoSuchDirectory { path } => McError::NoSuchDirectory { path },
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
        } => Connection::Reconnecting {
            attempt: *attempt,
            error: error.clone(),
            retry_in_ms: *retry_in_ms,
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

#[derive(uniffi::Object)]
pub struct MissionCtrl {
    runtime: tokio::runtime::Runtime,
    fleet: Fleet,
    trust: Arc<TrustStore>,
    keys: Arc<Mutex<Keys>>,
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
            Fleet::new(connector)
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
            socket_path: None,
        });
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
        let client = self.client(&server_id)?;
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
        let client = self.client(&server_id)?;
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
        let client = self.client(&server_id)?;
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
        let client = self.client(&server_id)?;
        self.run(async move {
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            client.send_keys(&pane_id, &keys).await
        })
        .await
    }

    pub async fn rename_workspace(
        &self,
        server_id: String,
        workspace_id: String,
        label: String,
    ) -> Result<(), McError> {
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
        let client = self.client(&server_id)?;
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
        let client = self.client(&server_id)?;
        self.run(async move { client.transport().read_plan(&path).await })
            .await
    }

    /// Store a file (e.g. a pasted screenshot) on the server under
    /// `~/.cache/missionctrl/uploads/` and return its absolute path.
    pub async fn upload_file(
        &self,
        server_id: String,
        file_name: String,
        data: Vec<u8>,
    ) -> Result<String, McError> {
        let client = self.client(&server_id)?;
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
    pub async fn move_tab(
        &self,
        server_id: String,
        tab_id: String,
        index: u32,
    ) -> Result<(), McError> {
        let client = self.client(&server_id)?;
        self.run(async move { client.move_tab(&tab_id, index).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    /// Move a workspace to `index` (0-based) in the list, on the server.
    pub async fn move_workspace(
        &self,
        server_id: String,
        workspace_id: String,
        index: u32,
    ) -> Result<(), McError> {
        let client = self.client(&server_id)?;
        self.run(async move { client.move_workspace(&workspace_id, index).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
    }

    pub async fn close_tab(&self, server_id: String, tab_id: String) -> Result<(), McError> {
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
        let client = self.client(&server_id)?;
        self.run(async move { client.close_workspace(&workspace_id).await })
            .await?;
        self.refresh(&server_id);
        Ok(())
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
        let client = self.client(&server_id)?;
        let sink = Arc::new(TerminalForwarder(observer));
        let session = self
            .run(async move {
                TerminalSession::open(
                    client.transport(),
                    client.socket_path(),
                    &pane_id,
                    cols,
                    rows,
                    sink,
                )
                .await
            })
            .await?;
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

    fn client(&self, server_id: &str) -> Result<Arc<HerdrClient>, McError> {
        Ok(self.fleet.host(server_id)?.client()?)
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

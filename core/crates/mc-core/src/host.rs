//! Per-host runtime: connection lifecycle, reconnect, and live session state.
//!
//! Synchronisation follows herdr's documented recipe (socket API docs, "Event
//! subscriptions"): subscribe first, then take `session.snapshot` as the
//! authoritative state. Events only invalidate the cache and trigger another
//! snapshot; they are never replayed onto it. On `events_lost` the runtime
//! resubscribes and resyncs.
//!
//! Agent status changes are only delivered per pane
//! (`pane.agent_status_changed` requires a `pane_id`), so the subscription is
//! rebuilt whenever the set of panes changes.
//!
//! A herdr-core server ([`crate::hc`]) is synced differently: its
//! `events.subscribe` starts with the full list of terminals and then sends
//! every change in order, which the runtime applies to its copy; the Herdr
//! app's layout document is read again when a `doc` event arrives.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::{Instant, sleep_until};

use crate::detect;
use crate::error::{Error, Result};
use crate::hc::model::{DocState, Hello, Terminal};
use crate::hc::write::LayoutState;
use crate::hc::{CoreClient, Notification, SeenMarks, snapshot as hc_snapshot};
use crate::herdr::model::SessionSnapshot;
use crate::herdr::{HerdrClient, Subscription};
use crate::profile::{Backend, HostProfile};
use crate::server::ServerClient;
use crate::transport::Transport;

/// Lifecycle events herdr can deliver without a pane filter.
const LIFECYCLE_EVENTS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

/// Coalesce bursts of events into one snapshot.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
/// Health check for quiet connections.
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Upper bound for transport plus socket discovery.
const CONNECT_DEADLINE: Duration = Duration::from_secs(30);
/// Retries when a pane vanishes between snapshot and subscribe.
const SUBSCRIBE_ATTEMPTS: usize = 3;
/// herdr-core sends a heartbeat every 15 s; three missing mean a dead line.
const CORE_SILENCE_LIMIT: Duration = Duration::from_secs(45);

/// A pending check whether the other backend runs on the machine too:
/// `Some(true)` it does, `Some(false)` surely not, `None` unclear.
type Probe = tokio::task::JoinHandle<Option<bool>>;
/// How often `auto` on herdr-core looks whether the TUI runs again.
#[cfg(not(test))]
const TUI_HINT_EVERY: Duration = Duration::from_secs(60);
#[cfg(test)]
const TUI_HINT_EVERY: Duration = Duration::from_millis(200);

/// Opens a transport for a profile. The app supplies this (SSH with its own
/// key store and host-key trust); tests can connect to a local socket.
pub type Connector = Arc<
    dyn Fn(HostProfile) -> Pin<Box<dyn Future<Output = Result<Transport>> + Send>> + Send + Sync,
>;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Connecting,
    Online,
    /// Lost or failed; retrying automatically.
    Reconnecting {
        attempt: u32,
        error: String,
        retry_in_ms: u64,
        /// No herdr server answered where the profile looks (nothing is
        /// running or installed there), as opposed to one that failed.
        absent: bool,
    },
    /// Needs the user (host key, authentication). No automatic retry.
    ActionRequired {
        error: ErrorInfo,
    },
    /// Stopped by the app.
    Offline,
}

/// Serializable error description for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrorInfo {
    HostKeyUnknown {
        host: String,
        port: u16,
        fingerprint: String,
        openssh: String,
    },
    HostKeyChanged {
        host: String,
        port: u16,
        fingerprint: String,
    },
    AuthFailed {
        user: String,
    },
    /// The server needs a newer MissionCtrl ([`Error::Incompatible`]).
    Incompatible {
        what: String,
    },
    Other {
        message: String,
    },
}

impl From<&Error> for ErrorInfo {
    fn from(e: &Error) -> Self {
        match e {
            Error::HostKeyUnknown {
                host,
                port,
                fingerprint,
                openssh,
            } => ErrorInfo::HostKeyUnknown {
                host: host.clone(),
                port: *port,
                fingerprint: fingerprint.clone(),
                openssh: openssh.clone(),
            },
            Error::HostKeyChanged {
                host,
                port,
                fingerprint,
            } => ErrorInfo::HostKeyChanged {
                host: host.clone(),
                port: *port,
                fingerprint: fingerprint.clone(),
            },
            Error::AuthFailed { user } => ErrorInfo::AuthFailed { user: user.clone() },
            Error::Incompatible { what } => ErrorInfo::Incompatible { what: what.clone() },
            other => ErrorInfo::Other {
                message: other.to_string(),
            },
        }
    }
}

/// Errors only the user can resolve: retrying on a timer would not help.
/// An incompatible server (a newer herdr-core generation) waits for a
/// MissionCtrl update; Retry tries again.
fn needs_user(e: &Error) -> bool {
    matches!(
        e,
        Error::HostKeyUnknown { .. }
            | Error::HostKeyChanged { .. }
            | Error::AuthFailed { .. }
            | Error::Incompatible { .. }
    )
}

/// Everything the UI needs to render one host.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HostView {
    pub profile: HostProfile,
    pub connection: ConnectionState,
    pub server_version: Option<String>,
    pub protocol: Option<u32>,
    /// Last known session state. Kept while reconnecting.
    pub snapshot: Option<Arc<SessionSnapshot>>,
    /// True when `snapshot` may be outdated (not Online or resync pending).
    pub stale: bool,
    /// Unix milliseconds of the last successful snapshot. Changing only this
    /// does not notify subscribers.
    pub synced_at_ms: Option<u64>,
    /// The server this connection talks to ([`Backend::Tui`] or
    /// [`Backend::Core`]); `None` until connected.
    pub backend: Option<Backend>,
    /// Every server that answered on this machine at the last connect. With
    /// two and the profile on [`Backend::Auto`], the app may ask which one.
    pub available_backends: Vec<Backend>,
}

enum Command {
    Refresh,
    Retry,
    /// Drop the connection and connect anew, with a fresh host-key check.
    Reconnect,
    Stop,
}

/// Handle to a running host. Cheap to clone.
#[derive(Clone)]
pub struct HostHandle {
    id: String,
    commands: mpsc::UnboundedSender<Command>,
    view: watch::Receiver<HostView>,
    client: Arc<RwLock<Option<Arc<HerdrClient>>>>,
    core_client: Arc<RwLock<Option<Arc<CoreClient>>>>,
    /// herdr-core turns marked seen while the user looks; outlives reconnects.
    seen_marks: Arc<SeenMarks>,
    /// herdr-core workspace and tab changes; outlives reconnects.
    layout: Arc<LayoutState>,
}

impl HostHandle {
    /// Start the runtime task for `profile`. `changed` is notified on every
    /// view update (used by the fleet to re-aggregate).
    pub fn spawn(profile: HostProfile, connector: Connector, changed: Arc<Notify>) -> Self {
        Self::spawn_with(profile, connector, changed, None)
    }

    /// [`Self::spawn`], going on with the layout state of the host this one
    /// replaces (`layout`), so starts still in flight there and their later
    /// looks stay with the machine.
    pub fn spawn_with(
        profile: HostProfile,
        connector: Connector,
        changed: Arc<Notify>,
        layout: Option<Arc<LayoutState>>,
    ) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (view_tx, view_rx) = watch::channel(HostView {
            profile: profile.clone(),
            connection: ConnectionState::Connecting,
            server_version: None,
            protocol: None,
            snapshot: None,
            stale: true,
            synced_at_ms: None,
            backend: None,
            available_backends: Vec::new(),
        });
        let client = Arc::new(RwLock::new(None));
        let core_client = Arc::new(RwLock::new(None));
        let seen_marks: Arc<SeenMarks> = Arc::default();
        let layout = match layout {
            Some(layout) => {
                layout.attach(core_client.clone());
                layout.allow(profile.edit_layout);
                layout
            }
            None => Arc::new(LayoutState::for_host(
                core_client.clone(),
                profile.edit_layout,
            )),
        };
        let runtime = Runtime {
            layout: layout.clone(),
            profile: profile.clone(),
            connector,
            commands: commands_rx,
            view: view_tx,
            changed,
            client: client.clone(),
            core_client: core_client.clone(),
            core_seen: Default::default(),
            seen_marks: seen_marks.clone(),
            resolved: None,
            probe: None,
            probed: None,
            probe_transport: None,
        };
        tokio::spawn(runtime.run());
        Self {
            id: profile.id,
            commands: commands_tx,
            view: view_rx,
            client,
            core_client,
            seen_marks,
            layout,
        }
    }

    /// What this host keeps about workspace and tab changes on herdr-core.
    pub fn layout(&self) -> Arc<LayoutState> {
        self.layout.clone()
    }

    /// The pane `pane_id` is on screen now: a look starts ([`SeenMarks`]),
    /// whether or not the host is connected. On herdr-core its turns are
    /// marked seen while the look lasts, once connected; on the herdr TUI
    /// nothing comes of it. Returns the look's generation.
    pub fn show_seen(&self, pane_id: &str) -> u64 {
        self.seen_marks.show(pane_id)
    }

    /// The generation of the running look at `pane_id`, if any.
    pub fn seen_look(&self, pane_id: &str) -> Option<u64> {
        self.seen_marks.look(pane_id)
    }

    /// The pane `pane_id` left the screen: on herdr-core, the next look marks
    /// its turn seen again ([`SeenMarks`]).
    pub fn forget_seen(&self, pane_id: &str) {
        self.seen_marks.forget(pane_id);
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn view(&self) -> HostView {
        self.view.borrow().clone()
    }

    pub fn is_online(&self) -> bool {
        self.view.borrow().connection == ConnectionState::Online
    }

    pub fn subscribe(&self) -> watch::Receiver<HostView> {
        self.view.clone()
    }

    /// Connected herdr TUI client for actions on this host. On a herdr-core
    /// server this is [`Error::Unsupported`]: its actions go through
    /// [`Self::core_client`].
    pub fn client(&self) -> Result<Arc<HerdrClient>> {
        if let Some(client) = self.client.read().ok().and_then(|c| c.clone()) {
            return Ok(client);
        }
        if self.core_client().is_ok() {
            return Err(Error::Unsupported {
                backend: "herdr-core".into(),
                what: "this action".into(),
            });
        }
        Err(Error::NotConnected)
    }

    /// Connected herdr-core client, when this host talks to herdr-core.
    pub fn core_client(&self) -> Result<Arc<CoreClient>> {
        self.core_client
            .read()
            .ok()
            .and_then(|c| c.clone())
            .ok_or(Error::NotConnected)
    }

    /// Client for the actions every backend offers (read, send, keys, live
    /// terminal), whichever server this host talks to.
    pub fn server(&self) -> Result<ServerClient> {
        if let Some(client) = self.client.read().ok().and_then(|c| c.clone()) {
            return Ok(ServerClient::Tui(client));
        }
        self.core_client().map(ServerClient::Core)
    }

    /// Ask for a fresh snapshot now.
    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    /// Reconnect immediately, e.g. after the user trusted a host key.
    pub fn retry(&self) {
        let _ = self.commands.send(Command::Retry);
    }

    /// Close a live connection and connect again, e.g. after its host key was
    /// forgotten: [`Self::retry`] only re-syncs a live connection.
    pub fn reconnect(&self) {
        let _ = self.commands.send(Command::Reconnect);
    }

    /// Whether this host is `host:port`.
    pub fn is_at(&self, host: &str, port: u16) -> bool {
        let view = self.view.borrow();
        view.profile.host == host && view.profile.port == port
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }
}

struct Runtime {
    profile: HostProfile,
    connector: Connector,
    commands: mpsc::UnboundedReceiver<Command>,
    view: watch::Sender<HostView>,
    changed: Arc<Notify>,
    client: Arc<RwLock<Option<Arc<HerdrClient>>>>,
    core_client: Arc<RwLock<Option<Arc<CoreClient>>>>,
    /// Turns counted as seen on a herdr-core server without the shared label;
    /// kept across reconnects.
    core_seen: hc_snapshot::LocalSeen,
    seen_marks: Arc<SeenMarks>,
    layout: Arc<LayoutState>,
    /// The backend [`Backend::Auto`] last connected to ([`select_backend`]).
    resolved: Option<Backend>,
    /// Background check for the other backend after an automatic connect.
    probe: Option<Probe>,
    /// What that check found for sure, for the backend connected then; asked
    /// again after a switch or when the last check had no clear answer.
    probed: Option<(Backend, Vec<Backend>)>,
    /// The connection the background check runs on.
    probe_transport: Option<Arc<Transport>>,
}

/// A connected server of either kind.
enum Connected {
    Tui(Arc<HerdrClient>, String, u32),
    Core(Arc<CoreClient>, Hello),
}

/// How a session ended.
enum Exit {
    Stop,
    Lost(Error),
    /// Connecting failed (no session was established). Only here can an
    /// error mean that no herdr is there: a failure of a running session
    /// (an error reply, an answer that does not parse) is not absence.
    Unreachable(Error),
    /// Asked to connect anew ([`Command::Reconnect`]).
    Reconnect,
}

impl Runtime {
    async fn run(mut self) {
        let mut attempt: u32 = 0;
        loop {
            self.update(|v| {
                if attempt == 0 {
                    v.connection = ConnectionState::Connecting;
                }
                v.stale = true;
            });
            let exit = self.session().await;
            self.set_client(None);
            self.set_core_client(None);
            // Still Online means this session synced: start the backoff over.
            if self.view.borrow().connection == ConnectionState::Online {
                attempt = 0;
            }
            let (error, connecting) = match exit {
                Exit::Stop => break,
                Exit::Reconnect => {
                    attempt = 0;
                    continue;
                }
                Exit::Lost(e) => (e, false),
                Exit::Unreachable(e) => (e, true),
            };
            tracing::warn!(host = %self.profile.name, error = %error, "host connection lost");

            if needs_user(&error) {
                let info = ErrorInfo::from(&error);
                self.update(|v| {
                    v.connection = ConnectionState::ActionRequired { error: info };
                    v.stale = true;
                });
                attempt = 0;
                // Wait for the user: Retry reconnects, Stop ends.
                loop {
                    match self.commands.recv().await {
                        Some(Command::Retry | Command::Reconnect) => break,
                        Some(Command::Refresh) => continue,
                        Some(Command::Stop) | None => return self.finish(),
                    }
                }
                continue;
            }

            attempt += 1;
            let delay = backoff(attempt);
            let message = error.to_string();
            let absent = connecting && error.is_absent();
            self.update(|v| {
                v.connection = ConnectionState::Reconnecting {
                    attempt,
                    error: message,
                    retry_in_ms: delay.as_millis() as u64,
                    absent,
                };
                v.stale = true;
            });
            let deadline = Instant::now() + delay;
            loop {
                tokio::select! {
                    _ = sleep_until(deadline) => break,
                    cmd = self.commands.recv() => match cmd {
                        Some(Command::Retry | Command::Reconnect) => break,
                        Some(Command::Refresh) => continue,
                        Some(Command::Stop) | None => return self.finish(),
                    },
                }
            }
        }
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(probe) = self.probe.take() {
            probe.abort();
        }
        self.set_client(None);
        self.set_core_client(None);
        self.update(|v| {
            v.connection = ConnectionState::Offline;
            v.stale = true;
        });
    }

    /// Change the view; observers only hear of it if more than the sync time
    /// changed (an unchanged snapshot is no news).
    fn update(&self, f: impl FnOnce(&mut HostView)) {
        let modified = self.view.send_if_modified(|v| {
            let before = v.clone();
            f(v);
            *v != HostView {
                synced_at_ms: v.synced_at_ms,
                ..before
            }
        });
        if modified {
            self.changed.notify_one();
        }
    }

    fn set_client(&self, client: Option<Arc<HerdrClient>>) {
        if let Ok(mut slot) = self.client.write() {
            *slot = client;
        }
    }

    fn set_core_client(&self, client: Option<Arc<CoreClient>>) {
        if let Ok(mut slot) = self.core_client.write() {
            *slot = client;
        }
    }

    /// One connected session, from connect until loss or stop.
    async fn session(&mut self) -> Exit {
        let profile = self.profile.clone();
        let connector = self.connector.clone();
        let marks = self.seen_marks.clone();
        let last = self.resolved;
        let connect = async move {
            let transport = Arc::new(connector(profile.clone()).await?);
            let socket = profile.socket_path.as_deref();
            let session = profile.session.as_deref();
            // Only a profile that chose herdr-core may start one.
            let start = profile.backend == Backend::Core;
            let connected = select_backend(profile.backend, last, |b| {
                connect_backend(&transport, b, socket, session, start, &marks)
            })
            .await?;
            Ok::<_, Error>((connected, transport))
        };
        let connect = tokio::time::timeout(CONNECT_DEADLINE, connect);
        tokio::pin!(connect);
        // Stop always wins; Refresh and Retry mean nothing while connecting.
        let (connected, transport) = loop {
            tokio::select! {
                result = &mut connect => match result {
                    Ok(Ok(ok)) => break ok,
                    Ok(Err(e)) => return Exit::Unreachable(e),
                    Err(_) => return Exit::Unreachable(Error::Timeout),
                },
                cmd = self.commands.recv() => match cmd {
                    Some(Command::Stop) | None => return Exit::Stop,
                    // The handshake may have used the old trust: start it over.
                    Some(Command::Reconnect) => return Exit::Reconnect,
                    Some(Command::Refresh | Command::Retry) => {}
                },
            }
        };
        let backend = match connected {
            Connected::Tui(..) => Backend::Tui,
            Connected::Core(..) => Backend::Core,
        };
        let available = self.start_probe(&transport, backend);
        match connected {
            Connected::Tui(client, version, protocol) => {
                tracing::info!(host = %self.profile.name, %version, socket = client.socket_path(), "connected to herdr");
                self.set_client(Some(client.clone()));
                self.update(|v| {
                    v.server_version = Some(version);
                    v.protocol = Some(protocol);
                    v.backend = Some(Backend::Tui);
                    v.available_backends = available;
                });
                let result = self.sync_loop(&client).await;
                client.transport().close().await;
                result
            }
            Connected::Core(client, hello) => {
                tracing::info!(host = %self.profile.name, version = %hello.version, endpoint = %client.endpoint(), "connected to herdr-core");
                self.set_core_client(Some(client.clone()));
                self.update(|v| {
                    v.server_version = Some(hello.version.clone());
                    v.protocol = Some(hello.generation);
                    v.backend = Some(Backend::Core);
                    v.available_backends = available;
                });
                let result = self.core_sync_loop(&client, &hello).await;
                client.transport().close().await;
                result
            }
        }
    }

    /// Note the connected backend and what is known about the other one.
    /// With [`Backend::Auto`], the other backend is checked in the background
    /// ([`detect::probe`]); its answer arrives through [`Self::probe_found`].
    /// Only a running server counts: one that is merely installed is never
    /// started by `auto`. A clear answer is kept until the connected backend
    /// changes; a check without one (a timeout, the session ending) is asked
    /// again on the next connect. On herdr-core, the TUI is looked for every
    /// [`TUI_HINT_EVERY`] until it runs again: that is published as a hint
    /// (`available_backends`), the host stays on herdr-core until this
    /// connection ends (then `auto` prefers the TUI). A fixed backend is
    /// never probed.
    fn start_probe(&mut self, transport: &Arc<Transport>, connected: Backend) -> Vec<Backend> {
        if self.profile.backend != Backend::Auto {
            return vec![connected];
        }
        self.resolved = Some(connected);
        self.probe_transport = Some(transport.clone());
        // On herdr-core the TUI hint is live (it may come and go): never
        // trust an earlier answer there.
        if let Some((for_backend, found)) = &self.probed {
            if *for_backend == connected && connected == Backend::Tui {
                return found.clone();
            }
            self.probed = None;
        }
        self.spawn_probe(connected, None);
        vec![connected]
    }

    /// Check the backend other than `connected` in the background. On
    /// herdr-core it keeps looking (every [`TUI_HINT_EVERY`], slower while
    /// nothing changes) until the TUI's presence differs from `known`, so the
    /// hint follows the TUI both ways; on the TUI it asks once.
    fn spawn_probe(&mut self, connected: Backend, known: Option<bool>) {
        if let Some(old) = self.probe.take() {
            old.abort();
        }
        let Some(transport) = self.probe_transport.clone() else {
            return;
        };
        let other = other_backend(connected);
        let (socket, session) = (
            self.profile.socket_path.clone(),
            self.profile.session.clone(),
        );
        let again = connected == Backend::Core;
        self.probe = Some(tokio::spawn(async move {
            let mut every = TUI_HINT_EVERY;
            loop {
                let found =
                    detect::probe(&transport, other, session.as_deref(), socket.as_deref()).await;
                let running = found.map(|p| p == detect::Presence::Running);
                if !again {
                    return running;
                }
                if transport.is_closed() {
                    return None;
                }
                if running.is_some() && running != known {
                    return running;
                }
                tokio::time::sleep(every).await;
                every = (every * 2).min(TUI_HINT_EVERY * 10);
            }
        }));
    }

    /// The background probe ended: publish both backends if it found the
    /// other; keep only a clear answer.
    fn probe_found(&mut self, found: Option<bool>) {
        let Some(connected) = self.view.borrow().backend else {
            return;
        };
        let Some(found) = found else {
            return;
        };
        let mut all = vec![connected];
        if found {
            all.push(other_backend(connected));
        }
        let all = sorted(all);
        self.probed = Some((connected, all.clone()));
        self.update(|v| v.available_backends = all);
        if connected == Backend::Core && self.profile.backend == Backend::Auto {
            // Keep watching: the hint goes when the TUI goes.
            self.spawn_probe(connected, Some(found));
        }
    }

    /// Keep a herdr-core server's state: apply its events to the list of
    /// terminals in order (herdr-core `schema/README.md`, "Events"), re-read
    /// the layout document on `doc` events, and publish a snapshot after each
    /// burst. Events at or below the last snapshot's `seq` are already in it
    /// and are skipped; when the list no longer matches the core, subscribe
    /// anew (the reply carries a fresh list and its `seq`).
    async fn core_sync_loop(&mut self, client: &Arc<CoreClient>, hello: &Hello) -> Exit {
        type DocFetch<'a> = Pin<Box<dyn Future<Output = Result<DocState>> + Send + 'a>>;
        let has_doc = hello.methods.iter().any(|m| m == "doc.get");
        'resync: loop {
            // Stop and Reconnect are heard while subscribing and reading the
            // first document, too.
            let (mut subscription, subscribed) = match self.unless_told(client.subscribe()).await {
                Ok(Ok(ok)) => ok,
                Ok(Err(e)) => return Exit::Lost(e),
                Err(exit) => return exit,
            };
            let mut watermark = subscribed.seq;
            let mut terminals: Vec<Terminal> = subscribed.terminals;
            let mut doc = match self.unless_told(client.app_document(has_doc)).await {
                Ok(Ok(doc)) => doc,
                Ok(Err(e)) => return Exit::Lost(e),
                Err(exit) => return exit,
            };
            self.publish_core(hello, &terminals, &doc);
            mark_shown(client, &terminals);

            let mut dirty_since: Option<Instant> = None;
            let mut need_doc = false;
            // The document is read beside the stream, so events, Stop and the
            // silence check are still served while it loads.
            let mut doc_fetch: Option<DocFetch<'_>> = None;
            let mut heard = Instant::now();
            loop {
                if need_doc && doc_fetch.is_none() {
                    need_doc = false;
                    doc_fetch = Some(Box::pin(client.app_document(has_doc)));
                }
                let debounce = async {
                    match dirty_since {
                        Some(t) => sleep_until(t + REFRESH_DEBOUNCE).await,
                        None => std::future::pending().await,
                    }
                };
                let fetched = async {
                    match doc_fetch.as_mut() {
                        Some(f) => f.await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    note = subscription.next() => {
                        heard = Instant::now();
                        match note {
                            Ok(Some(Notification::Event { seq, kind, terminal, data })) => {
                                // seq 0: a core that does not number events.
                                if seq != 0 && seq <= watermark {
                                    continue;
                                }
                                watermark = watermark.max(seq);
                                tracing::debug!(host = %self.profile.name, event = %kind, terminal = ?terminal, seq, "core event");
                                if kind == "doc" {
                                    need_doc = true;
                                } else if !hc_snapshot::apply_event(&mut terminals, &kind, terminal.as_deref(), &data) {
                                    tracing::info!(host = %self.profile.name, event = %kind, "core list out of step; resubscribing");
                                    self.update(|v| v.stale = true);
                                    continue 'resync;
                                } else if kind == "created" {
                                    // First sight is now, not at the next
                                    // snapshot: a quick first turn still counts.
                                    let created = terminal.as_deref();
                                    if let Some(t) = terminals.iter().find(|t| Some(t.address.as_str()) == created) {
                                        self.core_seen.baseline(t);
                                    }
                                }
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                            Ok(Some(Notification::Heartbeat { .. })) => {}
                            Ok(Some(Notification::Missed { seq, terminals: fresh })) => {
                                tracing::info!(host = %self.profile.name, "core events missed; taking the fresh list");
                                terminals = fresh;
                                watermark = seq;
                                need_doc = true;
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                            Ok(None) => return Exit::Lost(Error::Protocol("event subscription closed".into())),
                            Err(e) => return Exit::Lost(e),
                        }
                    }
                    result = fetched => {
                        doc_fetch = None;
                        match result {
                            Ok(fresh) => {
                                doc = fresh;
                                dirty_since.get_or_insert_with(Instant::now);
                            }
                            Err(e) => return Exit::Lost(e),
                        }
                    }
                    _ = debounce => {
                        dirty_since = None;
                        self.publish_core(hello, &terminals, &doc);
                        // A turn ended on a terminal the user looks at:
                        // seen (herdr-app labels.md: decide on the turn).
                        mark_shown(client, &terminals);
                    }
                    _ = sleep_until(heard + CORE_SILENCE_LIMIT) => {
                        return Exit::Lost(Error::Connect("herdr-core went silent".into()));
                    }
                    found = wait_probe(&mut self.probe) => self.probe_found(found),
                    cmd = self.commands.recv() => match cmd {
                        // A fresh subscription is the authoritative list.
                        Some(Command::Refresh | Command::Retry) => continue 'resync,
                        Some(Command::Reconnect) => return Exit::Reconnect,
                        Some(Command::Stop) | None => return Exit::Stop,
                    },
                }
            }
        }
    }

    /// Await `work` while still serving commands: Stop and Reconnect end it
    /// (as the session's exit), Refresh and Retry wait for it.
    async fn unless_told<T>(
        &mut self,
        work: impl Future<Output = T>,
    ) -> std::result::Result<T, Exit> {
        tokio::pin!(work);
        loop {
            tokio::select! {
                out = &mut work => return Ok(out),
                cmd = self.commands.recv() => match cmd {
                    Some(Command::Stop) | None => return Err(Exit::Stop),
                    Some(Command::Reconnect) => return Err(Exit::Reconnect),
                    Some(Command::Refresh | Command::Retry) => {}
                },
            }
        }
    }

    fn publish_core(&mut self, hello: &Hello, terminals: &[Terminal], doc: &DocState) {
        let starting = self.layout.pending();
        let snapshot =
            hc_snapshot::build_with(hello, terminals, &doc.doc, &mut self.core_seen, &starting);
        self.publish(snapshot);
    }

    async fn sync_loop(&mut self, client: &HerdrClient) -> Exit {
        'resync: loop {
            let (mut subscription, mut subscribed) = match subscribe_all(client).await {
                Ok(ok) => ok,
                Err(e) => return Exit::Lost(e),
            };
            if let Err(e) = self
                .refresh(client, &mut subscription, &mut subscribed)
                .await
            {
                return Exit::Lost(e);
            }

            let mut dirty_since: Option<Instant> = None;
            let mut health = tokio::time::interval(HEALTH_INTERVAL);
            health.reset();
            loop {
                let debounce = async {
                    match dirty_since {
                        Some(t) => sleep_until(t + REFRESH_DEBOUNCE).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    event = subscription.next() => match event {
                        Ok(Some(event)) => {
                            tracing::debug!(host = %self.profile.name, event = %event.event, pane = ?event.pane_id(), "event");
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                        Ok(None) => return Exit::Lost(Error::Protocol("event subscription closed".into())),
                        Err(e) if e.herdr_code() == Some("events_lost") => {
                            tracing::info!(host = %self.profile.name, "events lost; resyncing");
                            self.update(|v| v.stale = true);
                            continue 'resync;
                        }
                        Err(e) => return Exit::Lost(e),
                    },
                    _ = debounce => {
                        dirty_since = None;
                        if let Err(e) = self.refresh(client, &mut subscription, &mut subscribed).await {
                            return Exit::Lost(e);
                        }
                    }
                    _ = health.tick() => {
                        if client.transport().is_closed() {
                            return Exit::Lost(Error::Connect("connection closed".into()));
                        }
                        if let Err(e) = client.ping().await {
                            return Exit::Lost(e);
                        }
                    }
                    found = wait_probe(&mut self.probe) => self.probe_found(found),
                    cmd = self.commands.recv() => match cmd {
                        Some(Command::Refresh) => { dirty_since.get_or_insert_with(Instant::now); }
                        Some(Command::Retry) => continue 'resync,
                        Some(Command::Reconnect) => return Exit::Reconnect,
                        Some(Command::Stop) | None => return Exit::Stop,
                    },
                }
            }
        }
    }

    /// Take an authoritative snapshot, publish it, and rebuild the
    /// subscription if the pane set changed.
    async fn refresh(
        &self,
        client: &HerdrClient,
        subscription: &mut Subscription,
        subscribed: &mut BTreeSet<String>,
    ) -> Result<()> {
        let snapshot = client.snapshot().await?;
        let panes = pane_ids(&snapshot);
        if panes != *subscribed {
            let (fresh, fresh_panes) = subscribe_all(client).await?;
            *subscription = fresh;
            *subscribed = fresh_panes;
            // Events between this snapshot and the new subscription would be
            // missed, so take one more snapshot after subscribing.
            let snapshot = client.snapshot().await?;
            self.publish(snapshot);
        } else {
            self.publish(snapshot);
        }
        Ok(())
    }

    fn publish(&self, snapshot: SessionSnapshot) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .ok();
        self.update(|v| {
            v.connection = ConnectionState::Online;
            v.server_version = Some(snapshot.version.clone());
            v.protocol = Some(snapshot.protocol);
            v.snapshot = Some(Arc::new(snapshot));
            v.stale = false;
            v.synced_at_ms = now;
        });
    }
}

/// Connect to `backend` (never [`Backend::Auto`]). herdr-core is started
/// through `herdr-core connect` only with `start`.
async fn connect_backend(
    transport: &Arc<Transport>,
    backend: Backend,
    socket: Option<&str>,
    session: Option<&str>,
    start: bool,
    marks: &Arc<SeenMarks>,
) -> Result<Connected> {
    match backend {
        Backend::Core => {
            let (client, hello) =
                CoreClient::connect(transport.clone(), socket, session, start).await?;
            Ok(Connected::Core(
                Arc::new(client.with_marks(marks.clone())),
                hello,
            ))
        }
        _ => {
            let (client, pong) = HerdrClient::connect(transport.clone(), socket, session).await?;
            Ok(Connected::Tui(
                Arc::new(client),
                pong.version,
                pong.protocol,
            ))
        }
    }
}

/// How long the TUI gets when the host was on herdr-core: a TUI that hangs
/// must not keep a working core from connecting.
const TUI_RECHECK: Duration = Duration::from_secs(5);

/// Which server a profile reaches, as one rule for the app and `mc`:
/// `tui` and `core` exactly that one. `auto` prefers the herdr TUI: it
/// connects to herdr-core only when the TUI is surely not there
/// ([`Error::is_absent`]), or when the host was on herdr-core before (`last`)
/// and the TUI does not answer; so it finds back to the TUI once that runs
/// again, and a passing hiccup never swaps one server for the other.
/// `connect` must never start a server for `auto` (see [`connect_backend`]).
pub async fn select_backend<T, F, Fut>(
    chosen: Backend,
    last: Option<Backend>,
    mut connect: F,
) -> Result<T>
where
    F: FnMut(Backend) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    if chosen != Backend::Auto {
        return connect(chosen).await;
    }
    let tui = if last == Some(Backend::Core) {
        tokio::time::timeout(TUI_RECHECK, connect(Backend::Tui))
            .await
            .unwrap_or(Err(Error::Timeout))
    } else {
        connect(Backend::Tui).await
    };
    let tui_error = match tui {
        Ok(connected) => return Ok(connected),
        Err(e) => e,
    };
    if !tui_error.is_absent() && last != Some(Backend::Core) {
        // The TUI may be there and just slow: keep to it.
        return Err(tui_error);
    }
    connect(Backend::Core).await.map_err(|core_error| {
        tracing::debug!(error = %core_error, "no herdr-core either");
        // Report the server that seems to be there.
        if core_error.is_absent() {
            tui_error
        } else {
            core_error
        }
    })
}

fn other_backend(backend: Backend) -> Backend {
    match backend {
        Backend::Core => Backend::Tui,
        _ => Backend::Core,
    }
}

/// TUI first, as everywhere else.
fn sorted(mut backends: Vec<Backend>) -> Vec<Backend> {
    backends.sort_by_key(|b| match b {
        Backend::Tui => 0,
        Backend::Core => 1,
        Backend::Auto => 2,
    });
    backends.dedup();
    backends
}

/// Write `herdr.seen` for the terminals the user looks at whose turn is due,
/// in the background ([`CoreClient::mark_due`]). Each write carries the
/// generation of the look it was scheduled for, so it is dropped when the
/// user left meanwhile.
/// Wait before writing a failed seen mark once more.
const SEEN_RETRY: Duration = Duration::from_secs(2);

fn mark_shown(client: &Arc<CoreClient>, terminals: &[Terminal]) {
    for (address, generation) in client.shown() {
        let Some(t) = terminals
            .iter()
            .find(|t| t.address == address && client.needs_mark(t, generation))
        else {
            continue;
        };
        let (client, t) = (client.clone(), t.clone());
        tokio::spawn(async move {
            // One more try after a failed write: a quiet terminal brings no
            // event that would schedule it again while the user looks.
            for attempt in 0..2 {
                match client.mark_due(&t, generation).await {
                    Ok(()) => return,
                    Err(e) => {
                        tracing::debug!(terminal = %t.address, error = %e, attempt, "marking seen failed");
                    }
                }
                if attempt == 0 {
                    tokio::time::sleep(SEEN_RETRY).await;
                }
            }
        });
    }
}

/// Wait for the background probe; pending when there is none. Cancel-safe.
async fn wait_probe(probe: &mut Option<Probe>) -> Option<bool> {
    match probe {
        Some(handle) => {
            let found = handle.await.unwrap_or(None);
            *probe = None;
            found
        }
        None => std::future::pending().await,
    }
}

fn pane_ids(snapshot: &SessionSnapshot) -> BTreeSet<String> {
    snapshot.panes.iter().map(|p| p.pane_id.clone()).collect()
}

fn subscriptions_for(panes: &BTreeSet<String>) -> Vec<Value> {
    LIFECYCLE_EVENTS
        .iter()
        .map(|t| json!({ "type": t }))
        .chain(
            panes
                .iter()
                .map(|p| json!({ "type": "pane.agent_status_changed", "pane_id": p })),
        )
        .collect()
}

/// Subscribe to lifecycle events plus agent status for every current pane.
/// herdr rejects the whole request if a pane vanished meanwhile, so retry
/// with a fresh pane list.
async fn subscribe_all(client: &HerdrClient) -> Result<(Subscription, BTreeSet<String>)> {
    let mut last_error = None;
    for _ in 0..SUBSCRIBE_ATTEMPTS {
        let panes = pane_ids(&client.snapshot().await?);
        match client.subscribe(subscriptions_for(&panes)).await {
            Ok(sub) => return Ok((sub, panes)),
            Err(e) if e.herdr_code().is_some() => last_error = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::Protocol("subscribe failed".into())))
}

/// Exponential backoff with jitter: ~1s, 2s, 4s … capped at 30s.
fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_secs(1u64 << attempt.saturating_sub(1).min(5)).min(BACKOFF_MAX);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64 % 500)
        .unwrap_or(0);
    base + Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert!(backoff(1) < Duration::from_secs(2));
        assert!(backoff(3) >= Duration::from_secs(4));
        assert!(backoff(20) <= BACKOFF_MAX + Duration::from_millis(500));
    }

    #[test]
    fn subscriptions_include_each_pane() {
        let panes: BTreeSet<String> = ["w1:p1".to_owned(), "w1:p2".to_owned()].into();
        let subs = subscriptions_for(&panes);
        assert_eq!(subs.len(), LIFECYCLE_EVENTS.len() + 2);
        assert_eq!(subs.last().unwrap()["pane_id"], "w1:p2");
    }

    // -- Runtime against a fake herdr ------------------------------------------

    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};

    use crate::transport::LocalTransport;

    const WAIT: Duration = Duration::from_secs(5);

    /// Minimal herdr on a Unix socket: answers `ping` and `session.snapshot`,
    /// acknowledges `events.subscribe` and hands that stream to the test.
    struct FakeHerdr {
        socket: PathBuf,
        /// Label of the only workspace in the snapshot.
        label: Arc<Mutex<String>>,
        subscriptions: mpsc::UnboundedReceiver<UnixStream>,
        /// Every method asked, in order.
        log: Arc<Mutex<Vec<String>>>,
    }

    impl FakeHerdr {
        fn start(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("mc-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("herdr.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let label = Arc::new(Mutex::new("before".to_owned()));
            let (tx, subscriptions) = mpsc::unbounded_channel();
            let shared = label.clone();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let calls = log.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(serve(stream, shared.clone(), tx.clone(), calls.clone()));
                }
            });
            Self {
                socket,
                label,
                subscriptions,
                log,
            }
        }

        async fn next_subscription(&mut self) -> UnixStream {
            tokio::time::timeout(WAIT, self.subscriptions.recv())
                .await
                .expect("no subscription")
                .unwrap()
        }

        fn profile(&self) -> HostProfile {
            HostProfile {
                socket_path: Some(self.socket.to_string_lossy().into_owned()),
                ..profile()
            }
        }
    }

    impl Drop for FakeHerdr {
        fn drop(&mut self) {
            if let Some(dir) = self.socket.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    async fn serve(
        stream: UnixStream,
        label: Arc<Mutex<String>>,
        subscriptions: mpsc::UnboundedSender<UnixStream>,
        log: Arc<Mutex<Vec<String>>>,
    ) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap_or_default();
        log.lock().unwrap().push(method.to_owned());
        let result = match method {
            "ping" => json!({ "type": "pong", "version": "0.9.5", "protocol": 22 }),
            "session.snapshot" => json!({ "snapshot": snapshot(&label.lock().unwrap()) }),
            "events.subscribe" => json!({ "type": "subscription_started" }),
            // Not herdr-core: answer `hello` (backend detection) with an error.
            _ => {
                let mut stream = reader.into_inner();
                let reply = json!({ "id": request["id"], "error": { "code": "unknown_method", "message": method } });
                let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
                return;
            }
        };
        let mut stream = reader.into_inner();
        let reply = format!("{}\n", json!({ "id": request["id"], "result": result }));
        let _ = stream.write_all(reply.as_bytes()).await;
        if method == "events.subscribe" {
            let _ = subscriptions.send(stream);
        }
    }

    fn snapshot(label: &str) -> Value {
        json!({
            "version": "0.9.5", "protocol": 22,
            "workspaces": [{"workspace_id": "w1", "number": 1, "label": label, "focused": true,
                "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"}],
            "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "main",
                "focused": true, "pane_count": 1, "agent_status": "idle"}],
            "panes": [{"pane_id": "w1:p1", "terminal_id": "t1", "workspace_id": "w1",
                "tab_id": "w1:t1", "focused": true, "agent_status": "idle", "revision": 1}],
            "agents": [],
        })
    }

    fn profile() -> HostProfile {
        HostProfile {
            id: "h".into(),
            name: "h".into(),
            host: "localhost".into(),
            port: 22,
            user: "u".into(),
            session: None,
            socket_path: None,
            backend: Backend::Auto,
            edit_layout: false,
        }
    }

    /// Host connecting locally; also returns how often it connected.
    fn spawn_host(profile: HostProfile) -> (HostHandle, Arc<AtomicUsize>) {
        let connects = Arc::new(AtomicUsize::new(0));
        let counter = connects.clone();
        let connector: Connector = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Transport::Local(LocalTransport)) })
        });
        let handle = HostHandle::spawn(profile, connector, Arc::new(Notify::new()));
        (handle, connects)
    }

    async fn wait_for(host: &HostHandle, what: impl FnMut(&HostView) -> bool) -> HostView {
        let mut view = host.subscribe();
        tokio::time::timeout(WAIT, view.wait_for(what))
            .await
            .expect("state not reached")
            .unwrap()
            .clone()
    }

    fn online(v: &HostView) -> bool {
        v.connection == ConnectionState::Online
    }

    fn label(v: &HostView) -> Option<&str> {
        Some(v.snapshot.as_ref()?.workspaces.first()?.label.as_str())
    }

    #[tokio::test]
    async fn connects_and_publishes_snapshot() {
        let mut fake = FakeHerdr::start("online");
        let (host, _) = spawn_host(fake.profile());
        let view = wait_for(&host, online).await;
        assert_eq!(label(&view), Some("before"));
        assert!(!view.stale);
        assert_eq!(view.server_version.as_deref(), Some("0.9.5"));
        assert_eq!(view.backend, Some(Backend::Tui));
        assert_eq!(view.available_backends, vec![Backend::Tui]);
        assert!(host.client().is_ok());
        assert!(host.core_client().is_err());
        let _subscription = fake.next_subscription().await;

        // A refresh with an unchanged snapshot is no news for observers.
        let mut updates = host.subscribe();
        updates.mark_unchanged();
        host.refresh();
        tokio::time::sleep(REFRESH_DEBOUNCE * 4).await;
        assert!(host.view().synced_at_ms > view.synced_at_ms);
        assert!(!updates.has_changed().unwrap());

        host.stop();
        wait_for(&host, |v| v.connection == ConnectionState::Offline).await;
        assert!(host.client().is_err());
    }

    #[tokio::test]
    async fn reconnect_starts_backoff_over() {
        let mut fake = FakeHerdr::start("reconnect");
        let (host, connects) = spawn_host(fake.profile());
        for _ in 0..2 {
            wait_for(&host, online).await;
            // herdr closes the subscription.
            drop(fake.next_subscription().await);
            let view = wait_for(&host, |v| {
                matches!(v.connection, ConnectionState::Reconnecting { .. })
            })
            .await;
            assert!(
                matches!(
                    view.connection,
                    ConnectionState::Reconnecting { attempt: 1, .. }
                ),
                "{:?}",
                view.connection
            );
        }
        wait_for(&host, online).await;
        assert_eq!(connects.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn reconnect_drops_a_live_connection() {
        let mut fake = FakeHerdr::start("forget");
        let (host, connects) = spawn_host(fake.profile());
        wait_for(&host, online).await;
        let _first = fake.next_subscription().await;
        // Retry only re-syncs a live connection …
        host.retry();
        let _again = fake.next_subscription().await;
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        // … Reconnect connects anew (forgotten host key).
        host.reconnect();
        let _second = fake.next_subscription().await;
        wait_for(&host, online).await;
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        let p = fake.profile();
        assert!(host.is_at(&p.host, p.port));
        assert!(!host.is_at(&p.host, p.port.wrapping_add(1)));
        host.stop();
    }

    #[tokio::test]
    async fn event_split_across_writes_survives_other_wakeups() {
        let mut fake = FakeHerdr::start("split");
        let (host, connects) = spawn_host(fake.profile());
        wait_for(&host, online).await;
        let mut subscription = fake.next_subscription().await;

        let event = format!(
            "{}\n",
            json!({ "event": "workspace.renamed", "data": { "workspace_id": "w1" } })
        );
        let (head, tail) = event.split_at(10);
        subscription.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Wake the runtime while the event is half read (cancels `next`).
        host.refresh();
        tokio::time::sleep(REFRESH_DEBOUNCE * 3).await;
        *fake.label.lock().unwrap() = "after".into();
        subscription.write_all(tail.as_bytes()).await.unwrap();

        let view = wait_for(&host, |v| label(v) == Some("after")).await;
        assert!(online(&view));
        assert_eq!(connects.load(Ordering::SeqCst), 1, "must not reconnect");
    }

    // -- Runtime against a fake herdr-core -------------------------------------

    /// Minimal herdr-core: JSON-RPC 2.0, answers `hello`, `terminal.list`,
    /// `doc.get` (with `doc`) and `events.subscribe`, whose stream goes to the
    /// test. Every method asked lands in `log`.
    async fn serve_core(
        stream: UnixStream,
        subscriptions: mpsc::UnboundedSender<UnixStream>,
        log: Arc<Mutex<Vec<String>>>,
        doc: Arc<Mutex<Value>>,
        generation: Arc<AtomicUsize>,
        tui: Arc<AtomicBool>,
    ) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap_or_default();
        log.lock().unwrap().push(method.to_owned());
        if method == "ping" && tui.load(Ordering::SeqCst) {
            // A herdr TUI answers here too now.
            let reply = json!({"id": request["id"],
                "result": {"type": "pong", "version": "0.9.5", "protocol": 22}});
            let mut stream = reader.into_inner();
            let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
            return;
        }
        let terminal = json!({"address": "t1", "state": "live", "name": "claude",
            "reported": true, "turn": 0, "labels": {}, "cwd": "/tmp/w", "title": "",
            "status": {"state": "working", "app": "claude-code"}});
        let reply = match method {
            "hello" => json!({"result": {"version": "0.1.20",
                "generation": generation.load(Ordering::SeqCst),
                "core": {"id": "c", "name": "box"}, "framed": false, "features": [],
                "methods": ["events.subscribe", "terminal.get", "terminal.list", "doc.get"]}}),
            "terminal.list" => json!({"result": {"terminals": [terminal], "hidden_finished": 0}}),
            "doc.get" => json!({"result": {"value": doc.lock().unwrap().clone(), "version": 1}}),
            "events.subscribe" => json!({"result": {"seq": 1, "terminals": [terminal]}}),
            "terminal.label" => json!({"result": terminal}),
            _ => json!({"error": {"code": -32601, "message": "no such method",
                "data": {"kind": "usage"}}}),
        };
        let mut reply = reply;
        reply["jsonrpc"] = json!("2.0");
        reply["id"] = request["id"].clone();
        let mut stream = reader.into_inner();
        let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
        if method == "events.subscribe" {
            let _ = subscriptions.send(stream);
        }
    }

    struct FakeCore {
        socket: PathBuf,
        subscriptions: mpsc::UnboundedReceiver<UnixStream>,
        log: Arc<Mutex<Vec<String>>>,
        doc: Arc<Mutex<Value>>,
        /// The protocol generation `hello` answers with.
        generation: Arc<AtomicUsize>,
        /// Answer `ping` as a herdr TUI as well.
        tui: Arc<AtomicBool>,
    }

    impl FakeCore {
        fn start(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("mc-test-{}-core-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("default.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let (tx, subscriptions) = mpsc::unbounded_channel();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let doc = Arc::new(Mutex::new(Value::Null));
            let generation = Arc::new(AtomicUsize::new(1));
            let tui = Arc::new(AtomicBool::new(false));
            let (calls, shared, spoken, pong) =
                (log.clone(), doc.clone(), generation.clone(), tui.clone());
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(serve_core(
                        stream,
                        tx.clone(),
                        calls.clone(),
                        shared.clone(),
                        spoken.clone(),
                        pong.clone(),
                    ));
                }
            });
            Self {
                socket,
                subscriptions,
                log,
                doc,
                generation,
                tui,
            }
        }

        fn profile(&self, backend: Backend) -> HostProfile {
            HostProfile {
                socket_path: Some(self.socket.to_string_lossy().into_owned()),
                backend,
                edit_layout: false,
                ..profile()
            }
        }

        async fn next_subscription(&mut self) -> UnixStream {
            tokio::time::timeout(WAIT, self.subscriptions.recv())
                .await
                .expect("no subscription")
                .unwrap()
        }

        fn methods(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Drop for FakeCore {
        fn drop(&mut self) {
            if let Some(dir) = self.socket.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    async fn send_note(stream: &mut UnixStream, note: Value) {
        stream
            .write_all(format!("{note}\n").as_bytes())
            .await
            .unwrap();
    }

    fn status_event(seq: u64, state: &str) -> Value {
        json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": seq,
            "type": "status", "terminal": "t1",
            "data": {"status": {"state": state, "app": "claude-code"}, "records": {}, "reported": true}}})
    }

    fn first_status(v: &HostView) -> Option<crate::herdr::model::AgentStatus> {
        Some(v.snapshot.as_ref()?.agents.first()?.agent_status)
    }

    #[tokio::test]
    async fn core_server_syncs_through_events() {
        use crate::herdr::model::AgentStatus;
        for backend in [Backend::Core, Backend::Auto] {
            let mut fake = FakeCore::start(backend.as_str());
            let (host, _) = spawn_host(fake.profile(backend));
            let view = wait_for(&host, online).await;
            assert_eq!(view.backend, Some(Backend::Core));
            assert_eq!(view.available_backends, vec![Backend::Core]);
            assert_eq!(view.server_version.as_deref(), Some("0.1.20"));
            assert_eq!(first_status(&view), Some(AgentStatus::Working));
            assert!(matches!(host.client(), Err(Error::Unsupported { .. })));
            assert!(host.core_client().is_ok());

            let mut subscription = fake.next_subscription().await;
            send_note(&mut subscription, status_event(2, "blocked")).await;
            wait_for(&host, |v| first_status(v) == Some(AgentStatus::Blocked)).await;
            host.stop();
        }
    }

    #[tokio::test]
    async fn core_counts_the_first_turn_of_a_new_terminal() {
        use crate::herdr::model::AgentStatus;
        let mut fake = FakeCore::start("first-turn");
        let (host, _) = spawn_host(fake.profile(Backend::Core));
        wait_for(&host, online).await;
        let mut sub = fake.next_subscription().await;
        // Created at turn 0 and done with its first turn within one debounce.
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 10,
                "type": "created", "terminal": "t9", "data": {"address": "t9", "state": "live",
                "name": "quick", "reported": true, "turn": 0, "labels": {},
                "status": {"state": "working", "app": "codex"}}}}),
        )
        .await;
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 11,
                "type": "status", "terminal": "t9",
                "data": {"status": {"state": "idle", "app": "codex"}, "records": {}, "reported": true}}}),
        )
        .await;
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 12,
                "type": "turn", "terminal": "t9", "data": {"turn": 1, "state": "idle"}}}),
        )
        .await;
        let quick = |v: &HostView| {
            v.snapshot
                .as_ref()?
                .agents
                .iter()
                .find(|a| a.pane_id == "t9")
                .map(|a| a.agent_status)
        };
        wait_for(&host, |v| quick(v) == Some(AgentStatus::Done)).await;
        host.stop();
    }

    fn turn_event(seq: u64, turn: u64) -> Value {
        json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": seq,
            "type": "turn", "terminal": "t1", "data": {"turn": turn, "state": "idle"}}})
    }

    async fn labels_written(fake: &FakeCore, count: usize) {
        let deadline = Instant::now() + WAIT;
        while fake
            .methods()
            .iter()
            .filter(|m| *m == "terminal.label")
            .count()
            < count
        {
            assert!(
                Instant::now() < deadline,
                "{count} labels expected: {:?}",
                fake.methods()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn label_count(fake: &FakeCore) -> usize {
        fake.methods()
            .iter()
            .filter(|m| *m == "terminal.label")
            .count()
    }

    #[tokio::test]
    async fn a_look_started_offline_is_served_once_connected() {
        let fake = FakeCore::start("seen-offline");
        let (host, _) = spawn_host(fake.profile(Backend::Core));
        // On screen before the host is online (the app came to the front
        // while it reconnects): the look waits for the connection.
        host.show_seen("t1");
        wait_for(&host, online).await;
        labels_written(&fake, 1).await;
        host.stop();
    }

    #[tokio::test]
    async fn turns_are_marked_while_shown_and_not_after_leaving() {
        let mut fake = FakeCore::start("seen-turns");
        let (host, _) = spawn_host(fake.profile(Backend::Core));
        wait_for(&host, online).await;
        let mut sub = fake.next_subscription().await;
        assert_eq!(label_count(&fake), 0, "nothing is shown yet");
        host.show_seen("t1");
        send_note(&mut sub, turn_event(2, 1)).await;
        labels_written(&fake, 1).await;
        // The same turn again: written once.
        send_note(&mut sub, status_event(3, "idle")).await;
        tokio::time::sleep(REFRESH_DEBOUNCE * 3).await;
        assert_eq!(label_count(&fake), 1);
        // After leaving, a new turn writes nothing.
        host.forget_seen("t1");
        send_note(&mut sub, turn_event(4, 2)).await;
        tokio::time::sleep(REFRESH_DEBOUNCE * 3).await;
        assert_eq!(label_count(&fake), 1);
        // Looking again marks the new turn.
        host.show_seen("t1");
        send_note(&mut sub, status_event(5, "idle")).await;
        labels_written(&fake, 2).await;
        host.stop();
    }

    #[tokio::test]
    async fn core_skips_events_already_in_a_fresh_list() {
        use crate::herdr::model::AgentStatus;
        let mut fake = FakeCore::start("seq");
        let (host, _) = spawn_host(fake.profile(Backend::Core));
        wait_for(&host, online).await;
        let mut sub = fake.next_subscription().await;
        // Fell behind: a fresh list at seq 5 says blocked.
        let mut blocked = serde_json::from_value::<Value>(json!({"address": "t1", "state": "live",
            "name": "claude", "reported": true, "turn": 0, "labels": {},
            "status": {"state": "blocked", "app": "claude-code"}}))
        .unwrap();
        blocked["cwd"] = json!("/tmp/w");
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "missed",
            "params": {"seq": 5, "terminals": [blocked]}}),
        )
        .await;
        wait_for(&host, |v| first_status(v) == Some(AgentStatus::Blocked)).await;
        // An older event queued before the list must not undo it ...
        send_note(&mut sub, status_event(4, "working")).await;
        tokio::time::sleep(REFRESH_DEBOUNCE * 3).await;
        assert_eq!(first_status(&host.view()), Some(AgentStatus::Blocked));
        // ... a newer one applies.
        send_note(&mut sub, status_event(6, "working")).await;
        wait_for(&host, |v| first_status(v) == Some(AgentStatus::Working)).await;
        // `missed` reads the document again (once at start, once now).
        assert_eq!(fake.methods().iter().filter(|m| *m == "doc.get").count(), 2);
        host.stop();
    }

    #[tokio::test]
    async fn core_doc_event_reads_the_layout_again() {
        let mut fake = FakeCore::start("doc");
        let (host, _) = spawn_host(fake.profile(Backend::Core));
        let view = wait_for(&host, online).await;
        let workspace = |v: &HostView| {
            v.snapshot
                .as_ref()
                .and_then(|s| s.panes.first())
                .map(|p| p.workspace_id.clone())
        };
        assert_eq!(
            workspace(&view).as_deref(),
            Some(hc_snapshot::UNSORTED_WORKSPACE)
        );
        let mut sub = fake.next_subscription().await;
        *fake.doc.lock().unwrap() = json!({"spaces": [{"id": "s1", "name": "shop", "tabs": [
            {"id": "k1", "name": "api", "root": {"id": "n", "type": "block",
                "content": {"terminal": "t1"}}}]}]});
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event",
            "params": {"seq": 2, "type": "doc", "data": {"name": "herdr-app"}}}),
        )
        .await;
        wait_for(&host, |v| workspace(v).as_deref() == Some("s1")).await;
        host.stop();
    }

    #[tokio::test]
    async fn core_unknown_terminal_resubscribes_and_lost_stream_reconnects() {
        let mut fake = FakeCore::start("resub");
        let (host, connects) = spawn_host(fake.profile(Backend::Core));
        wait_for(&host, online).await;
        let mut sub = fake.next_subscription().await;
        // A change for a terminal we never heard of: subscribe anew.
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 3,
            "type": "title", "terminal": "t9", "data": {"title": "x"}}}),
        )
        .await;
        let mut sub = fake.next_subscription().await;
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        // A newer core's event type is ignored, no resubscribe.
        send_note(
            &mut sub,
            json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 4,
            "type": "sparkles", "terminal": "t1", "data": {}}}),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            fake.methods()
                .iter()
                .filter(|m| *m == "events.subscribe")
                .count(),
            2
        );
        // The stream ends: connect anew.
        drop(sub);
        let _sub = fake.next_subscription().await;
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        host.stop();
    }

    #[tokio::test]
    async fn a_silent_core_is_reconnected_after_45_seconds() {
        let mut fake = FakeCore::start("silence");
        let (host, connects) = spawn_host(fake.profile(Backend::Core));
        wait_for(&host, online).await;
        let _sub = fake.next_subscription().await;
        // The clock stands still while the test moves it; the sockets stay
        // real, so it runs again before waiting on them.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(30)).await;
        tokio::time::resume();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(connects.load(Ordering::SeqCst), 1, "30 s of quiet is fine");
        assert_eq!(host.view().connection, ConnectionState::Online);
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(16)).await;
        tokio::time::resume();
        let _sub = fake.next_subscription().await;
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        host.stop();
    }

    #[tokio::test]
    async fn a_newer_core_waits_for_the_user() {
        let fake = FakeCore::start("newer");
        fake.generation.store(2, Ordering::SeqCst);
        for backend in [Backend::Core, Backend::Auto] {
            let (host, _) = spawn_host(fake.profile(backend));
            let view = wait_for(&host, |v| {
                matches!(v.connection, ConnectionState::ActionRequired { .. })
            })
            .await;
            let ConnectionState::ActionRequired {
                error: ErrorInfo::Incompatible { what },
            } = view.connection
            else {
                panic!("{:?}", view.connection)
            };
            assert!(what.contains("generation"), "{what}");
            host.stop();
        }
    }

    #[tokio::test]
    async fn auto_looks_for_the_tui_again_on_reconnect() {
        let mut fake = FakeCore::start("auto-keep");
        let (host, connects) = spawn_host(fake.profile(Backend::Auto));
        let view = wait_for(&host, online).await;
        assert_eq!(view.backend, Some(Backend::Core));
        // The first connect asked the TUI first ...
        assert_eq!(fake.methods().first().map(String::as_str), Some("ping"));
        let sub = fake.next_subscription().await;
        fake.log.lock().unwrap().clear();
        drop(sub);
        let _sub = fake.next_subscription().await;
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        // ... and so does the reconnect (the TUI may be back), then stays
        // with herdr-core, since no TUI answers.
        let methods = fake.methods();
        assert_eq!(methods.first().map(String::as_str), Some("ping"));
        assert!(methods.iter().any(|m| m == "hello"), "{methods:?}");
        assert_eq!(host.view().backend, Some(Backend::Core));
        host.stop();
    }

    #[tokio::test]
    async fn auto_on_core_hints_when_the_tui_runs_again() {
        let fake = FakeCore::start("tui-hint");
        let (host, _) = spawn_host(fake.profile(Backend::Auto));
        let view = wait_for(&host, online).await;
        assert_eq!(view.backend, Some(Backend::Core));
        assert_eq!(view.available_backends, vec![Backend::Core]);
        // The TUI starts: a hint, the host stays on herdr-core.
        fake.tui.store(true, Ordering::SeqCst);
        let view = wait_for(&host, |v| v.available_backends.len() == 2).await;
        assert_eq!(view.available_backends, vec![Backend::Tui, Backend::Core]);
        assert_eq!(view.backend, Some(Backend::Core));
        // The TUI stops again: the hint goes.
        fake.tui.store(false, Ordering::SeqCst);
        let view = wait_for(&host, |v| v.available_backends.len() == 1).await;
        assert_eq!(view.available_backends, vec![Backend::Core]);
        host.stop();
    }

    #[tokio::test]
    async fn reconnect_switches_auto_back_to_the_tui() {
        // "Switch to herdr TUI": a retry only re-syncs herdr-core; a reconnect
        // chooses again, and the TUI comes first.
        let fake = FakeCore::start("tui-switch");
        let (host, connects) = spawn_host(fake.profile(Backend::Auto));
        wait_for(&host, online).await;
        fake.tui.store(true, Ordering::SeqCst);
        host.retry();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            connects.load(Ordering::SeqCst),
            1,
            "retry keeps the session"
        );
        fake.log.lock().unwrap().clear();
        host.reconnect();
        // The fake answers only the TUI's ping, so the TUI session it opens
        // asks for its snapshot: the TUI was chosen.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fake.methods().iter().any(|m| m == "session.snapshot") {
            assert!(Instant::now() < deadline, "{:?}", fake.methods());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        // (A later hello is the background look for herdr-core.)
        assert_eq!(&fake.methods()[..2], ["ping", "session.snapshot"]);
        // That TUI then fails its snapshot with an error reply: a failure of
        // a running server, never "no herdr here".
        let view = wait_for(&host, |v| {
            matches!(v.connection, ConnectionState::Reconnecting { .. })
        })
        .await;
        let ConnectionState::Reconnecting { absent, error, .. } = view.connection else {
            unreachable!()
        };
        assert!(!absent, "{error}");
        host.stop();
    }

    #[tokio::test]
    async fn fixed_tui_profile_is_never_probed() {
        let mut fake = FakeHerdr::start("fixed-tui");
        let (host, _) = spawn_host(HostProfile {
            backend: Backend::Tui,
            ..fake.profile()
        });
        let view = wait_for(&host, online).await;
        assert_eq!(view.available_backends, vec![Backend::Tui]);
        fake.next_subscription().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let log = fake.log.lock().unwrap().clone();
        assert!(log.iter().all(|m| m != "hello"), "{log:?}");
        // The connect itself: one ping, nothing in front of it.
        assert_eq!(log.first().map(String::as_str), Some("ping"));
        host.stop();
    }

    #[tokio::test]
    async fn auto_tui_probes_core_after_going_online() {
        let mut fake = FakeHerdr::start("auto-tui");
        let (host, _) = spawn_host(fake.profile());
        let view = wait_for(&host, online).await;
        assert_eq!(view.backend, Some(Backend::Tui));
        // Online before any core probe got an answer; the probe finds none.
        assert_eq!(view.available_backends, vec![Backend::Tui]);
        assert_eq!(
            fake.log.lock().unwrap().first().map(String::as_str),
            Some("ping")
        );
        fake.next_subscription().await;
        host.stop();
    }

    /// Runs [`select_backend`] against fixed answers per backend; returns
    /// the result and the backends tried in order.
    async fn select_with(
        chosen: Backend,
        last: Option<Backend>,
        tui: fn() -> Result<Backend>,
        core: fn() -> Result<Backend>,
    ) -> (Result<Backend>, Vec<Backend>) {
        let tried = Mutex::new(Vec::new());
        let got = select_backend(chosen, last, |b| {
            tried.lock().unwrap().push(b);
            let r = if b == Backend::Tui { tui() } else { core() };
            async move { r }
        })
        .await;
        (got, tried.into_inner().unwrap())
    }

    fn gone() -> Result<Backend> {
        Err(Error::SocketUnavailable {
            path: "/tmp/x".into(),
            reason: "refused".into(),
        })
    }

    #[tokio::test]
    async fn automatic_choice_prefers_the_tui_and_switches_only_when_surely_absent() {
        use Backend::{Auto, Core, Tui};
        // TUI answers: TUI, core never asked.
        let (got, tried) = select_with(Auto, None, || Ok(Tui), || Ok(Core)).await;
        assert_eq!((got.unwrap(), tried), (Tui, vec![Tui]));
        // TUI surely absent: herdr-core.
        let (got, tried) = select_with(Auto, None, gone, || Ok(Core)).await;
        assert_eq!((got.unwrap(), tried), (Core, vec![Tui, Core]));
        // TUI slow: stay with it, no switch on a timeout.
        let (got, tried) = select_with(Auto, None, || Err(Error::Timeout), || Ok(Core)).await;
        assert!(matches!(got, Err(Error::Timeout)));
        assert_eq!(tried, vec![Tui]);
        // Was on herdr-core, TUI back: back to the TUI.
        let (got, _) = select_with(Auto, Some(Core), || Ok(Tui), || Ok(Core)).await;
        assert_eq!(got.unwrap(), Tui);
        // Was on herdr-core, TUI hangs: keep the core.
        let (got, _) = select_with(Auto, Some(Core), || Err(Error::Timeout), || Ok(Core)).await;
        assert_eq!(got.unwrap(), Core);
        // Neither there: the TUI's "not reachable".
        let (got, _) = select_with(Auto, None, gone, gone).await;
        assert!(matches!(got, Err(Error::SocketUnavailable { .. })));
        // TUI absent, core of a newer generation: that is the news.
        let (got, _) = select_with(Auto, None, gone, || {
            Err(Error::Incompatible {
                what: "generation 2".into(),
            })
        })
        .await;
        assert!(matches!(got, Err(Error::Incompatible { .. })));
        // Fixed backend: only that one.
        let (got, tried) = select_with(Tui, None, gone, || Ok(Core)).await;
        assert!(matches!(got, Err(Error::SocketUnavailable { .. })));
        assert_eq!(tried, vec![Tui]);
        let (got, tried) = select_with(Core, None, || Ok(Tui), || Ok(Core)).await;
        assert_eq!((got.unwrap(), tried), (Core, vec![Core]));
    }

    #[test]
    fn incompatible_waits_for_the_user_and_is_not_absence() {
        let e = Error::Incompatible {
            what: "generation 2".into(),
        };
        assert!(needs_user(&e));
        assert!(!e.is_absent());
        assert!(
            Error::Herdr {
                code: "x".into(),
                message: String::new()
            }
            .is_absent()
        );
        assert!(!Error::Timeout.is_absent());
    }

    #[tokio::test]
    async fn stop_wins_while_connecting() {
        let connector: Connector = Arc::new(|_| Box::pin(std::future::pending()));
        let host = HostHandle::spawn(profile(), connector, Arc::new(Notify::new()));
        let started = Instant::now();
        host.retry();
        host.stop();
        wait_for(&host, |v| v.connection == ConnectionState::Offline).await;
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

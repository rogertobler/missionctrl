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

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::{Instant, sleep_until};

use crate::error::{Error, Result};
use crate::herdr::model::SessionSnapshot;
use crate::herdr::{HerdrClient, Subscription};
use crate::profile::HostProfile;
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
            other => ErrorInfo::Other {
                message: other.to_string(),
            },
        }
    }
}

fn needs_user(e: &Error) -> bool {
    matches!(
        e,
        Error::HostKeyUnknown { .. } | Error::HostKeyChanged { .. } | Error::AuthFailed { .. }
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
}

impl HostHandle {
    /// Start the runtime task for `profile`. `changed` is notified on every
    /// view update (used by the fleet to re-aggregate).
    pub fn spawn(profile: HostProfile, connector: Connector, changed: Arc<Notify>) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (view_tx, view_rx) = watch::channel(HostView {
            profile: profile.clone(),
            connection: ConnectionState::Connecting,
            server_version: None,
            protocol: None,
            snapshot: None,
            stale: true,
            synced_at_ms: None,
        });
        let client = Arc::new(RwLock::new(None));
        let runtime = Runtime {
            profile: profile.clone(),
            connector,
            commands: commands_rx,
            view: view_tx,
            changed,
            client: client.clone(),
        };
        tokio::spawn(runtime.run());
        Self {
            id: profile.id,
            commands: commands_tx,
            view: view_rx,
            client,
        }
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

    /// Connected API client for actions on this host.
    pub fn client(&self) -> Result<Arc<HerdrClient>> {
        self.client
            .read()
            .ok()
            .and_then(|c| c.clone())
            .ok_or(Error::NotConnected)
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
}

/// How a session ended.
enum Exit {
    Stop,
    Lost(Error),
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
            // Still Online means this session synced: start the backoff over.
            if self.view.borrow().connection == ConnectionState::Online {
                attempt = 0;
            }
            let error = match exit {
                Exit::Stop => break,
                Exit::Reconnect => {
                    attempt = 0;
                    continue;
                }
                Exit::Lost(e) => e,
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
            self.update(|v| {
                v.connection = ConnectionState::Reconnecting {
                    attempt,
                    error: message,
                    retry_in_ms: delay.as_millis() as u64,
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
        self.set_client(None);
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

    /// One connected session, from connect until loss or stop.
    async fn session(&mut self) -> Exit {
        let profile = self.profile.clone();
        let connector = self.connector.clone();
        let connect = async move {
            let transport = Arc::new(connector(profile.clone()).await?);
            let (client, pong) = HerdrClient::connect(
                transport,
                profile.socket_path.as_deref(),
                profile.session.as_deref(),
            )
            .await?;
            Ok::<_, Error>((Arc::new(client), pong))
        };
        let connect = tokio::time::timeout(CONNECT_DEADLINE, connect);
        tokio::pin!(connect);
        // Stop always wins; Refresh and Retry mean nothing while connecting.
        let (client, pong) = loop {
            tokio::select! {
                result = &mut connect => match result {
                    Ok(Ok(ok)) => break ok,
                    Ok(Err(e)) => return Exit::Lost(e),
                    Err(_) => return Exit::Lost(Error::Timeout),
                },
                cmd = self.commands.recv() => match cmd {
                    Some(Command::Stop) | None => return Exit::Stop,
                    // The handshake may have used the old trust: start it over.
                    Some(Command::Reconnect) => return Exit::Reconnect,
                    Some(Command::Refresh | Command::Retry) => {}
                },
            }
        };
        tracing::info!(host = %self.profile.name, version = %pong.version, socket = client.socket_path(), "connected");
        self.set_client(Some(client.clone()));
        self.update(|v| {
            v.server_version = Some(pong.version.clone());
            v.protocol = Some(pong.protocol);
        });

        let result = self.sync_loop(&client).await;
        client.transport().close().await;
        result
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(serve(stream, shared.clone(), tx.clone()));
                }
            });
            Self {
                socket,
                label,
                subscriptions,
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
    ) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "ping" => json!({ "type": "pong", "version": "0.9.5", "protocol": 22 }),
            "session.snapshot" => json!({ "snapshot": snapshot(&label.lock().unwrap()) }),
            "events.subscribe" => json!({ "type": "subscription_started" }),
            other => panic!("unexpected method {other}"),
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
        assert!(host.client().is_ok());
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

//! Several hosts managed together, with one aggregated view.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{Notify, watch};

use crate::error::{Error, Result};
use crate::herdr::model::{AgentInfo, AgentStatus};
use crate::host::{Connector, HostHandle, HostView};
use crate::profile::HostProfile;

/// One agent with enough context to show it in a cross-host list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FleetAgent {
    pub host_id: String,
    pub host_name: String,
    pub workspace_label: Option<String>,
    pub tab_label: Option<String>,
    /// True if the host is not live, so the status may be outdated.
    pub stale: bool,
    pub agent: AgentInfo,
}

/// Aggregated state across all hosts.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct FleetView {
    /// Hosts in the order they were added.
    pub hosts: Vec<HostView>,
    /// All agents, most urgent first (blocked, done, working, idle, unknown).
    pub agents: Vec<FleetAgent>,
}

impl FleetView {
    pub fn count(&self, status: AgentStatus) -> usize {
        self.agents
            .iter()
            .filter(|a| a.agent.agent_status == status)
            .count()
    }
}

struct Inner {
    /// Insertion order is kept by the `order` vector.
    hosts: BTreeMap<String, HostHandle>,
    order: Vec<String>,
}

pub struct Fleet {
    inner: Arc<Mutex<Inner>>,
    connector: Connector,
    changed: Arc<Notify>,
    view: watch::Receiver<FleetView>,
}

impl Fleet {
    pub fn new(connector: Connector) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            hosts: BTreeMap::new(),
            order: Vec::new(),
        }));
        let changed = Arc::new(Notify::new());
        let (view_tx, view_rx) = watch::channel(FleetView::default());
        tokio::spawn(aggregate(inner.clone(), changed.clone(), view_tx));
        Self {
            inner,
            connector,
            changed,
            view: view_rx,
        }
    }

    /// Add a host and start connecting. Replaces a host with the same id.
    pub fn add_host(&self, profile: HostProfile) -> HostHandle {
        let handle = HostHandle::spawn(
            profile.clone(),
            self.connector.clone(),
            self.changed.clone(),
        );
        let mut inner = self.inner.lock().expect("fleet lock");
        if let Some(old) = inner.hosts.insert(profile.id.clone(), handle.clone()) {
            old.stop();
        } else {
            inner.order.push(profile.id);
        }
        drop(inner);
        self.changed.notify_one();
        handle
    }

    pub fn remove_host(&self, id: &str) {
        let mut inner = self.inner.lock().expect("fleet lock");
        if let Some(handle) = inner.hosts.remove(id) {
            handle.stop();
        }
        inner.order.retain(|h| h != id);
        drop(inner);
        self.changed.notify_one();
    }

    pub fn host(&self, id: &str) -> Result<HostHandle> {
        self.inner
            .lock()
            .expect("fleet lock")
            .hosts
            .get(id)
            .cloned()
            .ok_or(Error::NotConnected)
    }

    /// Handles of all hosts, in the order they were added.
    pub fn hosts(&self) -> Vec<HostHandle> {
        let inner = self.inner.lock().expect("fleet lock");
        inner
            .order
            .iter()
            .filter_map(|id| inner.hosts.get(id).cloned())
            .collect()
    }

    pub fn view(&self) -> FleetView {
        self.view.borrow().clone()
    }

    /// Receiver that changes whenever any host changes.
    pub fn subscribe(&self) -> watch::Receiver<FleetView> {
        self.view.clone()
    }

    pub fn stop_all(&self) {
        for handle in self.inner.lock().expect("fleet lock").hosts.values() {
            handle.stop();
        }
    }
}

async fn aggregate(inner: Arc<Mutex<Inner>>, changed: Arc<Notify>, view: watch::Sender<FleetView>) {
    loop {
        changed.notified().await;
        let hosts: Vec<HostView> = {
            let inner = inner.lock().expect("fleet lock");
            inner
                .order
                .iter()
                .filter_map(|id| inner.hosts.get(id))
                .map(HostHandle::view)
                .collect()
        };
        let next = build_view(hosts);
        view.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        });
        if view.is_closed() {
            return;
        }
    }
}

fn build_view(hosts: Vec<HostView>) -> FleetView {
    let mut agents = Vec::new();
    for (host_index, host) in hosts.iter().enumerate() {
        let Some(snapshot) = &host.snapshot else {
            continue;
        };
        for agent in &snapshot.agents {
            let workspace_label = snapshot
                .workspaces
                .iter()
                .find(|w| w.workspace_id == agent.workspace_id)
                .map(|w| w.label.clone());
            let tab_label = snapshot
                .tabs
                .iter()
                .find(|t| t.tab_id == agent.tab_id)
                .map(|t| t.label.clone());
            agents.push((
                host_index,
                FleetAgent {
                    host_id: host.profile.id.clone(),
                    host_name: host.profile.name.clone(),
                    workspace_label,
                    tab_label,
                    stale: host.stale,
                    agent: agent.clone(),
                },
            ));
        }
    }
    // Most urgent first; stable within a host so rows don't jump around.
    agents.sort_by(|(ha, a), (hb, b)| {
        b.agent
            .agent_status
            .attention_rank()
            .cmp(&a.agent.agent_status.attention_rank())
            .then(ha.cmp(hb))
    });
    FleetView {
        hosts,
        agents: agents.into_iter().map(|(_, a)| a).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::model::SessionSnapshot;
    use crate::host::ConnectionState;

    fn agent(pane: &str, status: AgentStatus) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "pane_id": pane, "terminal_id": "t", "workspace_id": "w1", "tab_id": "w1:t1",
            "focused": false, "agent_status": status, "revision": 1
        }))
        .unwrap()
    }

    fn host(id: &str, agents: Vec<AgentInfo>) -> HostView {
        HostView {
            profile: HostProfile {
                id: id.into(),
                name: id.into(),
                host: "h".into(),
                port: 22,
                user: "u".into(),
                session: None,
                socket_path: None,
            },
            connection: ConnectionState::Online,
            server_version: None,
            protocol: None,
            snapshot: Some(Arc::new(SessionSnapshot {
                version: "0.9.5".into(),
                protocol: 22,
                focused_workspace_id: None,
                focused_tab_id: None,
                focused_pane_id: None,
                workspaces: vec![],
                tabs: vec![],
                panes: vec![],
                agents,
                layouts: vec![],
            })),
            stale: false,
            synced_at_ms: None,
        }
    }

    #[test]
    fn agents_sorted_by_attention_across_hosts() {
        let view = build_view(vec![
            host(
                "a",
                vec![
                    agent("w1:p1", AgentStatus::Idle),
                    agent("w1:p2", AgentStatus::Done),
                ],
            ),
            host(
                "b",
                vec![
                    agent("w1:p1", AgentStatus::Blocked),
                    agent("w1:p2", AgentStatus::Working),
                ],
            ),
        ]);
        let order: Vec<_> = view
            .agents
            .iter()
            .map(|a| (a.host_id.as_str(), a.agent.agent_status))
            .collect();
        assert_eq!(
            order,
            vec![
                ("b", AgentStatus::Blocked),
                ("a", AgentStatus::Done),
                ("b", AgentStatus::Working),
                ("a", AgentStatus::Idle),
            ]
        );
        assert_eq!(view.count(AgentStatus::Blocked), 1);
    }
}

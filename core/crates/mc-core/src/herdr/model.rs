//! Typed subset of herdr's socket API objects.
//!
//! Field names and shapes follow herdr's published JSON schema
//! (`herdr api schema --json`, protocol 22, herdr 0.9.5; herdr is Apache-2.0).
//! Only fields MissionCtrl uses are modelled. Unknown fields are ignored and
//! unknown enum values map to a fallback, as herdr's stability notes require.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Semantic agent state of a pane, workspace, or tab rollup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    /// herdr's name of the state, e.g. `working`.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Done => "done",
            AgentStatus::Unknown => "unknown",
        }
    }

    /// Higher means more in need of the user's attention.
    pub fn attention_rank(self) -> u8 {
        match self {
            AgentStatus::Blocked => 4,
            AgentStatus::Done => 3,
            AgentStatus::Working => 2,
            AgentStatus::Idle => 1,
            AgentStatus::Unknown => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub capabilities: Option<ServerCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub live_handoff: bool,
    #[serde(default)]
    pub health_check: bool,
    #[serde(default)]
    pub surface_interest: bool,
    #[serde(default)]
    pub endpoint_protocol_generation: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
    pub agents: Vec<AgentInfo>,
    /// Tab layouts (BSP trees). Kept opaque until the UI needs them.
    #[serde(default)]
    pub layouts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub number: u32,
    pub label: String,
    pub focused: bool,
    pub pane_count: u32,
    pub tab_count: u32,
    pub active_tab_id: String,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub number: u32,
    pub label: String,
    pub focused: bool,
    pub pane_count: u32,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub revision: u64,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub state_labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub focused: bool,
    pub agent_status: AgentStatus,
    pub revision: u64,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub state_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub state_change_seq: u64,
}

impl AgentInfo {
    /// Best human-readable name for the agent.
    pub fn display_name(&self) -> &str {
        self.display_agent
            .as_deref()
            .or(self.name.as_deref())
            .or(self.agent.as_deref())
            .unwrap_or("agent")
    }

    /// State label customized by integrations, falling back to the raw state.
    pub fn status_label(&self) -> String {
        let key = self.agent_status.as_str();
        self.state_labels
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }
}

/// Result of `pane.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneRead {
    pub pane_id: String,
    pub text: String,
    /// Changes whenever the pane content changes; lets callers skip re-rendering.
    pub revision: u64,
    #[serde(default)]
    pub truncated: bool,
}

/// A pushed event from an `events.subscribe` connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl Event {
    pub fn pane_id(&self) -> Option<&str> {
        self.data.get("pane_id").and_then(|v| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_status_falls_back() {
        let s: AgentStatus = serde_json::from_str("\"thinking_hard\"").unwrap();
        assert_eq!(s, AgentStatus::Unknown);
        let s: AgentStatus = serde_json::from_str("\"blocked\"").unwrap();
        assert_eq!(s, AgentStatus::Blocked);
    }

    #[test]
    fn status_names_match_serde() {
        use AgentStatus::*;
        for s in [Idle, Working, Blocked, Done, Unknown] {
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
    }

    #[test]
    fn snapshot_ignores_unknown_fields() {
        let json = r#"{
            "version": "0.9.5", "protocol": 22, "future_field": 1,
            "workspaces": [{"workspace_id":"w1","number":1,"label":"api","focused":true,
                "pane_count":1,"tab_count":1,"active_tab_id":"w1:t1","agent_status":"working","tokens":{}}],
            "tabs": [{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"main",
                "focused":true,"pane_count":1,"agent_status":"working"}],
            "panes": [{"pane_id":"w1:p1","terminal_id":"term_1","workspace_id":"w1","tab_id":"w1:t1",
                "focused":true,"agent_status":"working","revision":3,"scroll":null}],
            "agents": [{"pane_id":"w1:p1","terminal_id":"term_1","workspace_id":"w1","tab_id":"w1:t1",
                "focused":true,"agent_status":"working","revision":3,"agent":"claude",
                "state_labels":{"working":"refactoring"},"interactive_ready":true}],
            "layouts": []
        }"#;
        let snap: SessionSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents[0].display_name(), "claude");
        assert_eq!(snap.agents[0].status_label(), "refactoring");
        assert_eq!(snap.workspaces[0].agent_status, AgentStatus::Working);
    }
}

//! Maps herdr-core's terminals and the Herdr app's layout document onto the
//! [`SessionSnapshot`] of the herdr TUI.
//!
//! - Space → workspace, tab → tab, terminal → pane (pane id = address).
//! - A terminal belongs where the layout shows it; failing that, where its
//!   labels `herdr.space`/`herdr.tab` put it (the Herdr app adopts such
//!   terminals the same way). The rest goes to one extra workspace,
//!   [`UNSORTED_WORKSPACE`].
//! - Status follows herdr-app `docs/labels.md`: `blocked` and `error` need
//!   the user, `working` works, anything else is done when `turn` is above
//!   the shared `herdr.seen` label, else idle.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use super::model::{AppDocument, Hello, Space, Tab, Terminal};
use crate::herdr::model::{
    AgentInfo, AgentStatus, PaneInfo, SessionSnapshot, TabInfo, WorkspaceInfo,
};

/// Workspace for terminals that belong to no space.
pub const UNSORTED_WORKSPACE: &str = "hc:unsorted";
/// Its tabs, one per terminal: this prefix and the terminal's address.
pub const UNSORTED_TAB_PREFIX: &str = "hc:unsorted:";
/// Label of that workspace until the app names it itself.
const UNSORTED_LABEL: &str = "Terminals";

/// Label holding the last turn a person looked at (shared by all clients).
pub const SEEN_LABEL: &str = "herdr.seen";
pub const SPACE_LABEL: &str = "herdr.space";
pub const TAB_LABEL: &str = "herdr.tab";

/// Agents' OSC 7501 app names and process names, mapped to the names the
/// herdr TUI uses (`claude`, `codex`). After herdr-app `defaults/programs.json`.
const AGENT_NAMES: &[(&str, &str)] = &[
    ("claude-code", "claude"),
    ("claude", "claude"),
    ("codex", "codex"),
    ("gemini", "gemini"),
    ("opencode", "opencode"),
    ("cursor-agent", "cursor"),
    ("github-copilot", "copilot"),
    ("copilot", "copilot"),
    ("amp", "amp"),
    ("pi", "pi"),
];

fn agent_name(raw: &str) -> Option<&'static str> {
    let raw = raw.trim().to_ascii_lowercase();
    AGENT_NAMES
        .iter()
        .find(|(name, _)| *name == raw)
        .map(|(_, agent)| *agent)
}

/// The agent running in `t`, if any: its own status report names it, else
/// the foreground process is a known agent (one that does not report yet).
pub fn agent_of(t: &Terminal) -> Option<String> {
    if let Some(app) = t.status.as_ref().and_then(|s| s.app.as_deref()) {
        return Some(agent_name(app).map_or_else(|| app.to_owned(), str::to_owned));
    }
    let process = t
        .foreground
        .as_ref()
        .map(|f| f.name.as_str())
        .or_else(|| t.argv.first().map(|a| a.rsplit('/').next().unwrap_or(a)));
    if let Some(agent) = process.and_then(agent_name) {
        return Some(agent.to_owned());
    }
    // A status without an app name: still an agent. `reported` alone is not
    // enough: it stays true after the program cleared its status.
    t.status.is_some().then(|| "agent".to_owned())
}

/// What this client remembers about a core's terminals beyond what the core
/// reports. Kept for the life of a host's runtime (across reconnects).
///
/// - Turns seen while no client has set the shared `herdr.seen` label: the
///   turn a terminal had when this client first saw it counts as seen (what
///   happened before is unknown), so a missing label is never read as
///   "nothing seen yet".
/// - A counter per terminal that goes up when its shown status changes
///   (`state_change_seq`): the core's `status_ms` is not sent with status
///   events, so it cannot serve.
#[derive(Debug, Default, Clone)]
pub struct LocalSeen {
    seen: BTreeMap<String, u64>,
    changes: BTreeMap<String, (AgentStatus, u64)>,
    counter: u64,
}

impl LocalSeen {
    /// The turn counted as seen for `t`, recorded on first sight.
    pub fn baseline(&mut self, t: &Terminal) -> u64 {
        *self.seen.entry(t.address.clone()).or_insert(t.turn)
    }

    /// Note `status` for `t`; returns its change counter, which moves only
    /// when the status differs from the last one noted.
    pub fn note_status(&mut self, t: &Terminal, status: AgentStatus) -> u64 {
        match self.changes.get(&t.address) {
            Some((last, seq)) if *last == status => *seq,
            _ => {
                self.counter += 1;
                self.changes
                    .insert(t.address.clone(), (status, self.counter));
                self.counter
            }
        }
    }

    /// Forget terminals that are gone.
    pub fn retain(&mut self, terminals: &[Terminal]) {
        let known = |address: &String| terminals.iter().any(|t| &t.address == address);
        self.seen.retain(|address, _| known(address));
        self.changes.retain(|address, _| known(address));
    }
}

/// Status of a terminal as MissionCtrl shows it. `baseline` is the turn
/// counted as seen when the shared label is missing ([`LocalSeen`]).
pub fn status_of(t: &Terminal, baseline: u64) -> AgentStatus {
    let word = t.status.as_ref().map(|s| s.state.as_str());
    match word {
        // `error` needs the user like `blocked` (the Herdr app treats it so);
        // the word itself stays visible through `state_labels`.
        Some("blocked" | "error") => AgentStatus::Blocked,
        Some("working") => AgentStatus::Working,
        // No status: an agent that never reports, or a plain shell, also one
        // whose agent has ended (`reported` stays true after that). The herdr
        // TUI and the Herdr app show plain shells without a status as well.
        _ if word.is_none() && (!t.reported || agent_of(t).is_none()) => AgentStatus::Unknown,
        _ => {
            let seen = t
                .label(SEEN_LABEL)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(baseline);
            if t.turn > seen {
                AgentStatus::Done
            } else {
                AgentStatus::Idle
            }
        }
    }
}

/// A title worth showing: empty ones count as none.
fn title_of(t: &Terminal) -> Option<String> {
    t.title.clone().filter(|title| !title.trim().is_empty())
}

/// Words other than herdr's own, shown as the status label.
fn state_labels(t: &Terminal, status: AgentStatus) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    if t.status.as_ref().is_some_and(|s| s.state == "error") {
        labels.insert(status.as_str().to_owned(), "error".to_owned());
    }
    labels
}

fn rollup(statuses: impl Iterator<Item = AgentStatus>) -> AgentStatus {
    statuses
        .max_by_key(|s| s.attention_rank())
        .unwrap_or(AgentStatus::Unknown)
}

/// A workspace to show: id, label, and its tabs as (id, label), in order.
type PlacedWorkspace = (String, String, Vec<(String, String)>);

/// Where each terminal goes: (workspace id, tab id), in display order.
struct Placement {
    /// Workspaces in order with their tabs in order.
    workspaces: Vec<PlacedWorkspace>,
    /// Terminal address → (workspace, tab).
    of: BTreeMap<String, (String, String)>,
    /// Pane order inside each tab.
    order: BTreeMap<String, Vec<String>>,
}

/// A space's name; a nameless one by its folder's last part, as the Herdr
/// app shows it when nothing better is known (`~` for the home folder).
fn space_label(space: &Space) -> String {
    if !space.name.trim().is_empty() {
        return space.name.clone();
    }
    let folder = space.folder.trim_end_matches('/');
    if folder.is_empty() {
        return String::new();
    }
    let home = ["/home/", "/Users/"].iter().any(|h| {
        folder
            .strip_prefix(h)
            .is_some_and(|rest| !rest.contains('/'))
    }) || folder == "/root";
    if home {
        return "~".to_owned();
    }
    folder.rsplit('/').next().unwrap_or(folder).to_owned()
}

fn place(terminals: &[&Terminal], doc: &AppDocument) -> Placement {
    let live: BTreeSet<&str> = terminals.iter().map(|t| t.address.as_str()).collect();
    let mut placement = Placement {
        workspaces: Vec::new(),
        of: BTreeMap::new(),
        order: BTreeMap::new(),
    };
    for space in &doc.spaces {
        let mut tabs = Vec::new();
        for tab in &space.tabs {
            tabs.push((tab.id.clone(), tab.name.clone()));
            for address in tab.terminals() {
                if live.contains(address.as_str()) && !placement.of.contains_key(&address) {
                    placement
                        .of
                        .insert(address.clone(), (space.id.clone(), tab.id.clone()));
                    placement
                        .order
                        .entry(tab.id.clone())
                        .or_default()
                        .push(address);
                }
            }
        }
        placement
            .workspaces
            .push((space.id.clone(), space_label(space), tabs));
    }
    let mut unsorted: Vec<&Terminal> = Vec::new();
    for t in terminals {
        if placement.of.contains_key(&t.address) {
            continue;
        }
        // Not in the layout: its labels may still name a space and tab.
        let labeled = t.label(SPACE_LABEL).and_then(|space_id| {
            let (_, _, tabs) = placement
                .workspaces
                .iter()
                .find(|(id, _, _)| id == space_id)?;
            let tab = t
                .label(TAB_LABEL)
                .and_then(|tab_id| tabs.iter().find(|(id, _)| id == tab_id))
                .or_else(|| tabs.first())?;
            Some((space_id.to_owned(), tab.0.clone()))
        });
        match labeled {
            Some((space, tab)) => {
                placement
                    .order
                    .entry(tab.clone())
                    .or_default()
                    .push(t.address.clone());
                placement.of.insert(t.address.clone(), (space, tab));
            }
            None => unsorted.push(*t),
        }
    }
    if !unsorted.is_empty() {
        let mut tabs = Vec::new();
        for t in unsorted {
            let tab = format!("{UNSORTED_TAB_PREFIX}{}", t.address);
            placement.of.insert(
                t.address.clone(),
                (UNSORTED_WORKSPACE.to_owned(), tab.clone()),
            );
            placement.order.insert(tab.clone(), vec![t.address.clone()]);
            tabs.push((tab, loose_name(t)));
        }
        placement.workspaces.push((
            UNSORTED_WORKSPACE.to_owned(),
            UNSORTED_LABEL.to_owned(),
            tabs,
        ));
    }
    placement
}

/// The tab name of a loose terminal: its name, else its address. Not its
/// title: the app keeps a tab's colour and icon by its name, and a title
/// changes with whatever the program shows.
fn loose_name(t: &Terminal) -> String {
    t.name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&t.address)
        .to_owned()
}

/// Build the snapshot of one core. Only live terminals on this core are
/// shown; terminals of machines it added (`machine/t7`) keep their spaces in
/// that machine's own document and are left out for now.
pub fn build(
    hello: &Hello,
    terminals: &[Terminal],
    doc: &AppDocument,
    seen: &mut LocalSeen,
) -> SessionSnapshot {
    build_with(hello, terminals, doc, seen, &BTreeSet::new())
}

/// [`build`], leaving out the terminals of tabs MissionCtrl is creating
/// (`starting`: their tab ids) until the layout holds them, so they do not
/// flash up among the loose terminals in between.
pub fn build_with(
    hello: &Hello,
    terminals: &[Terminal],
    doc: &AppDocument,
    seen: &mut LocalSeen,
    starting: &BTreeSet<String>,
) -> SessionSnapshot {
    seen.retain(terminals);
    let in_layout: BTreeSet<String> = doc
        .spaces
        .iter()
        .flat_map(|s| s.tabs.iter().flat_map(Tab::terminals))
        .collect();
    let shown: Vec<&Terminal> = terminals
        .iter()
        .filter(|t| t.is_live() && !t.is_remote())
        .filter(|t| {
            let being_made = t.label(SPACE_LABEL).is_none()
                && t.label(TAB_LABEL).is_some_and(|tab| starting.contains(tab));
            !being_made || in_layout.contains(&t.address)
        })
        .collect();
    let by_address: BTreeMap<&str, &Terminal> =
        shown.iter().map(|t| (t.address.as_str(), *t)).collect();
    let placement = place(&shown, doc);

    let mut workspaces = Vec::new();
    let mut tabs = Vec::new();
    let mut panes = Vec::new();
    let mut agents = Vec::new();
    let mut layouts = Vec::new();

    for (ws_index, (ws_id, ws_label, ws_tabs)) in placement.workspaces.iter().enumerate() {
        let mut ws_statuses = Vec::new();
        let mut ws_panes = 0;
        for (tab_index, (tab_id, tab_label)) in ws_tabs.iter().enumerate() {
            let addresses = placement.order.get(tab_id).cloned().unwrap_or_default();
            let mut tab_statuses = Vec::new();
            for address in &addresses {
                let Some(t) = by_address.get(address.as_str()) else {
                    continue;
                };
                let status = status_of(t, seen.baseline(t));
                tab_statuses.push(status);
                let agent = agent_of(t);
                let cwd = t
                    .foreground
                    .as_ref()
                    .and_then(|f| f.cwd.clone())
                    .or_else(|| t.cwd.clone());
                let pane = PaneInfo {
                    pane_id: t.address.clone(),
                    terminal_id: t.address.clone(),
                    workspace_id: ws_id.clone(),
                    tab_id: tab_id.clone(),
                    focused: false,
                    agent_status: status,
                    revision: t.turn,
                    label: t.name.clone(),
                    agent: agent.clone(),
                    display_agent: None,
                    title: title_of(t),
                    terminal_title_stripped: title_of(t),
                    cwd: t.cwd.clone(),
                    foreground_cwd: cwd.clone(),
                    state_labels: state_labels(t, status),
                };
                if agent.is_some() {
                    agents.push(AgentInfo {
                        pane_id: pane.pane_id.clone(),
                        terminal_id: pane.terminal_id.clone(),
                        workspace_id: ws_id.clone(),
                        tab_id: tab_id.clone(),
                        focused: false,
                        agent_status: status,
                        revision: t.turn,
                        agent: agent.clone(),
                        display_agent: None,
                        name: t.name.clone(),
                        title: title_of(t),
                        terminal_title_stripped: title_of(t),
                        cwd,
                        state_labels: pane.state_labels.clone(),
                        state_change_seq: seen.note_status(t, status),
                    });
                }
                panes.push(pane);
            }
            ws_panes += tab_statuses.len() as u32;
            let tab_status = rollup(tab_statuses.iter().copied());
            ws_statuses.push(tab_status);
            tabs.push(TabInfo {
                tab_id: tab_id.clone(),
                workspace_id: ws_id.clone(),
                number: tab_index as u32 + 1,
                label: tab_label.clone(),
                focused: false,
                pane_count: tab_statuses.len() as u32,
                agent_status: tab_status,
            });
        }
        workspaces.push(WorkspaceInfo {
            workspace_id: ws_id.clone(),
            number: ws_index as u32 + 1,
            label: ws_label.clone(),
            focused: false,
            pane_count: ws_panes,
            tab_count: ws_tabs.len() as u32,
            active_tab_id: ws_tabs.first().map(|t| t.0.clone()).unwrap_or_default(),
            agent_status: rollup(ws_statuses.into_iter()),
        });
    }
    for space in &doc.spaces {
        for tab in &space.tabs {
            layouts.push(json!({ "workspace_id": space.id, "tab_id": tab.id, "root": tab.root }));
        }
    }

    SessionSnapshot {
        version: hello.version.clone(),
        protocol: hello.generation,
        focused_workspace_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        workspaces,
        tabs,
        panes,
        agents,
        layouts,
    }
}

/// Apply one `event` notification to the list of terminals, as herdr-core's
/// `schema/README.md` ("Events") describes. Returns false when the list no
/// longer matches the core (a change for a terminal it does not hold, or a
/// record it cannot read): the caller then subscribes anew. Event types this
/// build does not know are ignored, as the core's own relay does (the API
/// only grows). `doc` events concern documents and are left to the caller.
pub fn apply_event(
    terminals: &mut Vec<Terminal>,
    kind: &str,
    address: Option<&str>,
    data: &serde_json::Value,
) -> bool {
    use serde_json::Value;
    const KNOWN: &[&str] = &[
        "created",
        "restarted",
        "removed",
        "finished",
        "lost",
        "status",
        "turn",
        "title",
        "labels",
        "foreground",
        "size",
        "keep",
    ];
    if !KNOWN.contains(&kind) {
        return true;
    }
    let record = |v: &Value| serde_json::from_value::<Terminal>(v.clone()).ok();
    let Some(address) = address else {
        return false;
    };
    let index = terminals.iter().position(|t| t.address == address);
    match (kind, index) {
        ("created" | "restarted", _) => {
            let Some(t) = record(data) else { return false };
            match index {
                Some(i) => terminals[i] = t,
                None => terminals.push(t),
            }
        }
        ("removed", Some(i)) => {
            terminals.remove(i);
        }
        ("finished", Some(i)) => {
            terminals[i].state = "finished".into();
            terminals[i].foreground = None;
        }
        ("lost", Some(i)) => terminals[i].foreground = None,
        ("status", Some(i)) => {
            let t = &mut terminals[i];
            t.status = data
                .get("status")
                .and_then(|s| serde_json::from_value(s.clone()).ok());
            if let Some(r) = data.get("reported").and_then(Value::as_bool) {
                t.reported = r;
            }
        }
        ("turn", Some(i)) => {
            if let Some(turn) = data.get("turn").and_then(Value::as_u64) {
                terminals[i].turn = turn;
            }
        }
        ("title", Some(i)) => {
            terminals[i].title = data
                .get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(str::to_owned);
        }
        ("labels", _) => {
            if let Some(t) = data.get("record").and_then(record) {
                match index {
                    Some(i) => terminals[i] = t,
                    None => terminals.push(t),
                }
            } else {
                let Some(i) = index else { return false };
                match data
                    .get("labels")
                    .map(|l| serde_json::from_value(l.clone()))
                {
                    Some(Ok(labels)) => terminals[i].labels = labels,
                    _ => return false,
                }
            }
        }
        ("foreground", Some(i)) => {
            terminals[i].foreground = serde_json::from_value(data.clone()).ok().flatten();
        }
        ("size", Some(i)) => {
            if let (Some(c), Some(r)) = (
                data.get("cols").and_then(Value::as_u64),
                data.get("rows").and_then(Value::as_u64),
            ) {
                terminals[i].cols = c as u16;
                terminals[i].rows = r as u16;
            }
        }
        // Not shown by MissionCtrl.
        ("keep", Some(_)) => {}
        // A change for a terminal we do not hold: our list is off.
        (_, None) => return false,
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn hello() -> Hello {
        serde_json::from_value(json!({"version":"0.1.20","generation":1,
            "core":{"id":"c1","name":"box"},"framed":false,"methods":[],"features":[]}))
        .unwrap()
    }

    fn terminal(address: &str, extra: Value) -> Terminal {
        let mut t = json!({"address": address, "state": "live", "cwd": "/tmp/w",
            "argv": ["bash"], "labels": {}, "turn": 0, "reported": false});
        for (k, v) in extra.as_object().unwrap() {
            t[k] = v.clone();
        }
        serde_json::from_value(t).unwrap()
    }

    fn doc() -> AppDocument {
        AppDocument::parse(
            &json!({"spaces":[{"id":"s1","name":"shop","folder":"/tmp/shop",
            "tabs":[
                {"id":"k1","name":"api","root":{"id":"n1","type":"row","children":[
                    {"id":"n2","type":"block","content":{"terminal":"t2"}},
                    {"id":"n3","type":"block","content":{"terminal":"t1"}}]}},
                {"id":"k2","name":"web","root":{"id":"n4","type":"row","children":[]}}]}]}),
        )
        .unwrap()
    }

    #[test]
    fn status_follows_shared_seen_label() {
        let claude = |state: &str, turn: u64, seen: Option<&str>, baseline: u64| {
            let mut labels = json!({});
            if let Some(s) = seen {
                labels[SEEN_LABEL] = json!(s);
            }
            status_of(
                &terminal(
                    "t1",
                    json!({"status": {"state": state, "app": "claude-code"}, "reported": true,
                        "turn": turn, "labels": labels}),
                ),
                baseline,
            )
        };
        assert_eq!(claude("blocked", 3, Some("3"), 0), AgentStatus::Blocked);
        assert_eq!(claude("error", 3, None, 0), AgentStatus::Blocked);
        assert_eq!(claude("working", 3, Some("1"), 0), AgentStatus::Working);
        assert_eq!(claude("idle", 3, Some("2"), 0), AgentStatus::Done);
        assert_eq!(claude("done", 3, Some("3"), 0), AgentStatus::Idle);
        // No label: the turn first seen counts as seen, the program's own
        // "done" word does not decide.
        assert_eq!(claude("done", 3, None, 3), AgentStatus::Idle);
        assert_eq!(claude("idle", 4, None, 3), AgentStatus::Done);
        // A plain shell, and an agent that never reports: no status.
        assert_eq!(
            status_of(&terminal("t1", json!({})), 0),
            AgentStatus::Unknown
        );
        let gemini = terminal(
            "t1",
            json!({"foreground": {"name": "gemini", "argv": ["gemini"]}}),
        );
        assert_eq!(status_of(&gemini, 0), AgentStatus::Unknown);
        assert_eq!(agent_of(&gemini).as_deref(), Some("gemini"));
        // Reported once, status cleared since: no longer an agent.
        let cleared = terminal("t1", json!({"reported": true}));
        assert_eq!(agent_of(&cleared), None);
        assert_eq!(status_of(&cleared, 0), AgentStatus::Unknown);
    }

    #[test]
    fn local_seen_counts_the_first_turn_as_seen() {
        let mut seen = LocalSeen::default();
        let t = |turn: u64| {
            terminal(
                "t1",
                json!({"status": {"state": "idle", "app": "codex"}, "reported": true, "turn": turn}),
            )
        };
        let snap = build(&hello(), &[t(5)], &AppDocument::default(), &mut seen);
        assert_eq!(snap.agents[0].agent_status, AgentStatus::Idle);
        let snap = build(&hello(), &[t(6)], &AppDocument::default(), &mut seen);
        assert_eq!(snap.agents[0].agent_status, AgentStatus::Done);
        build(&hello(), &[], &AppDocument::default(), &mut seen);
        // Forgotten once gone: the same address later starts afresh.
        let snap = build(&hello(), &[t(9)], &AppDocument::default(), &mut seen);
        assert_eq!(snap.agents[0].agent_status, AgentStatus::Idle);
    }

    #[test]
    fn maps_spaces_tabs_and_unsorted_terminals() {
        let terminals = vec![
            terminal(
                "t1",
                json!({"name": "claude", "reported": true,
                "status": {"state": "blocked", "app": "claude-code"}}),
            ),
            terminal("t2", json!({})),
            // Not in the layout but labeled for the second tab.
            terminal(
                "t3",
                json!({"labels": {"herdr.space": "s1", "herdr.tab": "k2"}}),
            ),
            terminal("t4", json!({})),
            terminal("t5", json!({"state": "finished"})),
            terminal("box/t1", json!({})),
        ];
        let snap = build(&hello(), &terminals, &doc(), &mut LocalSeen::default());
        assert_eq!(snap.version, "0.1.20");
        assert_eq!(snap.protocol, 1);
        let ws: Vec<_> = snap
            .workspaces
            .iter()
            .map(|w| (&*w.workspace_id, &*w.label))
            .collect();
        assert_eq!(
            ws,
            vec![("s1", "shop"), (UNSORTED_WORKSPACE, UNSORTED_LABEL)]
        );
        assert_eq!(snap.workspaces[0].agent_status, AgentStatus::Blocked);
        assert_eq!(snap.workspaces[0].pane_count, 3);
        assert_eq!(snap.workspaces[0].tab_count, 2);
        let panes: Vec<_> = snap
            .panes
            .iter()
            .map(|p| (&*p.pane_id, &*p.tab_id))
            .collect();
        assert_eq!(
            panes,
            vec![
                ("t2", "k1"),
                ("t1", "k1"),
                ("t3", "k2"),
                ("t4", "hc:unsorted:t4")
            ]
        );
        assert_eq!(snap.agents.len(), 1);
        assert_eq!(snap.agents[0].agent.as_deref(), Some("claude"));
        assert_eq!(snap.agents[0].display_name(), "claude");
        assert_eq!(snap.tabs[0].agent_status, AgentStatus::Blocked);
        assert_eq!(snap.tabs[1].label, "web");
        assert_eq!(snap.layouts.len(), 2);
        assert_eq!(snap.layouts[0]["tab_id"], "k1");
    }

    #[test]
    fn nameless_spaces_go_by_their_folder_and_tabs_being_made_wait() {
        let doc = AppDocument::parse(&json!({"spaces":[
            {"id":"s1","name":"","folder":"/srv/shop/","tabs":[{"id":"k1"}]},
            {"id":"s2","name":"","folder":"/home/u","tabs":[{"id":"k2"}]}]}))
        .unwrap();
        // t2 is a tab MissionCtrl is making: not yet in the layout, no space.
        let being_made = terminal("t2", json!({"labels":{"herdr.tab":"tbNew"}}));
        let starting: BTreeSet<String> = ["tbNew".to_owned()].into();
        let snap = build_with(
            &hello(),
            std::slice::from_ref(&being_made),
            &doc,
            &mut LocalSeen::default(),
            &starting,
        );
        let labels: Vec<&str> = snap.workspaces.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["shop", "~"]);
        assert!(snap.panes.is_empty());
        // Without the start in progress it is a loose terminal.
        let snap = build(&hello(), &[being_made], &doc, &mut LocalSeen::default());
        assert_eq!(snap.panes.len(), 1);
    }

    #[test]
    fn terminals_in_tab_level_floating_panes_stand_in_their_tab() {
        let doc = AppDocument::parse(&json!({"spaces":[{"id":"s1","name":"shop","folder":"/","tabs":[
            {"id":"k1","name":"api","root":{"id":"p1","type":"block","content":{"terminal":"t1"}},
             "floating":[{"pane":{"id":"f1","type":"block","content":{"terminal":"t2"}},"rect":[0,0,10,10]}]},
            {"id":"k2","name":"web"}]}]}))
        .unwrap();
        let tab_of = |snap: &SessionSnapshot, terminal: &str| {
            snap.panes
                .iter()
                .find(|p| p.terminal_id == terminal)
                .map(|p| p.tab_id.clone())
        };
        // Without labels: where the layout has it, not among the loose ones.
        let snap = build(
            &hello(),
            &[terminal("t1", json!({})), terminal("t2", json!({}))],
            &doc,
            &mut LocalSeen::default(),
        );
        assert_eq!(tab_of(&snap, "t2").as_deref(), Some("k1"));
        assert!(
            !snap
                .workspaces
                .iter()
                .any(|w| w.workspace_id == UNSORTED_WORKSPACE)
        );
        // Stale labels naming another tab do not move it.
        let snap = build(
            &hello(),
            &[
                terminal("t1", json!({})),
                terminal(
                    "t2",
                    json!({"labels":{"herdr.space":"s1","herdr.tab":"k2"}}),
                ),
            ],
            &doc,
            &mut LocalSeen::default(),
        );
        assert_eq!(tab_of(&snap, "t2").as_deref(), Some("k1"));
    }

    #[test]
    fn without_document_everything_is_unsorted() {
        let snap = build(
            &hello(),
            &[terminal("t1", json!({}))],
            &AppDocument::default(),
            &mut LocalSeen::default(),
        );
        assert_eq!(snap.workspaces.len(), 1);
        assert_eq!(snap.workspaces[0].workspace_id, UNSORTED_WORKSPACE);
        assert_eq!(snap.tabs.len(), 1);
        // One tab per loose terminal, named after it.
        let snap = build(
            &hello(),
            &[
                terminal("t1", json!({"name": "build"})),
                terminal("t2", json!({"title": "vim notes"})),
                terminal("t3", json!({"name": " "})),
            ],
            &AppDocument::default(),
            &mut LocalSeen::default(),
        );
        let tabs: Vec<_> = snap.tabs.iter().map(|t| (&*t.tab_id, &*t.label)).collect();
        assert_eq!(
            tabs,
            [
                ("hc:unsorted:t1", "build"),
                ("hc:unsorted:t2", "t2"),
                ("hc:unsorted:t3", "t3")
            ]
        );
        let empty = build(
            &hello(),
            &[],
            &AppDocument::default(),
            &mut LocalSeen::default(),
        );
        assert!(empty.workspaces.is_empty());
    }

    #[test]
    fn events_update_terminals() {
        let mut ts = vec![terminal("t1", json!({}))];
        assert!(apply_event(
            &mut ts,
            "status",
            Some("t1"),
            &json!({"status": {"state": "working", "app": "claude-code"}, "records": {}, "reported": true})
        ));
        assert_eq!(status_of(&ts[0], 0), AgentStatus::Working);
        assert!(apply_event(
            &mut ts,
            "turn",
            Some("t1"),
            &json!({"turn": 2, "state": "idle"})
        ));
        assert!(apply_event(
            &mut ts,
            "status",
            Some("t1"),
            &json!({"status": {"state": "idle"}, "reported": true})
        ));
        assert_eq!(ts[0].turn, 2);
        assert!(apply_event(
            &mut ts,
            "labels",
            Some("t1"),
            &json!({"labels": {"herdr.seen": "1"}})
        ));
        assert_eq!(status_of(&ts[0], 0), AgentStatus::Done);
        assert!(apply_event(
            &mut ts,
            "title",
            Some("t1"),
            &json!({"title": "x"})
        ));
        assert!(apply_event(
            &mut ts,
            "created",
            Some("t2"),
            &serde_json::to_value(terminal("t2", json!({}))).unwrap()
        ));
        assert_eq!(ts.len(), 2);
        assert!(apply_event(
            &mut ts,
            "finished",
            Some("t2"),
            &json!({"reason": "exited"})
        ));
        assert!(!ts[1].is_live());
        assert!(apply_event(&mut ts, "removed", Some("t2"), &json!({})));
        assert_eq!(ts.len(), 1);
        assert!(!apply_event(&mut ts, "status", Some("t9"), &json!({})));
        assert!(!apply_event(
            &mut ts,
            "labels",
            Some("t9"),
            &json!({"labels": {}})
        ));
        // A newer core's event types are ignored, not a reason to resync.
        assert!(apply_event(&mut ts, "brand_new", Some("t1"), &json!({})));
        assert!(apply_event(&mut ts, "brand_new", Some("t9"), &json!({})));
        assert!(apply_event(
            &mut ts,
            "size",
            Some("t1"),
            &json!({"cols": 90, "rows": 30})
        ));
        assert_eq!((ts[0].cols, ts[0].rows), (90, 30));
    }

    #[test]
    fn status_change_counter_moves_only_on_change() {
        let mut seen = LocalSeen::default();
        let t = terminal("t1", json!({}));
        let first = seen.note_status(&t, AgentStatus::Working);
        assert_eq!(seen.note_status(&t, AgentStatus::Working), first);
        let second = seen.note_status(&t, AgentStatus::Done);
        assert!(second > first);
        let other = terminal("t2", json!({}));
        assert!(seen.note_status(&other, AgentStatus::Idle) > second);
        seen.retain(&[other]);
        // t1 forgotten: its next status counts as a change.
        assert!(seen.note_status(&t, AgentStatus::Done) > second);
    }

    #[test]
    fn error_word_stays_visible() {
        let t = terminal(
            "t1",
            json!({"reported": true, "status": {"state": "error", "app": "codex"}}),
        );
        let snap = build(
            &hello(),
            &[t],
            &AppDocument::default(),
            &mut LocalSeen::default(),
        );
        assert_eq!(snap.agents[0].status_label(), "error");
        assert_eq!(snap.agents[0].agent.as_deref(), Some("codex"));
    }

    /// A real `events.subscribe` reply of hc 0.1.20 (paths and names
    /// replaced): every field the core sends must parse, and the mapping
    /// must hold for it.
    #[test]
    fn recorded_subscription_of_hc_0_1_20() {
        let reply: Value =
            serde_json::from_str(include_str!("fixtures/subscribe-0.1.20.json")).unwrap();
        let subscribed: super::super::model::Subscribed =
            serde_json::from_value(reply["result"].clone()).unwrap();
        assert_eq!(subscribed.seq, 123);
        assert_eq!(subscribed.terminals.len(), 6);
        let snap = build(
            &hello(),
            &subscribed.terminals,
            &AppDocument::default(),
            &mut LocalSeen::default(),
        );
        // The finished codex is not shown; the rest have no space.
        let panes: Vec<&str> = snap.panes.iter().map(|p| p.pane_id.as_str()).collect();
        assert_eq!(panes, vec!["t1", "t13", "t14", "t15", "t16"]);
        assert!(
            snap.panes
                .iter()
                .all(|p| p.workspace_id == UNSORTED_WORKSPACE)
        );
        // Only the reporting claude is an agent; plain shells are not.
        assert_eq!(snap.agents.len(), 1);
        let claude = &snap.agents[0];
        assert_eq!(
            (claude.pane_id.as_str(), claude.agent.as_deref()),
            ("t13", Some("claude"))
        );
        assert_eq!(claude.agent_status, AgentStatus::Blocked);
        // The core sends "" for no title.
        assert_eq!(claude.title, None);
        assert_eq!(snap.panes[0].title.as_deref(), Some("user@box: ~"));

        // With the app's document, the labelled terminal joins its tab.
        let doc = AppDocument::parse(&json!({"spaces":[{"id":"s1","name":"shop",
            "tabs":[{"id":"k2","name":"web"}]}]}))
        .unwrap();
        let snap = build(
            &hello(),
            &subscribed.terminals,
            &doc,
            &mut LocalSeen::default(),
        );
        let t15 = snap.panes.iter().find(|p| p.pane_id == "t15").unwrap();
        assert_eq!(
            (t15.workspace_id.as_str(), t15.tab_id.as_str()),
            ("s1", "k2")
        );
    }
}

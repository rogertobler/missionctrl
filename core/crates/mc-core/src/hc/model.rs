//! Typed subset of herdr-core's API objects and of the Herdr app's layout
//! document.
//!
//! Field names follow herdr-core's OpenRPC schema (`schema/hc.openrpc.json`,
//! protocol generation 1) and herdr-app's `src/mirror.rs` and `src/layout.rs`
//! (both projects are Apache-2.0). Only fields MissionCtrl uses are modelled;
//! unknown fields are ignored, as the schema's "API only grows" rule allows.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result of `hello`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub version: String,
    /// Protocol generation; a real break needs a new one.
    pub generation: u32,
    #[serde(default)]
    pub core: CoreInfo,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CoreInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// One terminal record (`Terminal` in the schema).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Terminal {
    /// `t7` on this core, `machine/t7` on a machine this core added.
    pub address: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `live` or `finished`.
    #[serde(default)]
    pub state: String,
    /// The program's own word for what it is doing (OSC 7501 root record).
    #[serde(default)]
    pub status: Option<Status>,
    /// Whether the program ever reported a status.
    #[serde(default)]
    pub reported: bool,
    /// Goes up each time the terminal stops working.
    #[serde(default)]
    pub turn: u64,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub foreground: Option<Foreground>,
    #[serde(default)]
    pub status_ms: Option<u64>,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
    /// The terminal that started it, set once at start.
    #[serde(default)]
    pub parent: Option<String>,
    /// herdr-core ends it when its parent's program ends.
    #[serde(default)]
    pub die_with_parent: bool,
    /// Why it finished: exited, killed, spawn_failed, holder_lost … On a
    /// live terminal, `spawn_pending` marks a start that did not finish.
    #[serde(default)]
    pub reason: Option<String>,
    /// The program's process; missing until its start is confirmed.
    #[serde(default)]
    pub pid: Option<u32>,
}

impl Terminal {
    pub fn is_live(&self) -> bool {
        self.state == "live"
    }

    /// On a machine this core added (`machine/t7`), not on this core.
    pub fn is_remote(&self) -> bool {
        self.address.contains('/')
    }

    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// One OSC 7501 record. Words are the program's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// idle, working, done, blocked or error.
    pub state: String,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// permission, question or auth (blocked only).
    #[serde(default)]
    pub kind: Option<String>,
}

/// What runs in a terminal right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Foreground {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Result of `terminal.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub text: String,
    /// False when part of what was asked for may be missing.
    #[serde(default)]
    pub complete: bool,
    /// "screen", "history" or "stitched".
    #[serde(default)]
    pub how: String,
}

/// Result of `terminal.attach`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AttachResult {
    pub channel: u32,
    pub terminal: Terminal,
    #[serde(default)]
    pub resize_notifications: bool,
}

/// Result of `terminal.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalList {
    pub terminals: Vec<Terminal>,
}

/// Result of `events.subscribe` (plain mode: no channel).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscribed {
    pub seq: u64,
    pub terminals: Vec<Terminal>,
}

/// Result of `doc.get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocGet {
    /// `null` when never written or deleted.
    #[serde(default)]
    pub value: Value,
    /// Zero when never written; `doc.set` must name the current one.
    #[serde(default)]
    pub version: u64,
}

/// Name of the Herdr app's layout document in each core.
pub const APP_DOCUMENT: &str = "herdr-app";

/// The Herdr app's layout document (`Document` in herdr-app `src/mirror.rs`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppDocument {
    #[serde(default)]
    pub spaces: Vec<Space>,
    /// Top-level fields MissionCtrl does not read, kept for writing back.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub tabs: Vec<Tab>,
    /// Fields MissionCtrl does not read (e.g. `color`), kept for writing back.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    pub id: String,
    pub name: String,
    /// Split layout: a tree of `row`/`column`/`stack` containers and `block`
    /// leaves; a terminal block is `{"type":"block","content":{"terminal":"t7"}}`.
    /// Left out when missing: the Herdr app accepts no `root` but cannot read
    /// `"root": null`.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub root: Value,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl Tab {
    /// Terminal addresses in the layout, in tree order, then those of the
    /// tab's own floating panes.
    pub fn terminals(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_terminals(&self.root, &mut out);
        // Newer Herdr app builds keep tab-level floating panes next to the
        // root, each as `{"pane": node, "rect": [..]}`.
        if let Some(floating) = self.rest.get("floating").and_then(Value::as_array) {
            for float in floating {
                collect_terminals(float.get("pane").unwrap_or(float), &mut out);
            }
        }
        out
    }
}

/// A node's own block, the blocks floating over it (`floating`, herdr-app
/// `src/layout.rs`), then its children.
fn collect_terminals(node: &Value, out: &mut Vec<String>) {
    if let Some(t) = node.pointer("/content/terminal").and_then(Value::as_str) {
        out.push(t.to_owned());
    }
    if let Some(floating) = node.get("floating").and_then(Value::as_array) {
        for float in floating {
            if let Some(t) = float.pointer("/content/terminal").and_then(Value::as_str) {
                out.push(t.to_owned());
            }
        }
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_terminals(child, out);
        }
    }
}

impl AppDocument {
    /// Parse a document value; `None` when it is not in a shape this build
    /// understands.
    pub fn parse(value: &Value) -> Option<Self> {
        match serde_json::from_value(value.clone()) {
            Ok(doc) => Some(doc),
            Err(e) => {
                tracing::warn!(error = %e, "unreadable herdr-app document");
                None
            }
        }
    }
}

/// What a core holds as the Herdr app's layout document.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DocShape {
    /// Never written (or deleted): no Herdr app arranged this machine.
    #[default]
    Absent,
    /// Read and understood.
    Readable,
    /// Present but in a shape this build does not understand (e.g. written
    /// by a newer Herdr app). Shown as empty and never written back.
    Unreadable,
    /// The core has no `doc.get`.
    Unsupported,
}

/// The layout document with what is needed to write it back safely: the
/// version for `doc.set` (a revision counter, not a format version) and the
/// raw value as read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocState {
    pub doc: AppDocument,
    pub version: u64,
    pub raw: Value,
    pub shape: DocShape,
}

impl DocState {
    /// From the result of `doc.get`.
    pub fn from_get(get: DocGet) -> Self {
        if get.value.is_null() {
            return Self {
                version: get.version,
                ..Self::default()
            };
        }
        match AppDocument::parse(&get.value) {
            Some(doc) => Self {
                doc,
                version: get.version,
                raw: get.value,
                shape: DocShape::Readable,
            },
            None => Self {
                doc: AppDocument::default(),
                version: get.version,
                raw: get.value,
                shape: DocShape::Unreadable,
            },
        }
    }

    pub fn unsupported() -> Self {
        Self {
            shape: DocShape::Unsupported,
            ..Self::default()
        }
    }

    /// Whether MissionCtrl may write the document: only one it understood,
    /// or a fresh one where there is none.
    pub fn writable(&self) -> bool {
        matches!(self.shape, DocShape::Absent | DocShape::Readable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn document_keeps_unknown_fields_and_finds_terminals() {
        let value = json!({"spaces":[{"id":"s1","name":"work","folder":"/tmp/w","color":"teal",
            "tabs":[{"id":"k1","name":"","root":{"id":"n1","type":"row","children":[
                {"id":"n2","type":"block","content":{"terminal":"t1"}},
                {"id":"n3","type":"stack","children":[
                    {"id":"n4","type":"block","content":{"terminal":"t3"}},
                    {"id":"n5","type":"block","content":{"mod":"files"}}]}]}}]}],
            "future":{"x":1}});
        let doc = AppDocument::parse(&value).unwrap();
        assert_eq!(doc.spaces[0].rest["color"], "teal");
        assert_eq!(doc.spaces[0].tabs[0].terminals(), vec!["t1", "t3"]);
        // Round trip keeps unknown fields at every level.
        assert_eq!(serde_json::to_value(&doc).unwrap(), value);
    }

    #[test]
    fn floating_blocks_count() {
        let tab: Tab = serde_json::from_value(json!({"id":"k","root":{"id":"n1","type":"row",
            "floating":[{"id":"f1","content":{"terminal":"t9"},"edge":"right"}],
            "children":[{"id":"n2","type":"block","content":{"terminal":"t1"},
                "floating":[{"id":"f2","content":{"mod":"files"}}]}]}}))
        .unwrap();
        assert_eq!(tab.terminals(), vec!["t9", "t1"]);
    }

    #[test]
    fn tab_without_root_stays_without_root() {
        let tab: Tab = serde_json::from_value(json!({"id":"k","name":"x"})).unwrap();
        assert!(tab.terminals().is_empty());
        assert!(serde_json::to_value(&tab).unwrap().get("root").is_none());
    }

    #[test]
    fn document_states() {
        let absent = DocState::from_get(DocGet {
            value: Value::Null,
            version: 0,
        });
        assert_eq!(absent.shape, DocShape::Absent);
        assert!(absent.writable());
        let strange = DocState::from_get(DocGet {
            value: json!({"spaces": 3}),
            version: 7,
        });
        assert_eq!(strange.shape, DocShape::Unreadable);
        assert!(strange.doc.spaces.is_empty());
        assert_eq!((strange.version, &strange.raw), (7, &json!({"spaces": 3})));
        assert!(!strange.writable());
        let good = DocState::from_get(DocGet {
            value: json!({"spaces": []}),
            version: 2,
        });
        assert_eq!((good.shape.clone(), good.version), (DocShape::Readable, 2));
        assert!(!DocState::unsupported().writable());
    }
}

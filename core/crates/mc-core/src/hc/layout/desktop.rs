//! The layout document as the Herdr desktop app reads it, for checking only.
//!
//! Mirrors the `Deserialize` side of herdr-app `src/mirror.rs` (`Document`,
//! `SharedSpace`, `SharedTab`) and `src/layout.rs` (`Node` and the types in
//! it) at herdr-app a547b16, with the fields added up to 4deb605 (0.1.16)
//! (Apache-2.0): field names, defaults, tagging and untagged choices are the
//! same, so a value these types accept is one the desktop accepts. Older
//! builds ignore the newer fields, so accepting them in their own type is
//! safe for both. Nothing is written from them; edits stay on the raw JSON
//! so fields this mirror does not name survive.

#![allow(dead_code, clippy::large_enum_variant)]

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct Document {
    #[serde(default)]
    pub spaces: Vec<SharedSpace>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SharedSpace {
    id: String,
    name: String,
    folder: String,
    tabs: Vec<SharedTab>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct SharedTab {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    // The desktop defaults a missing root; an explicit `null` it refuses,
    // which the structural check in `check_writable` catches.
    #[serde(default)]
    root: Option<Node>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Terminal {
        terminal: String,
    },
    Mod {
        #[serde(rename = "mod")]
        name: String,
        #[serde(default)]
        props: Value,
    },
    Context {
        #[serde(alias = "context")]
        tab: String,
        #[serde(default)]
        pane: Option<PaneFrame>,
    },
    Agent {
        agent: String,
    },
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Edge {
    Top,
    #[default]
    Right,
    Bottom,
    Left,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Align {
    Start,
    #[default]
    Center,
    End,
}

#[derive(Deserialize)]
struct Float {
    id: String,
    content: Content,
    #[serde(default)]
    edge: Edge,
    #[serde(default)]
    align: Align,
    #[serde(default)]
    peek: Option<f32>,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Size {
    Px { px: f32 },
    Weight { weight: f32 },
}

#[derive(Deserialize)]
struct Line {
    #[serde(default = "one")]
    width: f32,
    #[serde(default)]
    color: String,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Background {
    Color(String),
    Gradient {
        gradient: [String; 2],
        #[serde(default = "one")]
        angle: f32,
    },
    Image {
        image: String,
        #[serde(default)]
        fit: Fit,
    },
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Fit {
    #[default]
    Cover,
    Contain,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Padding {
    All(f32),
    Axes([f32; 2]),
}

#[derive(Deserialize, Default)]
struct Look {
    #[serde(default)]
    theme: Option<ThemeChoice>,
    #[serde(default)]
    colors: Option<BTreeMap<String, String>>,
    #[serde(default)]
    font: Option<String>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    font_weight: Option<u16>,
    #[serde(default)]
    terminal_line_height: Option<f32>,
    #[serde(default)]
    terminal_cell_width: Option<f32>,
    #[serde(default)]
    terminal_scroll_speed: Option<f32>,
    #[serde(default)]
    ui_font: Option<String>,
    #[serde(default)]
    terminal_padding: Option<[f32; 2]>,
    #[serde(default)]
    terminal_opacity: Option<f32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ThemeChoice {
    One(String),
    Pair { light: String, dark: String },
}

#[derive(Deserialize)]
struct FocusLook {
    #[serde(default)]
    border: Option<Line>,
    #[serde(default)]
    background: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HeaderContent {
    Builtin(String),
    Mod {
        #[serde(rename = "mod")]
        name: String,
        #[serde(default)]
        props: Value,
    },
}

#[derive(Deserialize)]
struct Header {
    content: HeaderContent,
    #[serde(default)]
    reveal: Option<String>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default)]
    position: Option<String>,
}

#[derive(Deserialize, Default)]
struct PaneFrame {
    #[serde(default)]
    header: Option<Header>,
    #[serde(default)]
    background: Option<Background>,
    #[serde(default)]
    shadow: Option<String>,
    #[serde(default)]
    border: Option<Line>,
    #[serde(default)]
    focus: Option<Line>,
    #[serde(default)]
    radius: Option<f32>,
    #[serde(default)]
    gap: Option<f32>,
    #[serde(default)]
    lone: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Kind {
    Block {
        content: Content,
    },
    Row {
        children: Vec<Node>,
    },
    Column {
        children: Vec<Node>,
    },
    Stack {
        children: Vec<Node>,
        #[serde(default)]
        active: Option<String>,
        #[serde(default)]
        header: Option<Header>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Titlebar {
    System,
    #[serde(rename = "none")]
    Layout,
}

#[derive(Deserialize)]
struct ClonedFrom {
    name: String,
    version: String,
}

#[derive(Deserialize)]
pub struct Node {
    #[serde(default)]
    id: String,
    #[serde(default)]
    cloned_from: Option<ClonedFrom>,
    #[serde(flatten)]
    kind: Kind,
    #[serde(default)]
    size: Option<Size>,
    #[serde(default)]
    min: Option<f32>,
    #[serde(default)]
    max: Option<f32>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    background: Option<Background>,
    #[serde(default)]
    border: Option<Line>,
    #[serde(default)]
    radius: Option<f32>,
    #[serde(default)]
    shadow: Option<String>,
    #[serde(default)]
    padding: Option<Padding>,
    #[serde(default)]
    focus: Option<FocusLook>,
    #[serde(flatten)]
    look: Look,
    #[serde(default)]
    gap: Option<f32>,
    #[serde(default)]
    divider: Option<Line>,
    #[serde(default)]
    movable: Option<bool>,
    #[serde(default)]
    closable: Option<bool>,
    #[serde(default)]
    resizable: Option<bool>,
    #[serde(default)]
    floating: Vec<Float>,
    #[serde(default)]
    titlebar: Option<Titlebar>,
    #[serde(default)]
    traffic_lights: Option<[f32; 2]>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    terminal_menu: Option<String>,
    #[serde(default)]
    link_handlers: Vec<String>,
    #[serde(default)]
    toast: Option<String>,
    #[serde(default)]
    window_drag: bool,
    #[serde(default)]
    window_blur: bool,
}

/// A tab-level floating pane, `{"pane": Node, "rect": [x, y, w, h]}`: the
/// desktop up to 4deb605 keeps it as an unknown tab field without reading it.
#[derive(Deserialize)]
struct TabFloat {
    pane: Node,
    rect: [f32; 4],
}

/// Whether the desktop would read `value` as its layout document.
pub fn check(value: &Value) -> Result<(), String> {
    serde_json::from_value::<Document>(value.clone())
        .map(|_| ())
        .map_err(|e| format!("the Herdr app could not read it: {e}"))?;
    // Tab-level floating panes are kept but not read by the desktop up to
    // 4deb605; a malformed one is still not written next to.
    for space in value
        .get("spaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for tab in space
            .get("tabs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(floating) = tab.get("floating") else {
                continue;
            };
            serde_json::from_value::<Vec<TabFloat>>(floating.clone())
                .map_err(|e| format!("a tab's floating panes are unreadable: {e}"))?;
        }
    }
    Ok(())
}

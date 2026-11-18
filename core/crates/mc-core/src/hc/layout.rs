//! Changes to the Herdr app's layout document (`herdr-app`), as pure
//! functions on the raw JSON value.
//!
//! The document's shape follows herdr-app `src/mirror.rs` and `src/layout.rs`
//! (Apache-2.0): `{spaces:[{id,name,folder,tabs:[{id,name,root}]}]}`, where a
//! root is a tree of `row`/`column`/`stack` containers and `block` leaves.
//! Every function works on the value as read, so fields MissionCtrl does not
//! know survive its writes. [`check_writable`] decides whether a value may
//! be written at all: a shape the Herdr app could not read back must never
//! come from MissionCtrl.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

mod desktop;

/// herdr-core refuses a document larger than this as JSON (`doc.set`).
pub const MAX_BYTES: usize = 1024 * 1024;

/// A fresh id like the Herdr app's: `prefix` and 32 random hex digits.
pub fn fresh_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // No system randomness: fall back to the hasher's per-process keys.
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let n = std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish();
            chunk.copy_from_slice(&n.to_le_bytes()[..chunk.len()]);
        }
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}{hex}")
}

// ---------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------

/// Whether MissionCtrl may write `value` back: missing, `{}`, or a document
/// the Herdr app reads (every field it knows of the type it expects, see
/// [`desktop`]) whose structure MissionCtrl can edit safely (ids unique and
/// non-empty, every node of a known kind). Unknown fields are fine and kept;
/// unknown kinds are not, since writing next to them could leave a document
/// the Herdr app refuses. `Err` says what is wrong.
pub fn check_writable(value: &Value) -> Result<(), String> {
    let doc = match value {
        Value::Null => return Ok(()),
        Value::Object(doc) => doc,
        _ => return Err("the document is not an object".into()),
    };
    let Some(spaces) = doc.get("spaces") else {
        return if doc.is_empty() {
            Ok(())
        } else {
            Err("the document has no spaces but other fields".into())
        };
    };
    let spaces = spaces.as_array().ok_or("spaces is not a list")?;
    let mut space_ids = BTreeSet::new();
    let mut tab_ids = BTreeSet::new();
    for space in spaces {
        let space = space.as_object().ok_or("a space is not an object")?;
        let id = nonempty_id(space).ok_or("a space has no id")?;
        if !space_ids.insert(id) {
            return Err(format!("space id {id} appears twice"));
        }
        optional_string(space, "name")?;
        optional_string(space, "folder")?;
        let Some(tabs) = space.get("tabs") else {
            continue;
        };
        for tab in tabs.as_array().ok_or("tabs is not a list")? {
            let tab = tab.as_object().ok_or("a tab is not an object")?;
            let id = nonempty_id(tab).ok_or("a tab has no id")?;
            if !tab_ids.insert(id) {
                return Err(format!("tab id {id} appears twice"));
            }
            optional_string(tab, "name")?;
            if let Some(root) = tab.get("root") {
                check_node(root)?;
            }
        }
    }
    desktop::check(value)
}

fn nonempty_id(o: &Map<String, Value>) -> Option<&str> {
    o.get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn optional_string(o: &Map<String, Value>, key: &str) -> Result<(), String> {
    match o.get(key) {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(format!("{key} is not text")),
    }
}

/// A node as herdr-app `layout.rs` `Node` reads it.
fn check_node(node: &Value) -> Result<(), String> {
    let node = node.as_object().ok_or("a layout node is not an object")?;
    optional_string(node, "id")?;
    match node.get("type").and_then(Value::as_str) {
        Some("block") => check_content(node.get("content").ok_or("a block has no content")?)?,
        Some(kind @ ("row" | "column" | "stack")) => {
            let children = node
                .get("children")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("a {kind} has no children"))?;
            for child in children {
                check_node(child)?;
            }
            if kind == "stack" {
                optional_string(node, "active")?;
            }
        }
        Some(other) => return Err(format!("unknown layout node {other}")),
        None => return Err("a layout node has no type".into()),
    }
    optional_bool(node, "hidden")?;
    if let Some(floating) = node.get("floating") {
        for float in floating.as_array().ok_or("floating is not a list")? {
            check_float(float)?;
        }
    }
    Ok(())
}

fn optional_bool(o: &Map<String, Value>, key: &str) -> Result<(), String> {
    match o.get(key) {
        None | Some(Value::Bool(_)) => Ok(()),
        Some(_) => Err(format!("{key} is not true or false")),
    }
}

/// A floating block (herdr-app `Float`): an id and content are required.
fn check_float(float: &Value) -> Result<(), String> {
    let float = float
        .as_object()
        .ok_or("a floating block is not an object")?;
    match float.get("id") {
        Some(Value::String(_)) => {}
        _ => return Err("a floating block has no id".into()),
    }
    check_content(
        float
            .get("content")
            .ok_or("a floating block has no content")?,
    )?;
    one_of(float, "edge", &["top", "right", "bottom", "left"])?;
    one_of(float, "align", &["start", "center", "end"])?;
    optional_bool(float, "hidden")?;
    match float.get("peek") {
        None | Some(Value::Null) | Some(Value::Number(_)) => Ok(()),
        Some(_) => Err("peek is not a number".into()),
    }
}

fn one_of(o: &Map<String, Value>, key: &str, words: &[&str]) -> Result<(), String> {
    match o.get(key) {
        None => Ok(()),
        Some(Value::String(s)) if words.contains(&s.as_str()) => Ok(()),
        Some(_) => Err(format!("unknown {key}")),
    }
}

/// What a block shows (herdr-app `Content`): exactly one known kind.
fn check_content(content: &Value) -> Result<(), String> {
    let content = content
        .as_object()
        .ok_or("block content is not an object")?;
    let kinds = ["terminal", "mod", "tab", "context", "agent"]
        .iter()
        .filter(|k| content.contains_key(**k))
        .count();
    if kinds != 1 {
        return Err("unknown block content".into());
    }
    for key in ["terminal", "mod", "tab", "context", "agent"] {
        if let Some(v) = content.get(key)
            && !v.is_string()
        {
            return Err(format!("block content {key} is not text"));
        }
    }
    if let Some(pane) = content.get("pane")
        && !pane.is_object()
    {
        return Err("block content pane is not an object".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn spaces(doc: &Value) -> &[Value] {
    doc.get("spaces")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn tabs(space: &Value) -> &[Value] {
    space
        .get("tabs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn id_of(v: &Value) -> &str {
    v.get("id").and_then(Value::as_str).unwrap_or("")
}

/// Space ids in order.
pub fn space_ids(doc: &Value) -> Vec<String> {
    spaces(doc).iter().map(|s| id_of(s).to_owned()).collect()
}

/// Tab ids of `space` in order; `None` when there is no such space.
pub fn tab_ids(doc: &Value, space: &str) -> Option<Vec<String>> {
    let space = spaces(doc).iter().find(|s| id_of(s) == space)?;
    Some(tabs(space).iter().map(|t| id_of(t).to_owned()).collect())
}

/// The space a tab is in.
pub fn space_of_tab(doc: &Value, tab: &str) -> Option<String> {
    spaces(doc)
        .iter()
        .find(|s| tabs(s).iter().any(|t| id_of(t) == tab))
        .map(|s| id_of(s).to_owned())
}

pub fn has_space(doc: &Value, space: &str) -> bool {
    spaces(doc).iter().any(|s| id_of(s) == space)
}

/// Every node id (blocks, containers, floating blocks) and every terminal
/// address in a tree, floating blocks included.
fn walk(node: &Value, ids: &mut BTreeSet<String>, addresses: &mut BTreeSet<String>) {
    if let Some(id) = node.get("id").and_then(Value::as_str) {
        ids.insert(id.to_owned());
    }
    if let Some(t) = node.pointer("/content/terminal").and_then(Value::as_str) {
        addresses.insert(t.to_owned());
    }
    if let Some(floating) = node.get("floating").and_then(Value::as_array) {
        for float in floating {
            walk(float, ids, addresses);
        }
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            walk(child, ids, addresses);
        }
    }
}

/// What a tab or a space holds: tab ids, node ids and terminal addresses.
/// Closing compares this with what it saw first, so content added meanwhile
/// (a split, a mod, a tab) is never removed with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    pub tabs: BTreeSet<String>,
    pub nodes: BTreeSet<String>,
    pub addresses: BTreeSet<String>,
}

impl Image {
    /// Whether `self` holds something `before` did not.
    pub fn grew_since(&self, before: &Image) -> bool {
        !self.tabs.is_subset(&before.tabs)
            || !self.nodes.is_subset(&before.nodes)
            || !self.addresses.is_subset(&before.addresses)
    }
}

fn image_of_tab(tab: &Value, image: &mut Image) {
    image.tabs.insert(id_of(tab).to_owned());
    if let Some(root) = tab.get("root") {
        walk(root, &mut image.nodes, &mut image.addresses);
    }
    // Newer Herdr app builds keep tab-level floating panes here, each as
    // `{"pane": node, "rect": [..]}`.
    if let Some(floating) = tab.get("floating").and_then(Value::as_array) {
        for float in floating {
            walk(
                float.get("pane").unwrap_or(float),
                &mut image.nodes,
                &mut image.addresses,
            );
        }
    }
}

/// What a tab holds; `None` when it is not there.
pub fn tab_image(doc: &Value, tab: &str) -> Option<Image> {
    let tab = spaces(doc)
        .iter()
        .flat_map(tabs)
        .find(|t| id_of(t) == tab)?;
    let mut image = Image::default();
    image_of_tab(tab, &mut image);
    Some(image)
}

/// What a space holds; `None` when it is not there.
pub fn space_image(doc: &Value, space: &str) -> Option<Image> {
    let space = spaces(doc).iter().find(|s| id_of(s) == space)?;
    let mut image = Image::default();
    for tab in tabs(space) {
        image_of_tab(tab, &mut image);
    }
    Some(image)
}

/// Where a terminal stands in the layout: (space, tab).
pub fn placement_of(doc: &Value, address: &str) -> Option<(String, String)> {
    for space in spaces(doc) {
        for tab in tabs(space) {
            let mut image = Image::default();
            image_of_tab(tab, &mut image);
            if image.addresses.contains(address) {
                return Some((id_of(space).to_owned(), id_of(tab).to_owned()));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Changing
// ---------------------------------------------------------------------------

/// A target that is not in the document (any more).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gone;

/// The document as an object with a `spaces` list, made so when missing.
fn spaces_mut(doc: &mut Value) -> &mut Vec<Value> {
    if !doc.is_object() {
        *doc = json!({});
    }
    let obj = doc.as_object_mut().expect("just made an object");
    if !obj.get("spaces").is_some_and(Value::is_array) {
        obj.insert("spaces".into(), json!([]));
    }
    obj.get_mut("spaces")
        .and_then(Value::as_array_mut)
        .expect("just made a list")
}

fn space_mut<'a>(doc: &'a mut Value, space: &str) -> Option<&'a mut Value> {
    spaces_mut(doc).iter_mut().find(|s| id_of(s) == space)
}

fn tabs_mut(space: &mut Value) -> &mut Vec<Value> {
    let obj = space.as_object_mut().expect("a checked space is an object");
    if !obj.get("tabs").is_some_and(Value::is_array) {
        obj.insert("tabs".into(), json!([]));
    }
    obj.get_mut("tabs")
        .and_then(Value::as_array_mut)
        .expect("just made a list")
}

fn tab_mut<'a>(doc: &'a mut Value, tab: &str) -> Option<&'a mut Value> {
    spaces_mut(doc)
        .iter_mut()
        .flat_map(|s| {
            s.get_mut("tabs")
                .and_then(Value::as_array_mut)
                .map(|t| t.iter_mut())
                .into_iter()
                .flatten()
        })
        .find(|t| id_of(t) == tab)
}

/// Set `key` to `text` where it is empty or missing; true when it changed.
fn fill_in(o: &mut Value, key: &str, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let current = o.get(key).and_then(Value::as_str).unwrap_or("");
    if !current.is_empty() {
        return false;
    }
    o[key] = Value::from(text);
    true
}

/// A one-pane terminal block, as the Herdr app writes one.
fn terminal_block(pane: &str, address: &str) -> Value {
    json!({ "id": pane, "type": "block", "content": { "terminal": address } })
}

/// A new tab with one terminal pane, as herdr-app `Tab::new` writes it: the
/// root is a row whose id is the tab's.
pub fn new_tab(tab: &str, name: &str, pane: &str, address: &str) -> Value {
    json!({
        "id": tab,
        "name": name,
        "root": { "id": tab, "type": "row", "children": [terminal_block(pane, address)] },
    })
}

/// A new space with one tab, as herdr-app `add_space` writes it.
pub fn new_space(space: &str, name: &str, folder: &str, tab: Value) -> Value {
    json!({ "id": space, "name": name, "folder": folder, "tabs": [tab] })
}

/// What a create needs in the document.
#[derive(Debug, Clone)]
pub struct Placing<'a> {
    pub space: &'a str,
    /// Name and folder of a space this action creates (`None`: the space
    /// must already be there).
    pub new_space: Option<(&'a str, &'a str)>,
    pub tab: &'a str,
    pub tab_name: &'a str,
    pub pane: &'a str,
    pub address: &'a str,
}

/// Make sure the space and tab exist with `address` in them. Merges with
/// what is there: a terminal already in the layout gets no second pane, an
/// existing space or tab keeps its place and only gets a missing name or
/// folder. A space that must exist but does not is [`Gone`]: it is not
/// recreated. Returns whether anything changed.
pub fn place(doc: &mut Value, p: &Placing) -> Result<bool, Gone> {
    if let Some((_, tab)) = placement_of(doc, p.address) {
        if tab != p.tab {
            // Someone else placed it: leave it where it is.
            return Ok(false);
        }
        let mut changed = false;
        if let Some(t) = tab_mut(doc, p.tab) {
            changed |= fill_in(t, "name", p.tab_name);
        }
        if let (Some((name, folder)), Some(s)) = (p.new_space, space_mut(doc, p.space)) {
            changed |= fill_in(s, "name", name);
            changed |= fill_in(s, "folder", folder);
        }
        return Ok(changed);
    }
    if let Some(t) = tab_mut(doc, p.tab) {
        add_pane(t, p.tab, terminal_block(p.pane, p.address));
        fill_in(t, "name", p.tab_name);
        return Ok(true);
    }
    let tab = new_tab(p.tab, p.tab_name, p.pane, p.address);
    if let Some(s) = space_mut(doc, p.space) {
        if let Some((name, folder)) = p.new_space {
            fill_in(s, "name", name);
            fill_in(s, "folder", folder);
        }
        tabs_mut(s).push(tab);
        return Ok(true);
    }
    let Some((name, folder)) = p.new_space else {
        return Err(Gone);
    };
    spaces_mut(doc).push(new_space(p.space, name, folder, tab));
    Ok(true)
}

/// Add a block to an existing tab: into its root container, or around a
/// lone root block as a new row.
fn add_pane(tab: &mut Value, tab_id: &str, block: Value) {
    let obj = tab.as_object_mut().expect("a checked tab is an object");
    match obj.get_mut("root") {
        Some(root)
            if matches!(
                root.get("type").and_then(Value::as_str),
                Some("row" | "column" | "stack")
            ) =>
        {
            if let Some(children) = root.get_mut("children").and_then(Value::as_array_mut) {
                children.push(block);
            }
        }
        Some(root) if root.is_object() => {
            let old = root.take();
            *root = json!({ "id": fresh_id("s"), "type": "row", "children": [old, block] });
        }
        _ => {
            obj.insert(
                "root".into(),
                json!({ "id": tab_id, "type": "row", "children": [block] }),
            );
        }
    }
}

pub fn rename_space(doc: &mut Value, space: &str, name: &str) -> Result<bool, Gone> {
    let s = space_mut(doc, space).ok_or(Gone)?;
    Ok(set_name(s, name))
}

pub fn rename_tab(doc: &mut Value, tab: &str, name: &str) -> Result<bool, Gone> {
    let t = tab_mut(doc, tab).ok_or(Gone)?;
    Ok(set_name(t, name))
}

fn set_name(o: &mut Value, name: &str) -> bool {
    if o.get("name").and_then(Value::as_str) == Some(name) {
        return false;
    }
    o["name"] = Value::from(name);
    true
}

/// Where a moved space or tab goes: before another one, or to the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    Before(String),
    End,
}

impl Anchor {
    /// From an insertion boundary in the list as it was before the move
    /// (`index` = the place before the item now at `index`; moving one step
    /// down is `from + 2`).
    pub fn from_index(list: &[String], index: usize) -> Anchor {
        match list.get(index) {
            Some(id) => Anchor::Before(id.clone()),
            None => Anchor::End,
        }
    }
}

/// Why a move is not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotMoved {
    /// The item is gone.
    Gone,
    /// The item it was to go before is gone: where the user meant it to go
    /// is no longer clear (the end could even be the other way).
    AnchorGone,
}

impl From<Gone> for NotMoved {
    fn from(_: Gone) -> Self {
        NotMoved::Gone
    }
}

/// Move the item with `id` in `list` to `anchor`. True when the order
/// changed.
fn move_in(list: &mut Vec<Value>, id: &str, anchor: &Anchor) -> Result<bool, NotMoved> {
    let from = list
        .iter()
        .position(|v| id_of(v) == id)
        .ok_or(NotMoved::Gone)?;
    if matches!(anchor, Anchor::Before(a) if a == id) {
        return Ok(false);
    }
    if let Anchor::Before(a) = anchor
        && !list.iter().any(|v| id_of(v) == a)
    {
        return Err(NotMoved::AnchorGone);
    }
    let before: Vec<String> = list.iter().map(|v| id_of(v).to_owned()).collect();
    let item = list.remove(from);
    let to = match anchor {
        Anchor::Before(a) => list
            .iter()
            .position(|v| id_of(v) == a)
            .unwrap_or(list.len()),
        Anchor::End => list.len(),
    };
    list.insert(to, item);
    Ok(list.iter().map(id_of).ne(before.iter().map(String::as_str)))
}

pub fn move_space(doc: &mut Value, space: &str, anchor: &Anchor) -> Result<bool, NotMoved> {
    move_in(spaces_mut(doc), space, anchor)
}

pub fn move_tab(doc: &mut Value, tab: &str, anchor: &Anchor) -> Result<bool, NotMoved> {
    let space = space_of_tab(doc, tab).ok_or(Gone)?;
    let s = space_mut(doc, &space).ok_or(Gone)?;
    move_in(tabs_mut(s), tab, anchor)
}

/// Remove a tab; its space goes with it when it was the last tab, as the
/// Herdr app removes empty spaces. False when it was already gone.
pub fn remove_tab(doc: &mut Value, tab: &str) -> bool {
    let Some(space) = space_of_tab(doc, tab) else {
        return false;
    };
    let spaces = spaces_mut(doc);
    let Some(i) = spaces.iter().position(|s| id_of(s) == space) else {
        return false;
    };
    let tabs = tabs_mut(&mut spaces[i]);
    tabs.retain(|t| id_of(t) != tab);
    if tabs.is_empty() {
        spaces.remove(i);
    }
    true
}

/// Remove a space. False when it was already gone.
pub fn remove_space(doc: &mut Value, space: &str) -> bool {
    let spaces = spaces_mut(doc);
    let before = spaces.len();
    spaces.retain(|s| id_of(s) != space);
    spaces.len() != before
}

#[cfg(test)]
mod tests;

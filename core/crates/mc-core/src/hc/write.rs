//! Workspaces and tabs on herdr-core: changes to the Herdr app's layout
//! document plus the terminals in it.
//!
//! herdr-core only knows terminals; the Herdr desktop app keeps spaces and
//! tabs in the document `herdr-app` and adopts live terminals labelled
//! `herdr.space` that stand in no pane (herdr-app `window.rs`, `reconcile`).
//! A new tab therefore starts its terminal without `herdr.space` (only
//! `herdr.tab`, which doubles as the id of this start), writes the document,
//! and labels the terminal last: the desktop cannot adopt it as a nameless
//! space before the document names it. Closing ends the terminals first and
//! then edits the document, so no live labelled terminal is left for the
//! desktop to bring back.
//!
//! Every document write is a compare-and-swap (`doc.set` with the version
//! read) of a change applied to the document as read, so a desktop's own
//! changes survive; nothing is written in a shape the desktop could not read
//! ([`layout::check_writable`]). Terminal starts and kills are never blindly
//! repeated or undone: an unanswered call is looked up first.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::CoreClient;
use super::client::Endpoint;
use super::layout::{self, Anchor, Gone, Image, NotMoved, Placing};
use super::model::{APP_DOCUMENT, Terminal};
use super::snapshot::{SEEN_LABEL, SPACE_LABEL, TAB_LABEL};
use crate::error::{Error, LayoutProblem, Result, layout as layout_error};

/// Methods every change needs; without one of them nothing is started.
const WRITE_METHODS: &[&str] = &[
    "doc.get",
    "doc.set",
    "terminal.run",
    "terminal.kill",
    "terminal.label",
];

/// Tries of a document change against changes made elsewhere meanwhile.
const TRIES: usize = 3;

/// Looks for a start that did not answer, after it failed.
const LOOK_AGAIN: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];

/// After this long a start that did not answer and was never seen is taken
/// as never made. herdr-core creates the record as it reads the request
/// (`api/run.rs`), so one that arrived would long have shown.
const UNSETTLED_FOR: Duration = Duration::from_secs(60);

/// Size of a new terminal when the caller knows none.
pub const DEFAULT_SIZE: (u16, u16) = (80, 24);

/// How long to wait for a start that did not answer.
#[derive(Clone, Copy)]
struct Timing {
    look_again: [Duration; 2],
    unsettled_for: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            look_again: LOOK_AGAIN,
            unsettled_for: UNSETTLED_FOR,
        }
    }
}

/// Where a host keeps its current herdr-core client: empty while
/// disconnected, replaced on every reconnect.
pub type ClientSlot = Arc<RwLock<Option<Arc<CoreClient>>>>;

/// Keeps the open starts somewhere that outlives the process (the `mc`
/// command line's file); called after every change, with all of them.
pub type Persist = Arc<dyn Fn(&[UnsettledStart]) -> Result<()> + Send + Sync>;

/// Stands in for writing the backup file in tests.
#[cfg(test)]
type BackupHook = Arc<dyn Fn() -> Result<()> + Send + Sync>;

/// What a host keeps about its layout changes, across reconnects.
#[derive(Default)]
pub struct LayoutState {
    timing: Timing,
    /// Changes are allowed on this server ("Edit layout"); off by default.
    allowed: AtomicBool,
    /// The host's client, for looks that run after the action returned.
    /// Shared and replaceable: a host that is replaced (settings saved)
    /// hands this state to the new one, and later looks follow it there.
    slot: Arc<RwLock<Option<ClientSlot>>>,
    /// One change at a time per machine.
    lock: tokio::sync::Mutex<()>,
    /// Tab ids of starts in progress: their terminals are not shown as
    /// loose ones in between.
    pending: Mutex<BTreeSet<String>>,
    /// Starts that did not answer, by place (space id or [`NEW_SPACE`]).
    unsettled: Arc<Mutex<BTreeMap<String, Unsettled>>>,
    /// Where open starts are kept between processes, if anywhere.
    persist: Option<Persist>,
    /// The document was saved on the machine before the first write.
    backed_up: AtomicBool,
    /// Clearing a settled start from `persist` failed (kept for the caller).
    keep_failed: Mutex<Option<String>>,
    #[cfg(test)]
    backup_hook: Option<BackupHook>,
}

/// Place key for starts of a new workspace.
const NEW_SPACE: &str = "new space";

#[derive(Clone)]
struct Unsettled {
    tab: String,
    since: SystemTime,
}

/// A start that did not answer, as kept between runs of a short-lived
/// client (the `mc` command line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsettledStart {
    pub place: String,
    pub tab: String,
    /// When it was made, ms since the epoch.
    pub since_ms: u64,
}

impl LayoutState {
    /// A state on its own (no host): `allowed` turns changes on.
    pub fn new(allowed: bool) -> Self {
        let state = Self::default();
        state.allow(allowed);
        state
    }

    /// A host's state: later looks use whatever client `slot` then holds.
    pub fn for_host(slot: ClientSlot, allowed: bool) -> Self {
        let state = Self::new(allowed);
        state.attach(slot);
        state
    }

    /// Hand this state to a host that replaces the one it belonged to: its
    /// open starts, its lock (an action still running on the old host and
    /// a new one on this host take turns) and later looks all go on here.
    pub fn attach(&self, slot: ClientSlot) {
        if let Ok(mut s) = self.slot.write() {
            *s = Some(slot);
        }
    }

    /// Whether changes are allowed on this server.
    pub fn allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    pub fn allow(&self, on: bool) {
        self.allowed.store(on, Ordering::SeqCst);
    }

    /// Tab ids of starts in progress.
    pub fn pending(&self) -> BTreeSet<String> {
        self.pending.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn set_pending(&self, tab: &str, on: bool) {
        if let Ok(mut p) = self.pending.lock() {
            if on {
                p.insert(tab.to_owned());
            } else {
                p.remove(tab);
            }
        }
    }

    fn unsettled(&self, place: &str) -> Option<Unsettled> {
        self.unsettled.lock().ok()?.get(place).cloned()
    }

    /// Mark a start at `place` open (`Some(tab)`) or settled, and keep the
    /// open ones ([`Self::with_persist`]); an error says they could not be
    /// kept.
    fn set_unsettled(&self, place: &str, tab: Option<&str>) -> Result<()> {
        if let Ok(mut u) = self.unsettled.lock() {
            match tab {
                Some(tab) => {
                    u.insert(
                        place.to_owned(),
                        Unsettled {
                            tab: tab.to_owned(),
                            since: SystemTime::now(),
                        },
                    );
                }
                None => {
                    u.remove(place);
                }
            }
        }
        keep_unsettled(&self.unsettled, self.persist.as_ref())
    }

    /// Why clearing a settled start from `persist` failed, once. The record
    /// left behind is harmless (a terminal the layout holds is never ended
    /// when an old record is settled), but the caller should say so.
    pub fn take_keep_error(&self) -> Option<String> {
        self.keep_failed.lock().ok().and_then(|mut e| e.take())
    }

    /// Starts that did not answer and are not settled yet.
    pub fn unsettled_starts(&self) -> Vec<UnsettledStart> {
        let Ok(u) = self.unsettled.lock() else {
            return Vec::new();
        };
        listed(&u)
    }

    /// Keep open starts through `persist` after every change, so a process
    /// that ends (or is ended) right after a start leaves it on record.
    pub fn with_persist(mut self, persist: Persist) -> Self {
        self.persist = Some(persist);
        self
    }

    /// Take back starts kept by [`Self::unsettled_starts`].
    pub fn restore_unsettled(&self, starts: Vec<UnsettledStart>) {
        if let Ok(mut u) = self.unsettled.lock() {
            for start in starts {
                u.insert(
                    start.place,
                    Unsettled {
                        tab: start.tab,
                        since: UNIX_EPOCH + Duration::from_millis(start.since_ms),
                    },
                );
            }
        }
    }
}

fn listed(u: &BTreeMap<String, Unsettled>) -> Vec<UnsettledStart> {
    u.iter()
        .map(|(place, open)| UnsettledStart {
            place: place.clone(),
            tab: open.tab.clone(),
            since_ms: open
                .since
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
        .collect()
}

fn keep_unsettled(
    unsettled: &Mutex<BTreeMap<String, Unsettled>>,
    persist: Option<&Persist>,
) -> Result<()> {
    let Some(persist) = persist else {
        return Ok(());
    };
    let starts = unsettled.lock().map(|u| listed(&u)).unwrap_or_default();
    persist(&starts)
}

/// Changes only with "Edit layout" on: enforced here, not just in the app.
fn permit(state: &LayoutState) -> Result<()> {
    if state.allowed() {
        Ok(())
    } else {
        Err(Error::Unsupported {
            backend: "herdr-core".into(),
            what: "changing the layout (turn on layout editing for this server)".into(),
        })
    }
}

/// Whether a failed call may still have happened: the server never said no.
fn unanswered(e: &Error) -> bool {
    !matches!(
        e,
        Error::Herdr { .. }
            | Error::Unsupported { .. }
            | Error::Incompatible { .. }
            | Error::Layout { .. }
    )
}

async fn require_methods(client: &CoreClient) -> Result<()> {
    let hello = client.hello().await?;
    let missing: Vec<&str> = WRITE_METHODS
        .iter()
        .copied()
        .filter(|m| !hello.methods.iter().any(|h| h == m))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::Unsupported {
            backend: "herdr-core".into(),
            what: format!("changing workspaces and tabs (no {})", missing.join(", ")),
        })
    }
}

/// The document as read, checked for writing.
async fn read(client: &CoreClient) -> Result<(Value, u64)> {
    let got = client.doc_get(APP_DOCUMENT).await?;
    layout::check_writable(&got.value)
        .map_err(|why| layout_error(LayoutProblem::Unreadable, why))?;
    Ok((got.value, got.version))
}

/// The document as read, unchecked: only for looking things up.
async fn read_raw(client: &CoreClient) -> Result<Value> {
    Ok(client.doc_get(APP_DOCUMENT).await?.value)
}

/// Why a change is not made.
enum Refuse {
    Gone,
    /// The target holds something it did not when the action started.
    Grew,
    /// A move's anchor is gone.
    AnchorGone,
}

impl From<Gone> for Refuse {
    fn from(_: Gone) -> Self {
        Refuse::Gone
    }
}

impl From<NotMoved> for Refuse {
    fn from(n: NotMoved) -> Self {
        match n {
            NotMoved::Gone => Refuse::Gone,
            NotMoved::AnchorGone => Refuse::AnchorGone,
        }
    }
}

enum Written {
    /// Written now, or already so (nothing to write).
    Done,
    Refused(Refuse),
}

/// Apply `change` to the document as read and write it if it changed
/// anything, retrying against changes made elsewhere. A write that got no
/// answer is checked by reading back: when `change` finds nothing more to
/// do, it is in.
async fn write_with<F>(client: &CoreClient, state: &LayoutState, change: F) -> Result<Written>
where
    F: FnMut(&mut Value) -> std::result::Result<bool, Refuse>,
{
    write_checked(client, state, change, |_| std::future::ready(Ok(None))).await
}

/// [`write_with`], asking `check` before each write whether it may still go
/// (with the document as just read): `Some` refuses.
async fn write_checked<F, C, Fut>(
    client: &CoreClient,
    state: &LayoutState,
    mut change: F,
    mut check: C,
) -> Result<Written>
where
    F: FnMut(&mut Value) -> std::result::Result<bool, Refuse>,
    C: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Option<Refuse>>>,
{
    let (mut value, mut version) = read(client).await?;
    // Once a write went unanswered it may be in: from then on, not knowing
    // the outcome is never taken as "not written".
    let mut maybe_in = false;
    for _ in 0..TRIES {
        let mut next = value.clone();
        match change(&mut next) {
            Err(refuse) => return Ok(Written::Refused(refuse)),
            Ok(false) => return Ok(Written::Done),
            Ok(true) => {}
        }
        if let Some(refuse) = check(value.clone())
            .await
            .map_err(|e| unclear_if(maybe_in, e))?
        {
            return Ok(Written::Refused(refuse));
        }
        layout::check_writable(&next).map_err(|why| {
            unclear_if(
                maybe_in,
                layout_error(LayoutProblem::Unreadable, format!("would write: {why}")),
            )
        })?;
        let size = serde_json::to_vec(&next)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > layout::MAX_BYTES {
            return Err(unclear_if(
                maybe_in,
                layout_error(
                    LayoutProblem::TooLarge,
                    format!("the layout would grow to {size} bytes, over herdr-core's limit"),
                ),
            ));
        }
        back_up(client, state, &value, version)
            .await
            .map_err(|e| unclear_if(maybe_in, e))?;
        match client.doc_set(APP_DOCUMENT, &next, version).await {
            Ok(_) => return Ok(Written::Done),
            Err(e) if e.herdr_code() == Some("conflict") => {}
            Err(e) if unanswered(&e) => {
                tracing::info!(error = %e, "layout write unanswered; reading back");
                maybe_in = true;
            }
            Err(e) => return Err(unclear_if(maybe_in, e)),
        }
        (value, version) = read(client).await.map_err(|e| unclear_if(maybe_in, e))?;
    }
    // A last look: maybe the last try did get in.
    let mut last = value.clone();
    match change(&mut last) {
        Ok(false) => Ok(Written::Done),
        Err(refuse) => Ok(Written::Refused(refuse)),
        Ok(true) => Err(unclear_if(
            maybe_in,
            layout_error(
                LayoutProblem::Conflict,
                "the layout kept changing elsewhere",
            ),
        )),
    }
}

/// After a write that may be in, any failure to find out leaves the outcome
/// unclear: an unreadable read-back is no proof it is not in.
fn unclear_if(maybe_in: bool, e: Error) -> Error {
    if maybe_in {
        layout_error(
            LayoutProblem::Unclear,
            format!("a write did not answer: {e}"),
        )
    } else {
        e
    }
}

/// Before the first write of this app session, keep the document as it was
/// on the machine (`~/.cache/missionctrl/layout-backups/`), so it can be put
/// back (`mc doc restore`). Without that copy nothing is changed.
///
/// The file name names the herdr-core instance (its socket or connect
/// command) and the document (version and a hash of the content): several
/// cores under one account count their versions separately. An existing
/// copy is only reused when its content is exactly this document.
async fn back_up(
    client: &CoreClient,
    state: &LayoutState,
    value: &Value,
    version: u64,
) -> Result<()> {
    if value.is_null() || state.backed_up.load(Ordering::SeqCst) {
        return Ok(());
    }
    let data = serde_json::to_string_pretty(&json!({ "version": version, "value": value }))?;
    let instance = instance_tag(client.endpoint());
    let name = backup_name(&instance, version, &data);
    #[cfg(test)]
    if let Some(hook) = &state.backup_hook {
        hook().map_err(|e| {
            layout_error(LayoutProblem::Backup, format!("layout backup failed: {e}"))
        })?;
        state.backed_up.store(true, Ordering::SeqCst);
        return Ok(());
    }
    let transport = client.transport();
    // The newest copy with this name, if its content is this document (a
    // half-written file from a failed attempt is not).
    let existing = transport
        .exec_sh(&format!(
            "cd {BACKUP_DIR} 2>/dev/null && ls | grep -- '-{name}$' | sort -r | head -n 1 || true"
        ))
        .await
        .map(|out| out.trim().to_owned())
        .unwrap_or_default();
    let mut keep = String::new();
    if !existing.is_empty() && !existing.contains(['/', '\'', '\\']) {
        let same = transport
            .exec_sh(&format!("cd {BACKUP_DIR} && cat -- '{existing}'"))
            .await
            .is_ok_and(|text| text == data);
        if same {
            keep = existing;
        }
    }
    if keep.is_empty() {
        let path = transport
            .write_cache_file("layout-backups", &name, data.as_bytes())
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "layout backup failed");
                layout_error(LayoutProblem::Backup, format!("layout backup failed: {e}"))
            })?;
        tracing::info!(%path, "layout document backed up");
        keep = path.rsplit('/').next().unwrap_or_default().to_owned();
    }
    // Keep this instance's newest copies only, never the one this session
    // relies on. Best effort: a failure is logged, the change goes ahead.
    match transport
        .exec_sh(&prune_script(BACKUP_DIR, &instance, &keep))
        .await
    {
        Ok(out) => {
            if let Some(failed) = prune_failures(&out) {
                tracing::warn!(%failed, "pruning old layout backups failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "pruning old layout backups failed"),
    }
    state.backed_up.store(true, Ordering::SeqCst);
    Ok(())
}

/// Removes old backups of `instance` in `dir`: `keep` stays, and with it
/// the newest others up to [`BACKUPS_KEPT`] in all (names start with the
/// time in ms). It always ends
/// with one line `pruned <removed> <failed>`, `pruned - -` when `dir`
/// cannot be entered, or `pruned ? ?` when it cannot be listed (an empty
/// folder lists fine), so a failure shows even though the transport does
/// not report the exit status.
fn prune_script(dir: &str, instance: &str, keep: &str) -> String {
    format!(
        "if ! cd {dir} 2>/dev/null; then echo 'pruned - -'; \
         elif ! l=$(ls 2>/dev/null); then echo 'pruned ? ?'; \
         else printf '%s\\n' \"$l\" | grep -- '-{APP_DOCUMENT}-{instance}-v[0-9]*-[0-9a-f]*[.]json$' \
         | grep -v -x -F -- '{keep}' | sort -r | tail -n +{} \
         | {{ n=0; f=0; while IFS= read -r x; do if rm -f -- \"$x\" 2>/dev/null && [ ! -e \"$x\" ]; \
         then n=$((n+1)); else f=$((f+1)); fi; done; echo \"pruned $n $f\"; }}; fi",
        BACKUPS_KEPT
    )
}

/// What went wrong in a [`prune_script`] run, if anything.
fn prune_failures(out: &str) -> Option<String> {
    let Some(line) = out.lines().rev().find(|l| l.starts_with("pruned ")) else {
        return Some(format!("no answer: {}", out.trim()));
    };
    match line.split_whitespace().collect::<Vec<_>>()[..] {
        ["pruned", "-", "-"] => Some("the backup folder cannot be entered".into()),
        ["pruned", "?", "?"] => Some("the backup folder cannot be listed".into()),
        ["pruned", _, "0"] => None,
        ["pruned", _, failed] => Some(format!("{failed} old backups could not be removed")),
        _ => Some(format!("unexpected answer: {line}")),
    }
}

/// A short tag for one herdr-core instance: its socket path, or the command
/// that connects to it.
fn instance_tag(endpoint: &Endpoint) -> String {
    let key = match endpoint {
        Endpoint::Socket(path) => format!("socket:{path}"),
        Endpoint::Exec(command) => format!("exec:{command}"),
    };
    format!("{:08x}", fnv1a(key.as_bytes()) as u32)
}

/// The backup file name (before the time prefix) for this document.
fn backup_name(instance: &str, version: u64, data: &str) -> String {
    format!(
        "{APP_DOCUMENT}-{instance}-v{version}-{:016x}.json",
        fnv1a(data.as_bytes())
    )
}

/// 64-bit FNV-1a: a stable content hash for file names (not for security).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where layout backups go on the machine (as `write_cache_file` puts them).
const BACKUP_DIR: &str = "\"$HOME/.cache/missionctrl/layout-backups\"";

/// How many layout backups are kept per herdr-core instance.
const BACKUPS_KEPT: usize = 20;

// ---------------------------------------------------------------------------
// Starting
// ---------------------------------------------------------------------------

/// What runs in a new tab.
#[derive(Debug, Clone)]
pub enum Start {
    /// The machine's shell.
    Shell,
    /// A program (an agent) in the user's login shell, which stays when it
    /// ends: `argv` is never shell source.
    Program(Vec<String>),
    /// A shell, then this line typed into it.
    Line(String),
}

/// A new tab, in an existing workspace or a new one.
#[derive(Debug, Clone)]
pub struct NewTab {
    /// Existing workspace (space id); `None` makes a new one named `space_name`.
    pub space: Option<String>,
    pub space_name: String,
    pub tab_name: String,
    /// Absolute folder on the machine; empty = the core's own.
    pub cwd: String,
    pub start: Start,
    pub size: (u16, u16),
}

/// The new tab: its terminal and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub space: String,
    pub tab: String,
    pub address: String,
}

/// The argv that runs `command` in the user's login shell and leaves an
/// interactive shell when it ends.
///
/// The same approach as herdr-app's `remote_shell::launch` (Apache-2.0): a
/// fixed `sh` script picks `$SHELL`, loads its login and interactive
/// profiles (an agent installed through a profile's PATH is found), runs the
/// command from its arguments and then becomes an interactive shell. Only
/// the fixed script is shell source; the command stays separate arguments.
pub fn login_shell_argv(command: &[String]) -> Vec<String> {
    const SCRIPT: &str = r#"s=${SHELL:-/bin/sh}
n=${s##*/}
if [ "$n" = fish ]; then
  exec "$s" -l -i -c 'set -l sh $argv[1]; set -e argv[1]; command $argv; exec $sh -l -i' "$s" "$@"
fi
case "$n" in
  sh|bash|zsh|dash|ksh|mksh|ash) exec "$s" -l -i -c '"$@"; exec "$0" -l -i' "$s" "$@" ;;
esac
exec /bin/sh -c '"$@"; exec "${SHELL:-/bin/sh}" -i' sh "$@""#;
    let mut argv = vec!["sh".into(), "-c".into(), SCRIPT.into(), "sh".into()];
    argv.extend(command.iter().cloned());
    argv
}

/// Start a new tab: terminal, document, label.
pub async fn create(client: &Arc<CoreClient>, state: &LayoutState, new: NewTab) -> Result<Created> {
    permit(state)?;
    require_methods(client).await?;
    let _turn = state.lock.lock().await;
    let (doc, version) = read(client).await?;
    if let Some(space) = &new.space
        && !layout::has_space(&doc, space)
    {
        return Err(layout_error(LayoutProblem::Gone, "the workspace is gone"));
    }
    back_up(client, state, &doc, version).await?;
    let place = new.space.clone().unwrap_or_else(|| NEW_SPACE.to_owned());
    settle(client, state, &place).await?;

    let space = new.space.clone().unwrap_or_else(|| layout::fresh_id("sp"));
    let tab = layout::fresh_id("tb");
    let pane = layout::fresh_id("p");
    state.set_pending(&tab, true);
    let result = start_and_place(client, state, &new, &place, &space, &tab, &pane).await;
    state.set_pending(&tab, false);
    result
}

async fn start_and_place(
    client: &Arc<CoreClient>,
    state: &LayoutState,
    new: &NewTab,
    place: &str,
    space: &str,
    tab: &str,
    pane: &str,
) -> Result<Created> {
    let terminal = start(client, state, new, place, tab).await?;
    let folder = terminal
        .cwd
        .clone()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| new.cwd.clone());
    let address = terminal.address.clone();
    let placing = Placing {
        space,
        new_space: new
            .space
            .is_none()
            .then_some((new.space_name.as_str(), folder.as_str())),
        tab,
        tab_name: &new.tab_name,
        pane,
        address: &address,
    };
    let written = write_with(client, state, |doc| {
        layout::place(doc, &placing).map_err(Refuse::from)
    })
    .await;
    let failure = match written {
        Ok(Written::Done) => None,
        Ok(Written::Refused(_)) => Some(layout_error(LayoutProblem::Gone, "the workspace is gone")),
        Err(e) => Some(e),
    };
    if let Some(e) = failure {
        // A terminal the layout holds is never ended, whoever placed it: a
        // name or folder that did not get in is no reason to undo the tab.
        let placed = match read_raw(client).await {
            Ok(doc) => layout::placement_of(&doc, &address).is_some(),
            Err(_) => false,
        };
        if !placed {
            if matches!(
                e,
                Error::Layout {
                    problem: LayoutProblem::Unclear,
                    ..
                }
            ) {
                // It may stand in the layout: it stays.
                return Err(e);
            }
            end_started(client, state, place, tab, &address).await;
            return Err(e);
        }
        tracing::info!(terminal = %address, error = %e, "placed, though the layout write did not finish");
    }
    label_placed(client, space, tab, &address).await;
    if let Start::Line(line) = &new.start {
        client.send(&address, line, true).await?;
    }
    Ok(Created {
        space: space.to_owned(),
        tab: tab.to_owned(),
        address,
    })
}

/// End a terminal this action started and will not place. One that does not
/// end keeps its place blocked, so the next start there tries again first.
async fn end_started(
    client: &CoreClient,
    state: &LayoutState,
    place: &str,
    tab: &str,
    address: &str,
) {
    if !end(client, address).await {
        tracing::warn!(terminal = %address, "could not end a started terminal");
        if let Err(e) = state.set_unsettled(place, Some(tab)) {
            tracing::warn!(error = %e, "could not keep an open start");
        }
    }
}

/// Before a new start at `place`: an earlier start there that did not
/// answer must be settled. Found live → ended (the user saw it fail), and
/// settled only once it surely ended; found finished → settled; never seen
/// → settled only after [`UNSETTLED_FOR`]. Until then [`LayoutProblem::Busy`].
/// Only our own unplaced starts are ended: they never carry `herdr.space`
/// (it is added once they are placed). A found terminal that stands in the
/// layout, read again after the terminals were listed, did get placed: the
/// record is stale, and that terminal is never ended.
async fn settle(client: &CoreClient, state: &LayoutState, place: &str) -> Result<()> {
    let Some(open) = state.unsettled(place) else {
        return Ok(());
    };
    let found = client.terminals_labelled(&[(TAB_LABEL, &open.tab)]).await?;
    let doc = &if found.is_empty() {
        Value::Null
    } else {
        read_raw(client).await?
    };
    if found.is_empty() {
        let age = SystemTime::now()
            .duration_since(open.since)
            .unwrap_or(Duration::ZERO);
        if age < state.timing.unsettled_for {
            return Err(layout_error(
                LayoutProblem::Busy,
                "an earlier start here did not answer yet",
            ));
        }
        state.set_unsettled(place, None)?;
        return Ok(());
    }
    let mut running = Vec::new();
    for t in found.iter().filter(|t| ours_unplaced(doc, t)) {
        if !end(client, &t.address).await {
            running.push(t.address.clone());
        }
    }
    if !running.is_empty() {
        return Err(layout_error(
            LayoutProblem::Busy,
            format!("an earlier start here still runs: {}", running.join(", ")),
        ));
    }
    state.set_unsettled(place, None)?;
    Ok(())
}

/// `terminal.run` for a new tab, without `herdr.space`. A start that did
/// not answer is looked up by its `herdr.tab`.
async fn start(
    client: &Arc<CoreClient>,
    state: &LayoutState,
    new: &NewTab,
    place: &str,
    tab: &str,
) -> Result<Terminal> {
    let mut params = json!({
        "cwd": (!new.cwd.is_empty()).then_some(&new.cwd),
        "env": { "TERM": "xterm-256color", "COLORTERM": "truecolor" },
        "labels": { TAB_LABEL: tab, SEEN_LABEL: "0" },
        "keep": true,
        "size": { "cols": new.size.0.max(1), "rows": new.size.1.max(1) },
    });
    if let Start::Program(command) = &new.start {
        params["argv"] = json!(login_shell_argv(command));
    }
    // On record before it goes out: should this process end before the
    // answer, the next one still knows a start may arrive here.
    if let Err(e) = state.set_unsettled(place, Some(tab)) {
        // Not on record, so it does not go out (and blocks nothing).
        if let Ok(mut u) = state.unsettled.lock() {
            u.remove(place);
        }
        return Err(e);
    }
    let failed = match client.run_noting(params).await {
        Ok(t) => {
            settled(state, place);
            return startable(client, state, place, tab, t).await;
        }
        Err(failed) => failed,
    };
    // Never sent: the server saw nothing, so nothing can turn up later.
    if !failed.sent {
        settled(state, place);
        return Err(failed.error);
    }
    let failed = failed.error;
    // `spawn_failed` and lost answers can still leave a record.
    match client.terminals_labelled(&[(TAB_LABEL, tab)]).await {
        Ok(found) => {
            let t = found.iter().find(|t| t.is_live()).or(found.first());
            // Still starting (live, no process yet, no verdict): not placed
            // on a start that may yet fail; it stays open below.
            if let Some(t) = t.filter(|t| !starting(t)) {
                settled(state, place);
                return startable(client, state, place, tab, t.clone()).await;
            }
            if t.is_none() && !unanswered(&failed) {
                settled(state, place);
                return Err(failed);
            }
        }
        Err(e) => tracing::info!(error = %e, "looking up an unanswered start failed"),
    }
    // Not seen, maybe not made yet: it stays open; look again.
    look_again_later(state, client, place, tab);
    Err(layout_error(
        LayoutProblem::Unclear,
        format!("starting the terminal did not answer: {failed}"),
    ))
}

/// A live terminal of one of our starts that never got placed: in no pane
/// and without `herdr.space` (ours carry it only once placed; one that has
/// it belongs to a workspace, perhaps one the desktop is creating).
fn ours_unplaced(doc: &Value, t: &Terminal) -> bool {
    t.is_live() && t.label(SPACE_LABEL).is_none() && layout::placement_of(doc, &t.address).is_none()
}

/// The start at `place` has an outcome: no longer open.
fn settled(state: &LayoutState, place: &str) {
    if let Err(e) = state.set_unsettled(place, None) {
        tracing::warn!(error = %e, "could not keep the open starts");
        if let Ok(mut failed) = state.keep_failed.lock() {
            *failed = Some(e.to_string());
        }
    }
}

/// A record whose start herdr-core has not confirmed yet: live, without a
/// process and without a reason. It turns into a running program (pid set)
/// or a failed start (`spawn_pending`, or finished).
fn starting(t: &Terminal) -> bool {
    t.is_live() && t.pid.is_none() && t.reason.is_none()
}

/// A started terminal must run to be placed. A record herdr-core keeps
/// after its spawn timed out (live, reason `spawn_pending`) is a failed
/// start: it is ended, not placed.
async fn startable(
    client: &CoreClient,
    state: &LayoutState,
    place: &str,
    tab: &str,
    t: Terminal,
) -> Result<Terminal> {
    if t.is_live() && t.reason.as_deref() == Some("spawn_pending") {
        end_started(client, state, place, tab, &t.address).await;
        return Err(Error::Herdr {
            code: "spawn_failed".into(),
            message: format!("{} did not start", t.address),
        });
    }
    if t.is_live() {
        return Ok(t);
    }
    Err(Error::Herdr {
        code: t.reason.clone().unwrap_or_else(|| "finished".into()),
        message: format!("{} ended at once", t.address),
    })
}

/// Look for an unanswered start a little later and end it if it shows up:
/// the user was told it failed. Each look takes the host's client of the
/// moment (it may have reconnected); once everything found has surely
/// ended, the place is settled.
fn look_again_later(state: &LayoutState, client: &Arc<CoreClient>, place: &str, tab: &str) {
    let slot = state.slot.clone();
    let fallback = client.clone();
    let unsettled = state.unsettled.clone();
    let persist = state.persist.clone();
    let times = state.timing.look_again;
    let (place, tab) = (place.to_owned(), tab.to_owned());
    tokio::spawn(async move {
        let mut waited = Duration::ZERO;
        for at in times {
            tokio::time::sleep(at.saturating_sub(waited)).await;
            waited = at;
            let still_open = unsettled
                .lock()
                .is_ok_and(|u| u.get(&place).is_some_and(|o| o.tab == tab));
            if !still_open {
                return;
            }
            let host_slot = slot.read().ok().and_then(|s| s.clone());
            let client = match host_slot {
                Some(slot) => match slot.read().ok().and_then(|c| c.clone()) {
                    Some(c) => c,
                    None => continue,
                },
                None => fallback.clone(),
            };
            let Ok(found) = client.terminals_labelled(&[(TAB_LABEL, &tab)]).await else {
                continue;
            };
            if found.is_empty() {
                continue;
            }
            // A terminal the layout holds is never ended (it got placed).
            let Ok(doc) = read_raw(&client).await else {
                continue;
            };
            let mut ended = true;
            for t in found.iter().filter(|t| ours_unplaced(&doc, t)) {
                tracing::info!(terminal = %t.address, "late start ended");
                ended &= end(&client, &t.address).await;
            }
            if ended {
                if let Ok(mut u) = unsettled.lock()
                    && u.get(&place).is_some_and(|o| o.tab == tab)
                {
                    u.remove(&place);
                }
                if let Err(e) = keep_unsettled(&unsettled, persist.as_ref()) {
                    tracing::warn!(error = %e, "could not keep the open starts");
                }
                return;
            }
        }
    });
}

/// Label the placed terminal with its space, so the desktop treats it as
/// placed (its kill rule, and adoption should its pane go). Only while our
/// pane still holds it and nobody relabelled it: the desktop "parks" a
/// terminal by clearing its labels and removing its pane, and labelling it
/// after that would bring it back. Labels have no compare-and-swap; between
/// this check and the label a park can still slip in.
async fn label_placed(client: &CoreClient, space: &str, tab: &str, address: &str) {
    for attempt in 0..3 {
        if !still_ours(client, space, tab, address).await {
            return;
        }
        match client.label(address, &[(SPACE_LABEL, space)]).await {
            Ok(t) if t.label(SPACE_LABEL) == Some(space) => return,
            Ok(t) => {
                tracing::warn!(terminal = %address, labels = ?t.labels, "space label did not stick");
                return;
            }
            Err(e) if e.herdr_code() == Some("not_found") => return,
            Err(e) => {
                tracing::warn!(terminal = %address, attempt, error = %e, "labelling failed");
            }
        }
    }
}

async fn still_ours(client: &CoreClient, space: &str, tab: &str, address: &str) -> bool {
    let Ok((doc, _)) = read(client).await else {
        return false;
    };
    if layout::placement_of(&doc, address) != Some((space.to_owned(), tab.to_owned())) {
        return false;
    }
    match client.terminal(address).await {
        Ok(t) => {
            t.is_live()
                && t.label(TAB_LABEL) == Some(tab)
                && t.label(SPACE_LABEL).is_none_or(|s| s == space)
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Renaming and moving
// ---------------------------------------------------------------------------

async fn change_only<F>(client: &CoreClient, state: &LayoutState, change: F) -> Result<()>
where
    F: FnMut(&mut Value) -> std::result::Result<bool, Refuse>,
{
    permit(state)?;
    require_methods(client).await?;
    let _turn = state.lock.lock().await;
    match write_with(client, state, change).await? {
        Written::Done => Ok(()),
        Written::Refused(Refuse::AnchorGone) => Err(layout_error(
            LayoutProblem::Conflict,
            "what it was to go next to is gone",
        )),
        Written::Refused(_) => Err(layout_error(LayoutProblem::Gone, "it is gone")),
    }
}

pub async fn rename_space(
    client: &CoreClient,
    state: &LayoutState,
    space: &str,
    name: &str,
) -> Result<()> {
    change_only(client, state, |doc| {
        layout::rename_space(doc, space, name).map_err(Refuse::from)
    })
    .await
}

pub async fn rename_tab(
    client: &CoreClient,
    state: &LayoutState,
    tab: &str,
    name: &str,
) -> Result<()> {
    change_only(client, state, |doc| {
        layout::rename_tab(doc, tab, name).map_err(Refuse::from)
    })
    .await
}

/// Move a workspace before another one or to the end. The anchor comes from
/// the list the user saw, so a workspace added elsewhere meanwhile does not
/// shift where it lands.
pub async fn move_space(
    client: &CoreClient,
    state: &LayoutState,
    space: &str,
    anchor: &Anchor,
) -> Result<()> {
    change_only(client, state, |doc| {
        layout::move_space(doc, space, anchor).map_err(Refuse::from)
    })
    .await
}

/// Move a tab within its workspace (anchor as for [`move_space`]).
pub async fn move_tab(
    client: &CoreClient,
    state: &LayoutState,
    tab: &str,
    anchor: &Anchor,
) -> Result<()> {
    change_only(client, state, |doc| {
        layout::move_tab(doc, tab, anchor).map_err(Refuse::from)
    })
    .await
}

// ---------------------------------------------------------------------------
// Closing
// ---------------------------------------------------------------------------

/// A tab, a workspace or a loose terminal to close.
#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    Tab(&'a str),
    Space(&'a str),
    /// A terminal that stands in no workspace (the "Terminals" group).
    Terminal(&'a str),
}

impl Target<'_> {
    fn image(&self, doc: &Value) -> Option<Image> {
        match self {
            Target::Tab(t) => layout::tab_image(doc, t),
            Target::Space(s) => layout::space_image(doc, s),
            Target::Terminal(a) => Some(Image {
                addresses: [a.to_string()].into(),
                ..Image::default()
            }),
        }
    }

    /// Whether a terminal belongs here: by where it stands in the layout,
    /// by its labels when it stands nowhere. For a loose terminal: whatever
    /// stands nowhere and no workspace claims.
    fn holds(&self, doc: &Value, t: &Terminal) -> bool {
        match (layout::placement_of(doc, &t.address), self) {
            (Some((_, tab)), Target::Tab(id)) => tab == *id,
            (Some((space, _)), Target::Space(id)) => space == *id,
            (Some(_), Target::Terminal(_)) => false,
            (None, Target::Terminal(_)) => loose(doc, t),
            (None, Target::Tab(id)) => tab_claim(doc, t, id) == Claim::Held,
            (None, Target::Space(id)) => t.label(SPACE_LABEL) == Some(id),
        }
    }

    fn remove(&self, doc: &mut Value) -> bool {
        match self {
            Target::Tab(t) => layout::remove_tab(doc, t),
            Target::Space(s) => layout::remove_space(doc, s),
            Target::Terminal(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    /// Shown in the tab: closing it ends the terminal.
    Held,
    /// Not the tab's.
    Not,
    /// Its labels point at the tab but do not agree with where it is
    /// shown (a `herdr.tab` naming this tab from another workspace, or a
    /// tab of another workspace): closing the tab neither ends it nor goes
    /// ahead.
    Unsure,
    /// Shown in the tab only because its `herdr.tab` names a tab that is
    /// not in the layout: one the desktop is still writing, or a stale one
    /// nobody adopts. Closing the tab neither ends it nor goes ahead.
    Unplaced,
}

/// Whether a terminal standing in no pane belongs to tab `id`. The snapshot
/// shows such a terminal in the space its `herdr.space` names: in the tab
/// its `herdr.tab` names there, else in the space's first tab. Only a
/// terminal without `herdr.tab` falls back to the first tab; a `herdr.tab`
/// naming a tab that is not there may be one the desktop is still writing
/// (it starts the terminal before it writes the tab), so it is not ended,
/// not even with the space's only tab.
fn tab_claim(doc: &Value, t: &Terminal, id: &str) -> Claim {
    let named = t.label(TAB_LABEL);
    let Some(space) = t.label(SPACE_LABEL) else {
        // Shown among the loose terminals, whatever its tab label says.
        return Claim::Not;
    };
    let Some(tabs) = layout::tab_ids(doc, space) else {
        return Claim::Not;
    };
    if !tabs.iter().any(|tab| tab == id) {
        // Shown in another workspace; a tab label naming this tab disagrees.
        return if named == Some(id) {
            Claim::Unsure
        } else {
            Claim::Not
        };
    }
    match named {
        Some(tab) if tab == id => Claim::Held,
        Some(tab) if tabs.iter().any(|other| other == tab) => Claim::Not,
        Some(_) if tabs.first().map(String::as_str) != Some(id) => Claim::Not,
        Some(tab) if layout::space_of_tab(doc, tab).is_some() => Claim::Unsure,
        Some(_) => Claim::Unplaced,
        None if tabs.first().map(String::as_str) == Some(id) => Claim::Held,
        None => Claim::Not,
    }
}

/// A terminal that may be ended as a loose one: in no pane, and no space of
/// the layout claims it by label. Stricter than the snapshot, which also
/// lists a terminal labelled for a space without tabs as loose: the Herdr
/// app adopts such a terminal into a new tab before it writes the layout.
fn loose(doc: &Value, t: &Terminal) -> bool {
    layout::placement_of(doc, &t.address).is_none()
        && t.label(SPACE_LABEL)
            .is_none_or(|s| !layout::has_space(doc, s))
}

/// Labelled for a space of the layout that has no tab to show it in.
fn claimed_by_tabless_space(doc: &Value, t: &Terminal) -> bool {
    layout::placement_of(doc, &t.address).is_none()
        && t.label(SPACE_LABEL)
            .and_then(|s| layout::tab_ids(doc, s))
            .is_some_and(|tabs| tabs.is_empty())
}

/// Who closing a target ends: the terminals in its layout, live terminals
/// labelled for it, and their live descendants without a space of their own
/// that stand in no other pane (as the Herdr app's close does). Refused,
/// before anything ends, when one is on another machine or when herdr-core
/// would also end a terminal shown elsewhere (children marked
/// `die_with_parent` end with their parent, labels or not).
pub fn members(
    doc: &Value,
    image: &Image,
    terminals: &[Terminal],
    target: Target,
) -> Result<BTreeSet<String>> {
    let live: BTreeMap<&str, &Terminal> = terminals
        .iter()
        .filter(|t| t.is_live())
        .map(|t| (t.address.as_str(), t))
        .collect();
    let mut members: BTreeSet<String> = image.addresses.clone();
    if let Target::Tab(id) = target {
        for t in live.values() {
            if layout::placement_of(doc, &t.address).is_some() {
                continue;
            }
            match tab_claim(doc, t, id) {
                Claim::Unsure => {
                    return Err(layout_error(
                        LayoutProblem::Partial,
                        format!("{}: its labels disagree with where it is shown", t.address),
                    ));
                }
                Claim::Unplaced => {
                    return Err(layout_error(
                        LayoutProblem::Unplaced,
                        format!(
                            "{} is labelled for a tab that is not in the layout; \
                             close the workspace or open the Herdr app",
                            t.address
                        ),
                    ));
                }
                Claim::Held | Claim::Not => {}
            }
        }
    }
    if !matches!(target, Target::Terminal(_)) {
        // Live terminals in no pane that the target shows (by their labels).
        for t in live.values() {
            if layout::placement_of(doc, &t.address).is_none() && target.holds(doc, t) {
                members.insert(t.address.clone());
            }
        }
    }
    loop {
        let more: Vec<String> = live
            .values()
            .filter(|t| !members.contains(&t.address))
            .filter(|t| t.parent.as_deref().is_some_and(|p| members.contains(p)))
            .filter(|t| t.label(SPACE_LABEL).is_none())
            .filter(|t| layout::placement_of(doc, &t.address).is_none() || target.holds(doc, t))
            .map(|t| t.address.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        members.extend(more);
    }
    if let Some(remote) = members.iter().find(|a| a.contains('/')) {
        return Err(layout_error(
            LayoutProblem::OtherMachine,
            format!("{remote} is on another machine"),
        ));
    }
    // Everything herdr-core ends along with the members.
    let mut ending: BTreeSet<String> = members.clone();
    loop {
        let more: Vec<&Terminal> = live
            .values()
            .copied()
            .filter(|t| !ending.contains(&t.address))
            .filter(|t| t.die_with_parent)
            .filter(|t| t.parent.as_deref().is_some_and(|p| ending.contains(p)))
            .collect();
        if more.is_empty() {
            break;
        }
        for t in more {
            if !target.holds(doc, t) {
                return Err(layout_error(
                    LayoutProblem::ChildElsewhere,
                    format!("{} is shown elsewhere and would end too", t.address),
                ));
            }
            ending.insert(t.address.clone());
        }
    }
    Ok(members)
}

/// End `address`: done when it is finished or gone.
async fn end(client: &CoreClient, address: &str) -> bool {
    for _ in 0..2 {
        match client.kill(address).await {
            Ok(t) if !t.is_live() => return true,
            Ok(_) => match client.terminal(address).await {
                Ok(t) if !t.is_live() => return true,
                Err(e) if e.herdr_code() == Some("not_found") => return true,
                _ => {}
            },
            Err(e) if e.herdr_code() == Some("not_found") => return true,
            Err(e) => tracing::info!(terminal = %address, error = %e, "ending failed"),
        }
    }
    false
}

/// End every member; `Partial` names those still running.
async fn end_all(client: &CoreClient, members: &BTreeSet<String>) -> Result<()> {
    let mut running = Vec::new();
    for address in members {
        if !end(client, address).await {
            running.push(address.clone());
        }
    }
    if running.is_empty() {
        Ok(())
    } else {
        Err(layout_error(
            LayoutProblem::Partial,
            format!("still running: {}", running.join(", ")),
        ))
    }
}

/// Close a tab or a workspace: end its terminals, then take it out of the
/// layout, unless something was added to it meanwhile, in the layout or as a
/// live terminal that belongs to it (then it stays, with
/// [`LayoutProblem::Partial`]). Already gone counts as closed.
pub async fn close(client: &CoreClient, state: &LayoutState, target: Target<'_>) -> Result<()> {
    if let Target::Terminal(address) = target {
        return close_terminal(client, state, address).await;
    }
    permit(state)?;
    require_methods(client).await?;
    let _turn = state.lock.lock().await;
    let (doc, version) = read(client).await?;
    let Some(image) = target.image(&doc) else {
        return Ok(());
    };
    let terminals = client.terminals().await?;
    let first = members(&doc, &image, &terminals, target)?;
    back_up(client, state, &doc, version).await?;
    end_all(client, &first).await?;
    let first = &first;
    let written = write_checked(
        client,
        state,
        |doc| match target.image(doc) {
            None => Ok(false),
            Some(now) if now.grew_since(&image) => Err(Refuse::Grew),
            Some(_) => Ok(target.remove(doc)),
        },
        // Before each write: a terminal that joined meanwhile (labelled for
        // the target, or a child of one) keeps it.
        |doc| async move {
            let Some(image) = target.image(&doc) else {
                return Ok(None);
            };
            let terminals = client.terminals().await?;
            let now = members(&doc, &image, &terminals, target)?;
            let live: BTreeSet<&str> = terminals
                .iter()
                .filter(|t| t.is_live())
                .map(|t| t.address.as_str())
                .collect();
            let joined = now
                .iter()
                .any(|a| !first.contains(a) && live.contains(a.as_str()));
            Ok(joined.then_some(Refuse::Grew))
        },
    )
    .await?;
    match written {
        Written::Done | Written::Refused(Refuse::Gone) => Ok(()),
        Written::Refused(Refuse::Grew | Refuse::AnchorGone) => Err(layout_error(
            LayoutProblem::Partial,
            "something was added meanwhile and stays",
        )),
    }
}

/// End one loose terminal (the "Terminals" group), with the same checks as
/// closing a tab: it must still be loose and on this machine, and nothing
/// herdr-core would end with it may be shown in a workspace.
pub async fn close_terminal(client: &CoreClient, state: &LayoutState, address: &str) -> Result<()> {
    permit(state)?;
    require_methods(client).await?;
    let _turn = state.lock.lock().await;
    if address.contains('/') {
        return Err(layout_error(
            LayoutProblem::OtherMachine,
            format!("{address} is on another machine"),
        ));
    }
    let (doc, _) = read(client).await?;
    let terminals = client.terminals().await?;
    let Some(t) = terminals
        .iter()
        .find(|t| t.address == address && t.is_live())
    else {
        return Ok(());
    };
    if claimed_by_tabless_space(&doc, t) {
        return Err(layout_error(
            LayoutProblem::Claimed,
            format!("{address} is labelled for a workspace without tabs"),
        ));
    }
    if !loose(&doc, t) {
        return Err(layout_error(
            LayoutProblem::Placed,
            format!("{address} is no longer loose"),
        ));
    }
    let target = Target::Terminal(address);
    let image = target.image(&doc).unwrap_or_default();
    let members = members(&doc, &image, &terminals, target)?;
    end_all(client, &members).await
}

#[cfg(test)]
mod tests;

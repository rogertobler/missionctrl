//! Client for herdr-core's socket in plain mode: JSON-RPC 2.0, one message
//! per line (herdr-core `schema/README.md`, "Connecting").
//!
//! Each request opens a fresh stream through the [`Transport`], like the herdr
//! TUI client; `events.subscribe` keeps its stream open and then carries only
//! notifications.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};

use super::model::{
    APP_DOCUMENT, DocGet, DocState, Hello, ReadResult, Subscribed, Terminal, TerminalList,
};
use super::snapshot::SEEN_LABEL;
use crate::error::{Error, Result};
use crate::herdr::read_line;
use crate::transport::{BoxStream, RemoteEnv, Transport};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A failed [`CoreClient::run_noting`].
#[derive(Debug)]
pub struct RunFailed {
    pub error: Error,
    /// The request started going out: the server may have acted on it.
    pub sent: bool,
}
/// `terminal.send` waits while a person is typing in the terminal (a draft
/// holds it up to 60 s, herdr-core `input.rs`), so it gets longer.
const SEND_TIMEOUT: Duration = Duration::from_secs(90);
/// The protocol generation MissionCtrl speaks (herdr-core `schema/README.md`:
/// "A real break needs a new protocol generation").
pub const GENERATION: u32 = 1;
/// Methods the sync needs; a core without them is not usable.
const REQUIRED_METHODS: &[&str] = &["events.subscribe", "terminal.get", "terminal.list"];

/// How to reach a core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Its Unix socket, opened through the SSH connection.
    Socket(String),
    /// `herdr-core connect` run over SSH: the same protocol on stdin/stdout
    /// (herdr-core `schema/README.md`, "Connecting"). It finds the socket by
    /// the core's own rules (`HC_SOCKET`, `socket` in its config) and starts
    /// a core that is not running. The string is the shell command.
    Exec(String),
}

/// Printed by an [`Endpoint::Exec`] command right before `herdr-core
/// connect` takes over its stdout, so what the login prints first (motd,
/// `.bashrc` or fish greetings) is skipped.
const EXEC_MARK: &str = "@@missionctrl-hc@@";
/// Most bytes a login may print before [`EXEC_MARK`].
const EXEC_NOISE_MAX: usize = 64 * 1024;

impl Endpoint {
    async fn open(&self, transport: &Transport) -> Result<BoxStream> {
        match self {
            Endpoint::Socket(path) => transport.open_socket(path).await,
            Endpoint::Exec(command) => {
                // `echo` keeps the script free of backslashes (fish quoting,
                // see `Transport::exec_stream`).
                let stream = transport
                    .exec_stream(&format!("echo {EXEC_MARK}; {command}"))
                    .await?;
                skip_to_mark(stream).await
            }
        }
    }
}

/// Read past [`EXEC_MARK`] and its line end; bytes after it stay buffered
/// for the protocol. The mark counts also behind output without a line end
/// (`printf Welcome` in a login file). What comes before it is read in the
/// reader's chunks and never kept beyond a mark's length, so at most
/// [`EXEC_NOISE_MAX`] bytes are read and little is held.
async fn skip_to_mark(stream: BoxStream) -> Result<BoxStream> {
    use tokio::io::AsyncBufReadExt;
    let mark = EXEC_MARK.as_bytes();
    let mut reader = BufReader::new(stream);
    // The end of what came before, for a mark split across two chunks.
    let mut tail: Vec<u8> = Vec::new();
    let mut seen = 0usize;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Err(Error::Protocol(
                "herdr-core connect ended before it started".into(),
            ));
        }
        let mut joined = std::mem::take(&mut tail);
        let carried = joined.len();
        joined.extend_from_slice(chunk);
        let end = joined.windows(mark.len()).enumerate().find_map(|(at, w)| {
            if w != mark {
                return None;
            }
            // The line end after the mark: `\n`, maybe after `\r`.
            let rest = &joined[at + mark.len()..];
            match rest {
                [b'\n', ..] => Some(at + mark.len() + 1),
                [b'\r', b'\n', ..] => Some(at + mark.len() + 2),
                _ => None,
            }
        });
        if let Some(end) = end {
            reader.consume(end - carried);
            return Ok(Box::new(reader));
        }
        let len = chunk.len();
        reader.consume(len);
        seen += len;
        if seen > EXEC_NOISE_MAX {
            return Err(Error::Protocol(
                "the login printed too much before herdr-core connect".into(),
            ));
        }
        // Keep enough to find a mark (and its line end) that the next chunk
        // completes.
        let keep = joined.len().min(mark.len() + 1);
        tail = joined[joined.len() - keep..].to_vec();
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Socket(path) => f.write_str(path),
            Endpoint::Exec(command) => write!(f, "exec: {command}"),
        }
    }
}

/// Open a stream to `endpoint`.
pub(super) async fn open_endpoint(transport: &Transport, endpoint: &Endpoint) -> Result<BoxStream> {
    endpoint.open(transport).await
}

/// The shell command for [`Endpoint::Exec`] with the `herdr-core` binary at
/// `binary` (as `command -v` printed it). `None` when the path or the session
/// name has characters that would need quoting: over SSH the command is
/// itself quoted for the login shell, which may be fish, and must stay free
/// of backslashes ([`Transport::exec_stream`]).
pub fn connect_command(binary: &str, session: Option<&str>) -> Option<String> {
    let plain = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-+@:".contains(c))
    };
    if !plain(binary) || !session.is_none_or(plain) {
        return None;
    }
    let mut command = format!("exec {binary} connect");
    if let Some(session) = session {
        command.push_str(" --session ");
        command.push_str(session);
    }
    Some(command)
}

/// Whether [`CoreClient::connect`] may go on to `herdr-core connect` (which
/// starts a stopped core) after the sockets failed with `socket_error`: only
/// when the profile chose herdr-core (`start`), no socket was given, and no
/// socket hangs (a socket that hangs is a core that is there; starting
/// another beside it would not help). Every route to the exec endpoint in
/// `connect` passes here.
fn may_start(start: bool, socket_override: Option<&str>, socket_error: &Error) -> bool {
    start && socket_override.is_none() && socket_error.is_absent()
}

/// Where `herdr-core` is installed on the server: in the login's PATH, else
/// where its installer puts it (`~/.local/bin`). `None` when it is not.
/// `hc` is not tried: some window managers ship a different `hc`.
pub async fn find_core_binary(transport: &Transport) -> Result<Option<String>> {
    let out = transport
        .exec_sh(
            r#"p=$(command -v herdr-core 2>/dev/null) || p="$HOME/.local/bin/herdr-core"; [ -x "$p" ] && printf '%s' "$p"; true"#,
        )
        .await?;
    let path = out.trim();
    Ok((!path.is_empty()).then(|| path.to_owned()))
}

/// Candidate socket paths of a core, most likely first. Mirrors herdr-core's
/// own rules (`docs/security.md`, "State and endpoints"): the runtime dir if
/// set, else `/run/user/<uid>`, else the session's state directory.
pub fn socket_candidates(env: &RemoteEnv, session: Option<&str>) -> Vec<String> {
    let session = session.unwrap_or("default");
    let mut out = Vec::new();
    if let Some(run) = &env.xdg_runtime_dir {
        out.push(format!(
            "{}/herdr-core/{session}.sock",
            run.trim_end_matches('/')
        ));
    }
    if let Some(uid) = env.uid {
        out.push(format!("/run/user/{uid}/herdr-core/{session}.sock"));
    }
    let state = env
        .xdg_state_home
        .clone()
        .unwrap_or_else(|| format!("{}/.local/state", env.home.trim_end_matches('/')));
    out.push(format!(
        "{}/herdr-core/{session}/{session}.sock",
        state.trim_end_matches('/')
    ));
    out.dedup();
    out
}

pub struct CoreClient {
    transport: Arc<Transport>,
    endpoint: Endpoint,
    next_id: AtomicU64,
    /// Turns marked seen while the user looks ([`Self::mark_seen`]).
    marks: Arc<SeenMarks>,
    /// When a coloured read last found no end ([`Self::read_painted`]).
    unpainted: Mutex<Unpainted>,
}

/// How long a paint without end makes coloured reads go plain.
const UNPAINTED_FOR: Duration = Duration::from_secs(600);
/// Paints in a row that must come without their end before a core counts as
/// painting without it: one stall on a mobile link is no reason.
const UNPAINTED_AFTER: u32 = 2;

/// Coloured reads of a core lately: paints in a row without their end, and
/// since when they are given up ([`CoreClient::read_painted`]).
#[derive(Debug, Default)]
struct Unpainted {
    in_a_row: u32,
    since: Option<Instant>,
}

impl Unpainted {
    /// Paints that came but never ended, twice in a row: give up for a
    /// while. A link that brought nothing (a timeout) or any other failure
    /// says nothing about the core's paints.
    fn note<T>(&mut self, read: &Result<T>) {
        match read {
            Err(e) if super::attach::is_paint_without_end(e) => {
                self.in_a_row += 1;
                if self.in_a_row >= UNPAINTED_AFTER {
                    self.since = Some(Instant::now());
                    self.in_a_row = 0;
                }
            }
            Ok(_) => *self = Self::default(),
            Err(_) => {}
        }
    }
}

/// What the user looks at, for the shared label `herdr.seen=<turn>`
/// (herdr-app `docs/labels.md`): while a terminal is on screen with the app
/// in front, its turn is written once per turn, only to raise the label or
/// when it is missing, never to lower it. So another client's "mark unread"
/// stays while the user keeps looking; leaving the pane ([`Self::forget`])
/// and looking again raises it again.
///
/// Each look has a generation, taken when the look starts (synchronously,
/// before any network work, [`Self::show`]) and carried through every write
/// for it: a write still on its way when the user leaves is dropped, and
/// does not count for the next look. Marks only ever rise: a terminal state
/// older than what this look already handled writes nothing. The host keeps
/// one for its whole life, so a reconnect does not forget what was marked,
/// and a look that starts while offline is served once connected.
#[derive(Debug, Default)]
pub struct SeenMarks {
    looks: Mutex<HashMap<String, Look>>,
    /// The highest turn of each terminal handled so far, across looks: a
    /// state older than that (a lagging snapshot after leaving and looking
    /// again) never writes. A new look may still mark that same turn again.
    turns: Mutex<HashMap<String, u64>>,
    next_gen: AtomicU64,
    /// Check, label and note one at a time.
    writing: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Copy)]
struct Look {
    generation: u64,
    /// The highest turn written (or found already seen) during this look.
    marked: Option<u64>,
}

impl SeenMarks {
    /// The terminal is on screen now: a look starts unless one is running.
    /// Returns the look's generation.
    pub fn show(&self, terminal: &str) -> u64 {
        match self.looks.lock() {
            Ok(mut looks) => {
                looks
                    .entry(terminal.to_owned())
                    .or_insert_with(|| Look {
                        generation: self.next_gen.fetch_add(1, Ordering::Relaxed),
                        marked: None,
                    })
                    .generation
            }
            Err(_) => u64::MAX,
        }
    }

    /// The generation of the running look at `terminal`, if any. Never
    /// starts one: work that runs after the user left must not.
    pub fn look(&self, terminal: &str) -> Option<u64> {
        self.looks
            .lock()
            .ok()?
            .get(terminal)
            .map(|look| look.generation)
    }

    /// The terminal left the screen: the next look marks it again.
    pub fn forget(&self, terminal: &str) {
        if let Ok(mut looks) = self.looks.lock() {
            looks.remove(terminal);
        }
    }

    /// Terminals on screen now, with their looks' generations.
    pub fn shown(&self) -> Vec<(String, u64)> {
        self.looks
            .lock()
            .map(|l| {
                l.iter()
                    .map(|(t, look)| (t.clone(), look.generation))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The turn to write for `t` now within the look `generation`; `None`
    /// when that look is over, the turn is not above what the look already
    /// handled (also for an older state of the terminal arriving late), or
    /// the label already counts it as seen.
    fn due(&self, t: &Terminal, generation: u64) -> Option<u64> {
        let mut looks = self.looks.lock().ok()?;
        let look = looks.get_mut(&t.address)?;
        if look.generation != generation {
            return None;
        }
        {
            let mut turns = self.turns.lock().ok()?;
            let highest = turns.entry(t.address.clone()).or_insert(t.turn);
            if t.turn < *highest {
                return None;
            }
            *highest = t.turn;
        }
        if look.marked.is_some_and(|m| m >= t.turn) {
            return None;
        }
        match t.label(SEEN_LABEL).map(|v| v.trim().parse::<u64>()) {
            Some(Ok(seen)) if seen >= t.turn => {
                look.marked = Some(t.turn);
                None
            }
            _ => Some(t.turn),
        }
    }

    fn current(&self, terminal: &str, generation: u64) -> bool {
        self.look(terminal) == Some(generation)
    }

    fn done(&self, terminal: &str, generation: u64, turn: u64) {
        if let Ok(mut looks) = self.looks.lock()
            && let Some(look) = looks.get_mut(terminal)
            && look.generation == generation
        {
            look.marked = Some(look.marked.map_or(turn, |m| m.max(turn)));
        }
    }
}

impl CoreClient {
    /// Locate the core and verify it with `hello`: the socket override, else
    /// the default socket paths. With `start` (the profile chose herdr-core)
    /// and herdr-core installed, then `herdr-core connect`, which finds a
    /// socket by the core's own rules and starts a stopped core. Without
    /// `start` nothing is ever started.
    pub async fn connect(
        transport: Arc<Transport>,
        socket_override: Option<&str>,
        session: Option<&str>,
        start: bool,
    ) -> Result<(Self, Hello)> {
        let candidates = match socket_override {
            Some(path) => vec![path.to_owned()],
            None => socket_candidates(&transport.remote_env().await?, session),
        };
        let mut errors = Vec::new();
        for socket in candidates.clone() {
            match Self::connect_to(transport.clone(), Endpoint::Socket(socket.clone())).await {
                Ok(ok) => return Ok(ok),
                // A core that answers in a way we cannot use: no other path helps.
                Err(e @ Error::Incompatible { .. }) => return Err(e),
                Err(e) => {
                    tracing::debug!(%socket, error = %e, "herdr-core socket candidate failed");
                    errors.push(e);
                }
            }
        }
        let socket_error = crate::error::telling(errors)
            .unwrap_or_else(|| Error::Protocol("no socket candidates".into()));
        if !may_start(start, socket_override, &socket_error) {
            return Err(socket_error);
        }
        let command = match find_core_binary(&transport).await {
            Ok(binary) => binary.and_then(|b| connect_command(&b, session)),
            Err(e) => {
                tracing::debug!(error = %e, "looking for herdr-core failed");
                None
            }
        };
        let Some(command) = command else {
            return Err(socket_error);
        };
        let exec = Self::connect_to(transport.clone(), Endpoint::Exec(command)).await?;
        // `herdr-core connect` started the core: its socket answers now, and
        // a socket costs no login shell per request. Keep the command only
        // when the socket is somewhere this app cannot see (`HC_SOCKET`, the
        // core's own config).
        for socket in candidates {
            if let Ok(direct) = Self::connect_to(transport.clone(), Endpoint::Socket(socket)).await
            {
                return Ok(direct);
            }
        }
        Ok(exec)
    }

    /// Verify `endpoint` with `hello`.
    pub async fn connect_to(
        transport: Arc<Transport>,
        endpoint: Endpoint,
    ) -> Result<(Self, Hello)> {
        let client = Self {
            transport,
            endpoint,
            next_id: AtomicU64::new(1),
            marks: Arc::default(),
            unpainted: Mutex::new(Unpainted::default()),
        };
        let hello = client.hello().await?;
        Ok((client, hello))
    }

    /// Share `marks` with this client (the host's, kept across reconnects).
    pub fn with_marks(mut self, marks: Arc<SeenMarks>) -> Self {
        self.marks = marks;
        self
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn transport(&self) -> &Arc<Transport> {
        &self.transport
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Open a stream, send one request, read its response.
    async fn exchange(
        &self,
        id: u64,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<(BufReader<BoxStream>, Value)> {
        self.exchange_noting(id, method, params, deadline, None)
            .await
    }

    /// [`Self::exchange`]; `sent` turns true once the request starts going
    /// out (before that, a failure means the server never saw it).
    async fn exchange_noting(
        &self,
        id: u64,
        method: &str,
        params: Value,
        deadline: Duration,
        sent: Option<&AtomicBool>,
    ) -> Result<(BufReader<BoxStream>, Value)> {
        tokio::time::timeout(deadline, async {
            let mut stream = BufReader::new(self.endpoint.open(&self.transport).await?);
            let mut line = serde_json::to_vec(
                &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
            )?;
            line.push(b'\n');
            if let Some(sent) = sent {
                sent.store(true, Ordering::SeqCst);
            }
            stream.get_mut().write_all(&line).await?;
            stream.get_mut().flush().await?;
            let mut buf = Vec::new();
            loop {
                let line = read_line(&mut stream, &mut buf).await?.ok_or_else(|| {
                    Error::Protocol(format!("{method}: connection closed without response"))
                })?;
                // Notifications have no id; a response always has ours.
                if let Some(result) = parse_message(&line, id)? {
                    return Ok((stream, result));
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_within(method, params, REQUEST_TIMEOUT).await
    }

    async fn request_within(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value> {
        let id = self.next_id();
        let (_, result) = self.exchange(id, method, params, deadline).await?;
        Ok(result)
    }

    pub async fn request_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        Ok(serde_json::from_value(self.request(method, params).await?)?)
    }

    /// Handshake for [`GENERATION`]. A core of another generation, or one
    /// without the methods the sync needs, is [`Error::Incompatible`].
    pub async fn hello(&self) -> Result<Hello> {
        let hello: Hello = match self
            .request_as("hello", json!({ "generation": GENERATION }))
            .await
        {
            Err(e) if e.herdr_code() == Some("unsupported_generation") => {
                return Err(Error::Incompatible {
                    what: format!(
                        "herdr-core does not speak protocol generation {GENERATION}: {e}"
                    ),
                });
            }
            other => other?,
        };
        check_hello(&hello)?;
        Ok(hello)
    }

    pub async fn terminals(&self) -> Result<Vec<Terminal>> {
        Ok(self
            .request_as::<TerminalList>("terminal.list", json!({}))
            .await?
            .terminals)
    }

    pub async fn doc_get(&self, name: &str) -> Result<DocGet> {
        self.request_as("doc.get", json!({ "name": name })).await
    }

    /// The Herdr app's layout document with its version and shape.
    /// `has_doc`: whether the core offers `doc.get` (from `hello.methods`).
    pub async fn app_document(&self, has_doc: bool) -> Result<DocState> {
        if !has_doc {
            return Ok(DocState::unsupported());
        }
        Ok(DocState::from_get(self.doc_get(APP_DOCUMENT).await?))
    }

    /// Write a document if it is still at `version` (else `conflict`).
    /// Returns the new version.
    pub async fn doc_set(&self, name: &str, value: &Value, version: u64) -> Result<u64> {
        let result = self
            .request(
                "doc.set",
                json!({ "name": name, "value": value, "version": version }),
            )
            .await?;
        result
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Protocol("doc.set returned no version".into()))
    }

    /// Terminals carrying all of `labels`, finished ones included.
    pub async fn terminals_labelled(&self, labels: &[(&str, &str)]) -> Result<Vec<Terminal>> {
        let labels: serde_json::Map<String, Value> = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect();
        Ok(self
            .request_as::<TerminalList>("terminal.list", json!({ "all": true, "labels": labels }))
            .await?
            .terminals)
    }

    /// Start a terminal (`terminal.run`). Never retried: after a timeout or
    /// a lost connection it may have started anyway.
    pub async fn run(&self, params: Value) -> Result<Terminal> {
        self.request_as("terminal.run", params).await
    }

    /// [`Self::run`], telling on failure whether the request got as far as
    /// the server: one that never went out surely started nothing.
    pub async fn run_noting(&self, params: Value) -> std::result::Result<Terminal, RunFailed> {
        let sent = AtomicBool::new(false);
        let id = self.next_id();
        let answer = self
            .exchange_noting(id, "terminal.run", params, REQUEST_TIMEOUT, Some(&sent))
            .await
            .and_then(|(_, result)| Ok(serde_json::from_value::<Terminal>(result)?));
        answer.map_err(|error| RunFailed {
            error,
            sent: sent.load(Ordering::SeqCst),
        })
    }

    /// End a terminal's program; the record as it is afterwards.
    pub async fn kill(&self, terminal: &str) -> Result<Terminal> {
        self.request_as("terminal.kill", json!({ "terminal": terminal }))
            .await
    }

    pub async fn terminal(&self, terminal: &str) -> Result<Terminal> {
        self.request_as("terminal.get", json!({ "terminal": terminal }))
            .await
    }

    /// Paste `text` and submit it with the program's own Enter. `force` types
    /// even into a program that reports `blocked` (it may answer its dialog);
    /// without it the core refuses with `blocked`. Never retried: after a
    /// timeout or a lost connection the text may already be in, and sending it
    /// again would type it twice.
    pub async fn send(&self, terminal: &str, text: &str, force: bool) -> Result<()> {
        self.request_within(
            "terminal.send",
            json!({ "terminal": terminal, "text": text, "force": force }),
            SEND_TIMEOUT,
        )
        .await
        .map(drop)
    }

    /// Key names (`enter`, `esc`, `up`, `ctrl+c`, `home`, `pageup`, `a` …),
    /// encoded by the core for the program's current keyboard mode.
    pub async fn keys(&self, terminal: &str, keys: &[&str]) -> Result<()> {
        self.request(
            "terminal.keys",
            json!({ "terminal": terminal, "keys": keys }),
        )
        .await
        .map(drop)
    }

    /// The screen and `lines` lines of history above it, as plain text.
    pub async fn read(&self, terminal: &str, lines: u32) -> Result<ReadResult> {
        self.request_as(
            "terminal.read",
            json!({ "terminal": terminal, "lines": lines }),
        )
        .await
    }

    /// Set labels; an empty value removes one.
    pub async fn label(&self, terminal: &str, labels: &[(&str, &str)]) -> Result<Terminal> {
        let labels: serde_json::Map<String, Value> = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect();
        self.request_as(
            "terminal.label",
            json!({ "terminal": terminal, "labels": labels }),
        )
        .await
    }

    /// The user looks at `terminal` now: start a look (unless one runs) and
    /// write its turn if due. For tools; the app starts the look on the host
    /// first ([`SeenMarks::show`]) and then calls [`Self::mark_look`].
    pub async fn mark_seen(&self, terminal: &str) -> Result<()> {
        let generation = self.marks.show(terminal);
        self.mark_look(terminal, generation).await
    }

    /// Write the turn of `terminal` if the look `generation` still runs and
    /// its turn is due. Later turns are written by the host while the look
    /// lasts ([`Self::mark_due`]).
    pub async fn mark_look(&self, terminal: &str, generation: u64) -> Result<()> {
        if !self.marks.current(terminal, generation) {
            return Ok(());
        }
        let t = self.terminal(terminal).await?;
        self.mark_due(&t, generation).await
    }

    /// Write `herdr.seen` for `t` if the look `generation` still runs and its
    /// turn is due. Only agents: plain shells have no turns worth showing.
    pub async fn mark_due(&self, t: &Terminal, generation: u64) -> Result<()> {
        if super::snapshot::agent_of(t).is_none() {
            return Ok(());
        }
        let _one_at_a_time = self.marks.writing.lock().await;
        // Under the lock: another write may have done it meanwhile, the user
        // may have left, or `t` may be older than what was handled since.
        let Some(turn) = self.marks.due(t, generation) else {
            return Ok(());
        };
        self.label(&t.address, &[(SEEN_LABEL, &turn.to_string())])
            .await?;
        self.marks.done(&t.address, generation, turn);
        Ok(())
    }

    /// Whether `t` is on screen in the look `generation` and its turn still
    /// to be written.
    pub fn needs_mark(&self, t: &Terminal, generation: u64) -> bool {
        super::snapshot::agent_of(t).is_some() && self.marks.due(t, generation).is_some()
    }

    /// Terminals the user looks at now, with their looks' generations.
    pub fn shown(&self) -> Vec<(String, u64)> {
        self.marks.shown()
    }

    /// The terminal left the screen: the next look marks it again.
    pub fn forget_seen(&self, terminal: &str) {
        self.marks.forget(terminal);
    }

    /// Type text without submitting it (no Enter).
    pub async fn type_text(&self, terminal: &str, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let t = self.terminal(terminal).await?;
        super::attach::type_bytes(
            &self.transport,
            &self.endpoint,
            terminal,
            t.cols,
            t.rows,
            text.as_bytes(),
        )
        .await
    }

    /// The screen and up to `lines` lines of history above it with their
    /// colours (SGR codes), and whether the paint was complete.
    ///
    /// A paint that never ends (a core whose paint has no end marker, or a
    /// link too slow for [`super::attach::read_painted`]'s deadline) is
    /// remembered for [`UNPAINTED_FOR`]: meanwhile this fails at once, so the
    /// caller reads plain text without waiting the deadline every time.
    pub async fn read_painted(&self, terminal: &str, lines: u32) -> Result<super::attach::Painted> {
        let given_up = self
            .unpainted
            .lock()
            .is_ok_and(|u| u.since.is_some_and(|t| t.elapsed() < UNPAINTED_FOR));
        if given_up {
            return Err(Error::Protocol(
                "no coloured paint from this core lately".into(),
            ));
        }
        let read =
            super::attach::read_painted(&self.transport, &self.endpoint, terminal, lines).await;
        if let Ok(mut u) = self.unpainted.lock() {
            u.note(&read);
        }
        read
    }

    /// Subscribe to all terminals: returns the current ones and the stream of
    /// changes after them.
    pub async fn subscribe(&self) -> Result<(CoreSubscription, Subscribed)> {
        let id = self.next_id();
        let (stream, result) = self
            .exchange(id, "events.subscribe", json!({}), REQUEST_TIMEOUT)
            .await?;
        let subscribed: Subscribed = serde_json::from_value(result)?;
        Ok((
            CoreSubscription {
                stream,
                buf: Vec::new(),
            },
            subscribed,
        ))
    }
}

/// A notification on an `events.subscribe` stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    /// A fact changed (`type`: created, status, turn, labels, doc, …).
    Event {
        seq: u64,
        kind: String,
        terminal: Option<String>,
        data: Value,
    },
    /// Sent every 15 s on a quiet stream.
    Heartbeat { seq: u64 },
    /// The subscriber fell behind; a fresh list of terminals follows.
    Missed { seq: u64, terminals: Vec<Terminal> },
}

/// A live `events.subscribe` stream.
pub struct CoreSubscription {
    stream: BufReader<BoxStream>,
    /// Line read so far; kept so `next` is cancel-safe.
    buf: Vec<u8>,
}

impl CoreSubscription {
    /// Next notification; `Ok(None)` when the core closed the stream.
    /// Cancel-safe.
    pub async fn next(&mut self) -> Result<Option<Notification>> {
        loop {
            let Some(line) = read_line(&mut self.stream, &mut self.buf).await? else {
                return Ok(None);
            };
            if let Some(n) = parse_notification(&line)? {
                return Ok(Some(n));
            }
        }
    }
}

fn parse_notification(line: &str) -> Result<Option<Notification>> {
    let mut value: Value = serde_json::from_str(line)?;
    if let Some(error) = value.get("error") {
        return Err(rpc_error(error));
    }
    let seq = value
        .pointer("/params/seq")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut params = value.get_mut("params").map(Value::take).unwrap_or_default();
    Ok(match value.get("method").and_then(Value::as_str) {
        Some("event") => Some(Notification::Event {
            seq,
            kind: params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            terminal: params
                .get("terminal")
                .and_then(Value::as_str)
                .map(str::to_owned),
            data: params.get_mut("data").map(Value::take).unwrap_or_default(),
        }),
        Some("heartbeat") => Some(Notification::Heartbeat { seq }),
        Some("missed") => Some(Notification::Missed {
            seq,
            terminals: serde_json::from_value(
                params
                    .get_mut("terminals")
                    .map(Value::take)
                    .unwrap_or(json!([])),
            )?,
        }),
        // Unknown notifications may come with a newer core.
        _ => None,
    })
}

/// A response for `id` gives `Some(result)`; anything without an id (a
/// notification) gives `None`.
pub(super) fn parse_message(line: &str, id: u64) -> Result<Option<Value>> {
    let mut value: Value = serde_json::from_str(line)?;
    let Some(got) = value.get("id").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    if got.as_u64() != Some(id) {
        return Err(Error::Protocol(format!(
            "response id {got} does not match {id}"
        )));
    }
    if let Some(error) = value.get("error") {
        return Err(rpc_error(error));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .map(Some)
        .ok_or_else(|| Error::Protocol(format!("response without result: {line}")))
}

/// JSON-RPC error → typed error; `code` is herdr-core's stable `data.kind`.
fn rpc_error(error: &Value) -> Error {
    Error::Herdr {
        code: error
            .pointer("/data/kind")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| error.get("code").map(|c| c.to_string()))
            .unwrap_or_else(|| "unknown".into()),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

/// A `hello` this build can work with: generation [`GENERATION`] and the
/// methods the sync needs.
fn check_hello(hello: &Hello) -> Result<()> {
    if hello.generation != GENERATION {
        return Err(Error::Incompatible {
            what: format!(
                "herdr-core speaks protocol generation {}, MissionCtrl {GENERATION}",
                hello.generation
            ),
        });
    }
    let missing: Vec<&str> = REQUIRED_METHODS
        .iter()
        .copied()
        .filter(|m| !hello.methods.iter().any(|have| have == m))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Incompatible {
            what: format!("herdr-core lacks {}", missing.join(", ")),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_with(generation: u32, methods: &[&str]) -> Hello {
        serde_json::from_value(json!({"version":"0.1.20","generation":generation,
            "methods":methods}))
        .unwrap()
    }

    #[test]
    fn hello_needs_generation_and_methods() {
        let all = [
            "hello",
            "events.subscribe",
            "terminal.get",
            "terminal.list",
            "doc.get",
        ];
        assert!(check_hello(&hello_with(1, &all)).is_ok());
        assert!(matches!(
            check_hello(&hello_with(2, &all)),
            Err(Error::Incompatible { .. })
        ));
        assert!(matches!(
            check_hello(&hello_with(1, &["hello", "terminal.list"])),
            Err(Error::Incompatible { .. })
        ));
    }

    fn agent(turn: u64, seen: Option<&str>) -> Terminal {
        let mut labels = json!({});
        if let Some(seen) = seen {
            labels[SEEN_LABEL] = json!(seen);
        }
        serde_json::from_value(json!({"address": "t1", "state": "live", "turn": turn,
            "status": {"state": "idle", "app": "codex"}, "labels": labels}))
        .unwrap()
    }

    #[test]
    fn seen_follows_the_look() {
        let marks = SeenMarks::default();
        // Not on screen: nothing due.
        assert_eq!(marks.look("t1"), None);
        assert_eq!(marks.due(&agent(3, None), 0), None);
        let g = marks.show("t1");
        assert_eq!(marks.show("t1"), g, "a running look is kept");
        // Missing label: due, also at turn 0 (so terminals started elsewhere join).
        assert_eq!(marks.due(&agent(0, None), g), Some(0));
        let turn = marks.due(&agent(3, Some("2")), g).unwrap();
        assert_eq!(turn, 3);
        marks.done("t1", g, 3);
        // Once per turn: another client's "unread" (2) stays while looking.
        assert_eq!(marks.due(&agent(3, Some("2")), g), None);
        // The next turn is due again.
        assert_eq!(marks.due(&agent(4, Some("2")), g), Some(4));
        // A label at or above the turn counts as seen; never lowered.
        assert_eq!(marks.due(&agent(4, Some("7")), g), None);
        assert_eq!(marks.due(&agent(4, Some("2")), g), None);
    }

    #[test]
    fn a_write_after_leaving_does_not_count() {
        let marks = SeenMarks::default();
        let old = marks.show("t1");
        let turn = marks.due(&agent(3, None), old).unwrap();
        // The user leaves while the write is on its way, then looks again.
        marks.forget("t1");
        assert!(!marks.current("t1", old));
        let new = marks.show("t1");
        assert_ne!(old, new);
        marks.done("t1", old, turn);
        // The late write noted nothing for the new look: it is due again.
        assert_eq!(marks.due(&agent(3, Some("2")), new), Some(3));
        // And work scheduled for the old look finds nothing to do.
        assert_eq!(marks.due(&agent(3, Some("2")), old), None);
        assert_eq!(marks.shown(), vec![("t1".to_owned(), new)]);
    }

    #[test]
    fn a_late_older_state_never_lowers_the_mark() {
        let marks = SeenMarks::default();
        let g = marks.show("t1");
        // Turn 4 was written while an older state (turn 3, label 2) was on
        // its way: that state must not write 3 over 4.
        assert_eq!(marks.due(&agent(4, Some("2")), g), Some(4));
        marks.done("t1", g, 4);
        assert_eq!(marks.due(&agent(3, Some("2")), g), None);
        // A late "done" for the older turn keeps the higher mark.
        marks.done("t1", g, 3);
        assert_eq!(marks.due(&agent(4, Some("2")), g), None);
    }

    #[test]
    fn a_lagging_state_never_lowers_the_mark_across_looks() {
        let marks = SeenMarks::default();
        let first = marks.show("t1");
        assert_eq!(marks.due(&agent(4, Some("2")), first), Some(4));
        marks.done("t1", first, 4);
        // Leave and look again while the host's snapshot still has turn 3.
        marks.forget("t1");
        let again = marks.show("t1");
        assert_eq!(marks.due(&agent(3, Some("2")), again), None);
        // Another client marked turn 4 unread: the new look marks it again.
        assert_eq!(marks.due(&agent(4, Some("3")), again), Some(4));
    }

    #[test]
    fn work_after_leaving_never_starts_a_look() {
        let marks = SeenMarks::default();
        marks.show("t1");
        marks.forget("t1");
        // What runs after the user left asks, it does not start.
        assert_eq!(marks.look("t1"), None);
        assert!(marks.shown().is_empty());
    }

    #[tokio::test]
    async fn exec_endpoint_skips_what_the_login_prints() {
        // A login that greets on stdout before the command runs, with the
        // protocol's first bytes right behind the mark.
        let stream: BoxStream = Box::new(tokio::io::join(
            std::io::Cursor::new(
                b"Welcome to fish\nlast login\n@@missionctrl-hc@@\n{\"id\":1}\n".to_vec(),
            ),
            tokio::io::sink(),
        ));
        let mut rest = BufReader::new(skip_to_mark(stream).await.unwrap());
        let line = read_line(&mut rest, &mut Vec::new()).await.unwrap();
        assert_eq!(line.as_deref(), Some("{\"id\":1}"));
        // No mark at all: an error, not protocol garbage.
        let stream: BoxStream = Box::new(tokio::io::join(
            std::io::Cursor::new(b"motd only\n".to_vec()),
            tokio::io::sink(),
        ));
        assert!(skip_to_mark(stream).await.is_err());
    }

    async fn after_mark(bytes: Vec<u8>) -> Result<Option<String>> {
        let stream: BoxStream = Box::new(tokio::io::join(
            std::io::Cursor::new(bytes),
            tokio::io::sink(),
        ));
        let mut rest = BufReader::new(skip_to_mark(stream).await?);
        read_line(&mut rest, &mut Vec::new()).await
    }

    #[tokio::test]
    async fn the_mark_counts_after_output_without_a_line_end() {
        // `printf Welcome` in a login file: the mark shares its line.
        let got = after_mark(b"Welcome@@missionctrl-hc@@\n{\"id\":1}\n".to_vec()).await;
        assert_eq!(got.unwrap().as_deref(), Some("{\"id\":1}"));
        // CRLF line ends.
        let got = after_mark(b"motd\r\n@@missionctrl-hc@@\r\n{\"id\":2}\n".to_vec()).await;
        assert_eq!(got.unwrap().as_deref(), Some("{\"id\":2}"));
    }

    #[tokio::test]
    async fn a_mark_split_across_reads_is_found() {
        // The reader fills 8 KiB at a time: the mark straddles the border.
        let mut bytes = vec![b'x'; 8 * 1024 - 7];
        bytes.extend_from_slice(b"@@missionctrl-hc@@\n{\"id\":3}\n");
        let got = after_mark(bytes).await;
        assert_eq!(got.unwrap().as_deref(), Some("{\"id\":3}"));
    }

    #[tokio::test]
    async fn endless_output_before_the_mark_is_cut_off() {
        // No line end ever: still an error after the limit, not a growing
        // buffer.
        let got = after_mark(vec![b'x'; EXEC_NOISE_MAX * 4]).await;
        assert!(
            matches!(&got, Err(Error::Protocol(m)) if m.contains("too much")),
            "{got:?}"
        );
    }

    #[tokio::test]
    async fn exec_endpoint_runs_behind_a_noisy_shell() {
        // A real process through a shell that prints before the mark, as a
        // login with a chatty rc file does.
        let transport = Transport::Local(crate::transport::LocalTransport);
        let raw = transport
            .exec_stream(&format!(
                "echo greeting; echo noise; echo {EXEC_MARK}; exec cat"
            ))
            .await
            .unwrap();
        let mut stream = BufReader::new(skip_to_mark(raw).await.unwrap());
        stream.get_mut().write_all(b"ping\n").await.unwrap();
        stream.get_mut().flush().await.unwrap();
        let line = read_line(&mut stream, &mut Vec::new()).await.unwrap();
        assert_eq!(line.as_deref(), Some("ping"));
    }

    #[tokio::test]
    async fn a_missing_socket_is_absent() {
        // No socket: "not there" (so `auto` may look elsewhere), not a hang.
        let dir = std::env::temp_dir().join(format!("mc-test-{}-nosocket", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("none.sock");
        let transport = Arc::new(Transport::Local(crate::transport::LocalTransport));
        let got = CoreClient::connect(transport, Some(socket.to_str().unwrap()), None, false).await;
        assert!(
            matches!(got, Err(ref e) if e.is_absent()),
            "{:?}",
            got.err()
        );
    }

    #[test]
    fn connect_command_stays_plain() {
        assert_eq!(
            connect_command("/home/u/.local/bin/herdr-core", None).as_deref(),
            Some("exec /home/u/.local/bin/herdr-core connect")
        );
        assert_eq!(
            connect_command("/usr/bin/herdr-core", Some("work")).as_deref(),
            Some("exec /usr/bin/herdr-core connect --session work")
        );
        assert_eq!(connect_command("/opt/hc dir/herdr-core", None), None);
        assert_eq!(connect_command("/usr/bin/herdr-core", Some("a;b")), None);
    }

    #[test]
    fn candidates_follow_core_rules() {
        let env = RemoteEnv {
            home: "/home/u".into(),
            xdg_runtime_dir: Some("/tmp/run/".into()),
            uid: Some(1000),
            ..Default::default()
        };
        assert_eq!(
            socket_candidates(&env, None),
            vec![
                "/tmp/run/herdr-core/default.sock",
                "/run/user/1000/herdr-core/default.sock",
                "/home/u/.local/state/herdr-core/default/default.sock",
            ]
        );
        let env = RemoteEnv {
            home: "/home/u".into(),
            xdg_state_home: Some("/tmp/state".into()),
            ..Default::default()
        };
        assert_eq!(
            socket_candidates(&env, Some("work")),
            vec!["/tmp/state/herdr-core/work/work.sock"]
        );
    }

    #[test]
    fn parses_responses_and_errors() {
        let ok = r#"{"jsonrpc":"2.0","id":3,"result":{"value":null,"version":0}}"#;
        assert_eq!(parse_message(ok, 3).unwrap().unwrap()["version"], 0);
        assert!(parse_message(ok, 4).is_err());
        let note = r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":1}}"#;
        assert_eq!(parse_message(note, 3).unwrap(), None);
        let err = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32000,"message":"no terminal t9","data":{"kind":"not_found","hint":"h"}}}"#;
        let e = parse_message(err, 3).unwrap_err();
        assert_eq!(e.herdr_code(), Some("not_found"));
    }

    #[test]
    fn parses_notifications() {
        let event = r#"{"jsonrpc":"2.0","method":"event","params":{"seq":42,"type":"status","terminal":"t7","data":{"status":{"state":"working"}}}}"#;
        match parse_notification(event).unwrap().unwrap() {
            Notification::Event {
                seq,
                kind,
                terminal,
                data,
            } => {
                assert_eq!(
                    (seq, kind.as_str(), terminal.as_deref()),
                    (42, "status", Some("t7"))
                );
                assert_eq!(data["status"]["state"], "working");
            }
            other => panic!("{other:?}"),
        }
        let missed = r#"{"jsonrpc":"2.0","method":"missed","params":{"seq":57,"terminals":[{"address":"t1","state":"live"}]}}"#;
        assert!(matches!(
            parse_notification(missed).unwrap(),
            Some(Notification::Missed { seq: 57, ref terminals }) if terminals.len() == 1
        ));
        assert_eq!(
            parse_notification(r#"{"jsonrpc":"2.0","method":"later","params":{}}"#).unwrap(),
            None
        );
    }

    /// `herdr-core connect` speaks the protocol on stdin/stdout; here a shell
    /// stands in for it.
    #[tokio::test]
    async fn exec_endpoint_speaks_through_a_command() {
        let transport = Arc::new(Transport::Local(crate::transport::LocalTransport));
        let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"version":"0.1.20","generation":1,"methods":["events.subscribe","terminal.get","terminal.list"]}}"#;
        let command = format!("read line; printf '%s\\n' '{reply}'");
        let (client, hello) = CoreClient::connect_to(transport.clone(), Endpoint::Exec(command))
            .await
            .unwrap();
        assert_eq!(hello.version, "0.1.20");
        assert!(matches!(client.endpoint(), Endpoint::Exec(_)));
        // A command that is no core: closed without a reply.
        let failed = CoreClient::connect_to(transport, Endpoint::Exec("exit 127".into())).await;
        assert!(failed.is_err());
    }

    #[test]
    fn only_paints_without_their_end_twice_in_a_row_go_plain() {
        let no_end = || Err::<(), _>(Error::Protocol("paint without its end marker".into()));
        let mut u = Unpainted::default();
        u.note(&Err::<(), _>(Error::Timeout));
        u.note(&Err::<(), _>(Error::Timeout));
        assert!(u.since.is_none(), "a link that brought nothing is no sign");
        u.note(&no_end());
        assert!(u.since.is_none(), "one stall is no reason");
        u.note(&Ok(()));
        u.note(&no_end());
        assert!(u.since.is_none(), "a good paint starts the count over");
        u.note(&no_end());
        assert!(u.since.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_core_that_never_answers_is_asked_again() {
        let dir = std::env::temp_dir().join(format!("mc-test-{}-unpainted", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("default.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let accepted = Arc::new(AtomicU64::new(0));
        let counting = accepted.clone();
        // A core that accepts and never answers: no paint ever ends.
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                counting.fetch_add(1, Ordering::SeqCst);
                held.push(stream);
            }
        });
        let client = CoreClient {
            transport: Arc::new(Transport::Local(crate::transport::LocalTransport)),
            endpoint: Endpoint::Socket(socket.to_string_lossy().into_owned()),
            next_id: AtomicU64::new(1),
            marks: Arc::default(),
            unpainted: Mutex::new(Unpainted::default()),
        };
        assert!(matches!(
            client.read_painted("t1", 10).await,
            Err(Error::Timeout)
        ));
        // Nothing came: no sign of a core without the marker, so the next
        // read tries again.
        assert!(matches!(
            client.read_painted("t1", 10).await,
            Err(Error::Timeout)
        ));
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_core_profile_ever_starts_a_core() {
        let absent = Error::SocketUnavailable {
            path: "/tmp/x.sock".into(),
            reason: "no such file".into(),
        };
        // `auto` and detection pass `start = false`: never, whatever failed.
        assert!(!may_start(false, None, &absent));
        assert!(!may_start(false, None, &Error::Timeout));
        // A core profile: only when the sockets are surely not there.
        assert!(may_start(true, None, &absent));
        assert!(!may_start(true, None, &Error::Timeout));
        assert!(!may_start(true, Some("/tmp/x.sock"), &absent));
    }
}

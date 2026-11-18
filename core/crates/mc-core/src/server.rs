//! The actions on a pane that work on both servers: the herdr TUI's own
//! server and herdr-core. Pane ids are whatever the host's snapshot gives
//! (`w1:p1` on the TUI, the terminal address `t7` on herdr-core).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::hc::CoreClient;
use crate::herdr::HerdrClient;
use crate::herdr::model::PaneRead;
use crate::profile::Backend;
use crate::terminal::{TerminalSession, TerminalSink};
use crate::transport::Transport;

#[derive(Clone)]
pub enum ServerClient {
    Tui(Arc<HerdrClient>),
    Core(Arc<CoreClient>),
}

impl ServerClient {
    pub fn backend(&self) -> Backend {
        match self {
            ServerClient::Tui(_) => Backend::Tui,
            ServerClient::Core(_) => Backend::Core,
        }
    }

    /// The connection to the machine (uploads, directories, plans).
    pub fn transport(&self) -> &Arc<Transport> {
        match self {
            ServerClient::Tui(c) => c.transport(),
            ServerClient::Core(c) => c.transport(),
        }
    }

    /// The herdr TUI client, for actions herdr-core has no counterpart for
    /// yet (workspaces and tabs).
    pub fn tui(&self) -> Result<&Arc<HerdrClient>> {
        match self {
            ServerClient::Tui(c) => Ok(c),
            ServerClient::Core(_) => Err(Error::Unsupported {
                backend: "herdr-core".into(),
                what: "workspaces and tabs".into(),
            }),
        }
    }

    /// The last `lines` lines including history; with `ansi` with SGR colours.
    pub async fn read_pane_as(&self, pane_id: &str, lines: u32, ansi: bool) -> Result<PaneRead> {
        match self {
            ServerClient::Tui(c) => c.read_pane_as(pane_id, lines, ansi).await,
            ServerClient::Core(c) => core_read(c, pane_id, lines, ansi).await,
        }
    }

    /// Only what the pane shows now; never scrolls a full-screen program.
    pub async fn read_visible(&self, pane_id: &str, ansi: bool) -> Result<PaneRead> {
        match self {
            ServerClient::Tui(c) => c.read_visible(pane_id, ansi).await,
            ServerClient::Core(c) => core_read(c, pane_id, 0, ansi).await,
        }
    }

    /// Type text without Enter.
    pub async fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        match self {
            ServerClient::Tui(c) => c.send_text(pane_id, text).await,
            ServerClient::Core(c) => c.type_text(pane_id, text).await,
        }
    }

    /// Type text and submit it. On herdr-core the text goes in even when the
    /// agent reports `blocked` (`force`): every send in MissionCtrl is the user
    /// answering what they see, often exactly that dialog.
    pub async fn send_line(&self, pane_id: &str, text: &str) -> Result<()> {
        match self {
            ServerClient::Tui(c) => c.send_line(pane_id, text).await,
            ServerClient::Core(c) if text.is_empty() => c.keys(pane_id, &["enter"]).await,
            // The app wraps several lines as one bracketed paste for the TUI;
            // herdr-core pastes by itself (bracketed when the program asked).
            ServerClient::Core(c) => c.send(pane_id, unbracket(text), true).await,
        }
    }

    /// Key combos such as `enter`, `esc`, `ctrl+c`, `shift+tab`, `home`, `pageup`.
    pub async fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        match self {
            ServerClient::Tui(c) => c.send_keys(pane_id, keys).await,
            ServerClient::Core(c) => {
                for run in key_runs(keys) {
                    match run {
                        KeyRun::Bytes(bytes) => c.type_text(pane_id, &bytes).await?,
                        KeyRun::Named(names) => c.keys(pane_id, &names).await?,
                    }
                }
                Ok(())
            }
        }
    }

    /// The user has seen what the agent did. herdr-core keeps this as the
    /// shared label `herdr.seen`, so "done" clears on every client; the herdr
    /// TUI derives "done" itself, and there this does nothing. Starts a look
    /// if none runs (tools); the app uses [`Self::mark_look`].
    pub async fn mark_seen(&self, pane_id: &str) -> Result<()> {
        match self {
            ServerClient::Tui(_) => Ok(()),
            ServerClient::Core(c) => c.mark_seen(pane_id).await,
        }
    }

    /// Mark the pane's turn seen within the running look `generation`
    /// ([`crate::HostHandle::show_seen`]); nothing once the look is over.
    pub async fn mark_look(&self, pane_id: &str, generation: u64) -> Result<()> {
        match self {
            ServerClient::Tui(_) => Ok(()),
            ServerClient::Core(c) => c.mark_look(pane_id, generation).await,
        }
    }

    /// Control the pane as a live terminal of `cols × rows`.
    pub async fn open_terminal(
        &self,
        pane_id: &str,
        cols: u16,
        rows: u16,
        sink: Arc<dyn TerminalSink>,
    ) -> Result<TerminalSession> {
        match self {
            ServerClient::Tui(c) => {
                TerminalSession::open(c.transport(), c.socket_path(), pane_id, cols, rows, sink)
                    .await
            }
            ServerClient::Core(c) => {
                crate::hc::attach::open_session(
                    c.transport(),
                    c.endpoint(),
                    pane_id,
                    cols,
                    rows,
                    sink,
                )
                .await
            }
        }
    }
}

/// Safe bytes for a modified Enter, Backspace or Esc on herdr-core. herdr-core
/// encodes these as `CSI 27;m;k~` for a program that asked for no keyboard
/// mode (a shell leaves `;2;13~` on its line) and holds them until typing has
/// paused. These bytes go at once: Shift+Enter is LF (Ctrl+J), a new line in
/// agent prompts and Enter in a shell, but Ctrl+J in programs that bind it
/// (nano justifies, fzf and vim move down), and herdr-core's draft guard does
/// not count it as the end of a typed line; Alt+ is ESC before the key.
pub fn safe_chord(key: &str) -> Option<&'static str> {
    match key {
        "shift+enter" => Some("\n"),
        "alt+enter" => Some("\x1b\r"),
        "alt+backspace" => Some("\x1b\x7f"),
        "shift+backspace" => Some("\x7f"),
        "alt+esc" => Some("\x1b\x1b"),
        _ => None,
    }
}

/// Keys for herdr-core in order: runs of [`safe_chord`]s as bytes, the rest
/// by name.
#[derive(Debug, PartialEq)]
enum KeyRun<'a> {
    Bytes(String),
    Named(Vec<&'a str>),
}

fn key_runs<'a>(keys: &[&'a str]) -> Vec<KeyRun<'a>> {
    let mut runs: Vec<KeyRun<'a>> = Vec::new();
    for key in keys {
        match (safe_chord(key), runs.last_mut()) {
            (Some(b), Some(KeyRun::Bytes(bytes))) => bytes.push_str(b),
            (Some(b), _) => runs.push(KeyRun::Bytes(b.to_owned())),
            (None, Some(KeyRun::Named(names))) => names.push(key),
            (None, _) => runs.push(KeyRun::Named(vec![key])),
        }
    }
    runs
}

/// `text` without one bracketed paste around it (`ESC[200~ … ESC[201~`).
fn unbracket(text: &str) -> &str {
    text.strip_prefix("\x1b[200~")
        .and_then(|t| t.strip_suffix("\x1b[201~"))
        .unwrap_or(text)
}

/// The last `lines` lines of `text` (0: all), as the herdr TUI's `lines`
/// counts them: herdr-core counts history lines above the screen instead.
fn last_lines(text: &str, lines: u32) -> &str {
    if lines == 0 {
        return text;
    }
    // The line breaks; the last `lines` lines start after the break that
    // many from the end.
    let breaks: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    match breaks.len().checked_sub(lines as usize) {
        Some(at) => &text[breaks[at] + 1..],
        None => text,
    }
}

/// herdr-core has no revision counter for a pane's text; a hash of the text
/// serves the same purpose (skip redrawing what did not change).
async fn core_read(c: &CoreClient, pane_id: &str, lines: u32, ansi: bool) -> Result<PaneRead> {
    let painted = if ansi {
        match c.read_painted(pane_id, lines).await {
            Ok(painted) => Some(painted),
            // No complete paint (a very slow link, or a core whose paint has
            // no end marker): the plain text, whole but without colours,
            // rather than a cut-off picture.
            Err(e) => {
                tracing::debug!(pane = pane_id, error = %e, "coloured read failed; reading plain text");
                None
            }
        }
    } else {
        None
    };
    let (text, truncated) = if let Some(painted) = painted {
        (painted.text, !painted.complete)
    } else {
        let read = c.read(pane_id, lines).await?;
        (read.text, !read.complete)
    };
    let text = last_lines(&text, lines).to_owned();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    Ok(PaneRead {
        pane_id: pane_id.to_owned(),
        text,
        revision: hasher.finish(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::LocalTransport;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    type CallLog = Arc<Mutex<Vec<(String, Value)>>>;

    /// A core answering plain-mode requests and recording them. Its terminal
    /// `t1` is on turn 3; `herdr.seen` is whatever was labelled last.
    fn fake_core(name: &str) -> (String, CallLog) {
        let dir = std::env::temp_dir().join(format!("mc-test-{}-srv-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("default.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let log: CallLog = Arc::default();
        let seen: Arc<Mutex<Option<String>>> = Arc::default();
        let calls = log.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (calls, seen) = (calls.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let method = request["method"].as_str().unwrap_or_default().to_owned();
                    let params = request["params"].clone();
                    calls.lock().unwrap().push((method.clone(), params.clone()));
                    if method == "terminal.label" {
                        *seen.lock().unwrap() =
                            params["labels"]["herdr.seen"].as_str().map(str::to_owned);
                    }
                    let mut labels = json!({});
                    if let Some(s) = seen.lock().unwrap().clone() {
                        labels["herdr.seen"] = json!(s);
                    }
                    let terminal = json!({"address": "t1", "state": "live", "turn": 3,
                        "status": {"state": "idle", "app": "claude-code"},
                        "labels": labels, "cols": 80, "rows": 24});
                    let result = match method.as_str() {
                        "hello" => json!({"version": "0.1.20", "generation": 1,
                            "framed": false, "features": [],
                            "methods": ["events.subscribe", "terminal.get", "terminal.list"]}),
                        "terminal.get" | "terminal.label" => terminal,
                        "terminal.read" => {
                            json!({"text": "hello", "complete": false, "how": "history"})
                        }
                        _ => json!({"turn": 3}),
                    };
                    let reply = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
                });
            }
        });
        (socket.to_string_lossy().into_owned(), log)
    }

    async fn core(name: &str) -> (ServerClient, CallLog) {
        let (socket, log) = fake_core(name);
        let transport = Arc::new(Transport::Local(LocalTransport));
        let (client, _) = CoreClient::connect(transport, Some(&socket), None, false)
            .await
            .unwrap();
        (ServerClient::Core(Arc::new(client)), log)
    }

    fn methods(log: &Mutex<Vec<(String, Value)>>) -> Vec<(String, Value)> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m != "hello")
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn core_actions_map_to_its_methods() {
        let (server, log) = core("actions").await;
        server.send_line("t1", "go on").await.unwrap();
        server.send_line("t1", "").await.unwrap();
        server.send_keys("t1", &["home", "ctrl+c"]).await.unwrap();
        let read = server.read_pane_as("t1", 50, false).await.unwrap();
        assert_eq!((read.text.as_str(), read.truncated), ("hello", true));
        server.read_visible("t1", false).await.unwrap();
        let calls = methods(&log);
        assert_eq!(calls[0].0, "terminal.send");
        // Every send is the user's own answer: it goes in even when blocked.
        assert_eq!(calls[0].1["force"], true);
        assert_eq!(calls[0].1["text"], "go on");
        assert_eq!(
            (calls[1].0.as_str(), &calls[1].1["keys"]),
            ("terminal.keys", &json!(["enter"]))
        );
        assert_eq!(calls[2].1["keys"], json!(["home", "ctrl+c"]));
        assert_eq!(
            (calls[3].0.as_str(), &calls[3].1["lines"]),
            ("terminal.read", &json!(50))
        );
        assert_eq!(calls[4].1["lines"], 0);
    }

    #[test]
    fn safe_chords_go_as_bytes_in_order() {
        assert_eq!(
            key_runs(&[
                "home",
                "shift+enter",
                "alt+enter",
                "ctrl+enter",
                "shift+backspace"
            ]),
            vec![
                KeyRun::Named(vec!["home"]),
                KeyRun::Bytes("\n\x1b\r".into()),
                KeyRun::Named(vec!["ctrl+enter"]),
                KeyRun::Bytes("\x7f".into()),
            ]
        );
        assert_eq!(safe_chord("alt+esc"), Some("\x1b\x1b"));
        assert_eq!(safe_chord("alt+backspace"), Some("\x1b\x7f"));
        assert_eq!(safe_chord("ctrl+shift+enter"), None);
        assert_eq!(safe_chord("enter"), None);
    }

    #[tokio::test]
    async fn seen_is_marked_once_per_turn_while_looking() {
        let (server, log) = core("seen").await;
        server.mark_seen("t1").await.unwrap();
        let labelled = |log: &Mutex<Vec<(String, Value)>>| {
            methods(log)
                .iter()
                .filter(|(m, _)| m == "terminal.label")
                .count()
        };
        assert_eq!(labelled(&log), 1);
        assert_eq!(methods(&log).last().unwrap().1["labels"]["herdr.seen"], "3");
        let ServerClient::Core(c) = &server else {
            unreachable!()
        };
        // Another client marks it unread; still looking leaves that.
        c.label("t1", &[("herdr.seen", "")]).await.unwrap();
        server.mark_seen("t1").await.unwrap();
        assert_eq!(labelled(&log), 2, "only the other client's label call");
        // Leaving and looking again marks it again.
        c.forget_seen("t1");
        server.mark_seen("t1").await.unwrap();
        assert_eq!(labelled(&log), 3);
        assert_eq!(server.backend(), Backend::Core);
    }

    #[tokio::test]
    async fn seen_marks_outlive_the_client() {
        let (server, log) = core("seen-shared").await;
        let ServerClient::Core(c) = &server else {
            unreachable!()
        };
        let marks: Arc<crate::hc::SeenMarks> = Arc::default();
        let first = CoreClient::connect_to(c.transport().clone(), c.endpoint().clone())
            .await
            .unwrap()
            .0
            .with_marks(marks.clone());
        first.mark_seen("t1").await.unwrap();
        first.label("t1", &[("herdr.seen", "")]).await.unwrap();
        // A reconnect makes a new client with the host's marks.
        let again = CoreClient::connect_to(c.transport().clone(), c.endpoint().clone())
            .await
            .unwrap()
            .0
            .with_marks(marks);
        again.mark_seen("t1").await.unwrap();
        let labels = methods(&log)
            .iter()
            .filter(|(m, _)| m == "terminal.label")
            .count();
        assert_eq!(labels, 2, "the other client's unread survives a reconnect");
    }

    #[tokio::test]
    async fn core_sends_paste_by_themselves() {
        let (server, log) = core("paste").await;
        server
            .send_line("t1", "\x1b[200~one\ntwo\x1b[201~")
            .await
            .unwrap();
        assert_eq!(methods(&log)[0].1["text"], "one\ntwo");
    }

    #[test]
    fn lines_count_like_the_tui() {
        assert_eq!(last_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(last_lines("a\nb", 5), "a\nb");
        assert_eq!(last_lines("a\nb\nc", 0), "a\nb\nc");
        assert_eq!(last_lines("a\nb\nc", 3), "a\nb\nc");
        assert_eq!(unbracket("plain"), "plain");
    }
}

//! Client for herdr's newline-delimited JSON socket API.
//!
//! herdr answers one request per connection and then closes it; only
//! `events.subscribe` keeps its connection open. Every request therefore opens
//! a fresh stream through the [`Transport`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::model::{Event, PaneRead, Pong, SessionSnapshot};
use crate::error::{Error, Result};
use crate::transport::{BoxStream, RemoteEnv, Transport};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Pause between typed text and its Enter: Codex takes an Enter that follows a burst of
/// characters right away as part of a paste (a new line), not as "send" (seen with
/// codex-cli 0.163, 18.10.2026). Claude does not mind.
pub const ENTER_AFTER_TEXT: Duration = Duration::from_millis(250);
/// Most names [`HerdrClient::start_agent_free_name`] tries (`claude` … `claude-50`).
const AGENT_NAME_TRIES: u32 = 50;
/// Upper bound for a single response line (snapshots of large sessions).
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// xterm bytes for a key that herdr's `pane.send_keys` rejects (herdr 0.9.5: `unsupported key
/// home`; its names in `src/input/parse.rs` are only for herdr's own input), optionally after
/// `shift+`, `alt+`, `ctrl+`. `pane.send_text` writes them to the pane unchanged.
pub fn raw_key(combo: &str) -> Option<String> {
    let mut parts: Vec<&str> = combo.split('+').collect();
    let base = parts.pop()?;
    // xterm's modifier parameter: 1 + Shift 1, Alt 2, Ctrl 4.
    let mut modifier = 1;
    for part in parts {
        modifier += match part {
            "shift" => 1,
            "alt" => 2,
            "ctrl" => 4,
            _ => return None,
        };
    }
    let (number, last) = match base {
        "home" => (1, 'H'),
        "end" => (1, 'F'),
        "delete" => (3, '~'),
        "pageup" => (5, '~'),
        "pagedown" => (6, '~'),
        _ => return None,
    };
    Some(match (modifier, last) {
        (1, '~') => format!("\x1b[{number}~"),
        (1, _) => format!("\x1b[{last}"),
        _ => format!("\x1b[{number};{modifier}{last}"),
    })
}

/// Candidate API socket paths, most specific first. Mirrors herdr's own
/// resolution: `$XDG_CONFIG_HOME/herdr` if set, else `~/.config/herdr`.
pub fn socket_candidates(env: &RemoteEnv, session: Option<&str>) -> Vec<String> {
    let mut bases = Vec::new();
    if let Some(xdg) = &env.xdg_config_home {
        bases.push(format!("{}/herdr", xdg.trim_end_matches('/')));
    }
    bases.push(format!("{}/.config/herdr", env.home.trim_end_matches('/')));
    bases.dedup();
    bases
        .into_iter()
        .map(|base| match session {
            Some(name) => format!("{base}/sessions/{name}/herdr.sock"),
            None => format!("{base}/herdr.sock"),
        })
        .collect()
}

pub struct HerdrClient {
    transport: Arc<Transport>,
    socket: String,
    next_id: AtomicU64,
}

impl HerdrClient {
    /// Locate the API socket and verify it with `ping`.
    pub async fn connect(
        transport: Arc<Transport>,
        socket_override: Option<&str>,
        session: Option<&str>,
    ) -> Result<(Self, Pong)> {
        let candidates = match socket_override {
            Some(path) => vec![path.to_owned()],
            None => {
                let env = transport.remote_env().await?;
                tracing::debug!(home = %env.home, xdg = ?env.xdg_config_home, "remote environment");
                socket_candidates(&env, session)
            }
        };
        let mut last_error = None;
        for socket in candidates {
            let client = Self {
                transport: transport.clone(),
                socket,
                next_id: AtomicU64::new(1),
            };
            match client.ping().await {
                Ok(pong) => return Ok((client, pong)),
                Err(e) => {
                    tracing::debug!(socket = %client.socket, error = %e, "herdr socket candidate failed");
                    last_error = Some(e);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| Error::Protocol("no socket candidates".into())))
    }

    pub fn socket_path(&self) -> &str {
        &self.socket
    }

    pub fn transport(&self) -> &Arc<Transport> {
        &self.transport
    }

    fn next_id(&self) -> String {
        format!("mc_{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    async fn open(&self) -> Result<BufReader<BoxStream>> {
        Ok(BufReader::new(
            self.transport.open_socket(&self.socket).await?,
        ))
    }

    /// Open a stream, send one request and read its response, all within
    /// [`REQUEST_TIMEOUT`]. Returns the still open stream and `result`.
    async fn exchange(
        &self,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<(BufReader<BoxStream>, Value)> {
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut stream = self.open().await?;
            write_request(&mut stream, id, method, params).await?;
            let line = read_line(&mut stream, &mut Vec::new())
                .await?
                .ok_or_else(|| {
                    Error::Protocol(format!("{method}: connection closed without response"))
                })?;
            Ok((stream, parse_response(&line, id)?))
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Send one request and return its `result` object.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id();
        let (_, result) = self.exchange(&id, method, params).await?;
        Ok(result)
    }

    /// Like [`request`](Self::request), deserializing `result` into `T`.
    pub async fn request_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let result = self.request(method, params).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn ping(&self) -> Result<Pong> {
        self.request_as("ping", json!({})).await
    }

    pub async fn snapshot(&self) -> Result<SessionSnapshot> {
        let mut result = self.request("session.snapshot", json!({})).await?;
        let snapshot = result
            .get_mut("snapshot")
            .map(Value::take)
            .ok_or_else(|| Error::Protocol("session.snapshot: missing snapshot".into()))?;
        Ok(serde_json::from_value(snapshot)?)
    }

    /// Open an event subscription and wait for herdr's acknowledgement.
    pub async fn subscribe(&self, subscriptions: Vec<Value>) -> Result<Subscription> {
        let id = self.next_id();
        let (stream, result) = self
            .exchange(
                &id,
                "events.subscribe",
                json!({ "subscriptions": subscriptions }),
            )
            .await?;
        if result.get("type").and_then(Value::as_str) != Some("subscription_started") {
            return Err(Error::Protocol(format!(
                "unexpected subscription ack: {result}"
            )));
        }
        Ok(Subscription {
            stream,
            buf: Vec::new(),
            id,
        })
    }

    // -- Actions --------------------------------------------------------------

    /// Type text into a pane (no Enter appended).
    pub async fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        self.request(
            "pane.send_text",
            json!({ "pane_id": pane_id, "text": text }),
        )
        .await
        .map(drop)
    }

    /// Type text and press Enter after [`ENTER_AFTER_TEXT`], so every agent takes it as sent.
    pub async fn send_line(&self, pane_id: &str, text: &str) -> Result<()> {
        if !text.is_empty() {
            self.send_text(pane_id, text).await?;
            tokio::time::sleep(ENTER_AFTER_TEXT).await;
        }
        self.send_keys(pane_id, &["enter"]).await
    }

    /// Send key combos such as `enter`, `esc`, `ctrl+c` to a pane. Keys herdr has no name
    /// for ([`raw_key`]: `home`, `ctrl+end` …) go out as their bytes, in order.
    pub async fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let mut named: Vec<&str> = Vec::new();
        for key in keys {
            match raw_key(key) {
                Some(bytes) => {
                    if !named.is_empty() {
                        self.send_named_keys(pane_id, &named).await?;
                        named.clear();
                    }
                    self.send_text(pane_id, &bytes).await?;
                }
                None => named.push(key),
            }
        }
        if !named.is_empty() {
            self.send_named_keys(pane_id, &named).await?;
        }
        Ok(())
    }

    async fn send_named_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        self.request(
            "pane.send_keys",
            json!({ "pane_id": pane_id, "keys": keys }),
        )
        .await
        .map(drop)
    }

    /// Create a tab without stealing focus on other clients; returns
    /// `(tab_id, root_pane_id)`.
    pub async fn create_tab(
        &self,
        workspace_id: &str,
        label: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<(String, String)> {
        let result = self
            .request(
                "tab.create",
                json!({ "workspace_id": workspace_id, "label": label, "cwd": cwd, "focus": false }),
            )
            .await?;
        let tab = result.pointer("/tab/tab_id").and_then(Value::as_str);
        let pane = result.pointer("/root_pane/pane_id").and_then(Value::as_str);
        match (tab, pane) {
            (Some(tab), Some(pane)) => Ok((tab.to_owned(), pane.to_owned())),
            _ => Err(Error::Protocol(format!(
                "tab.create: unexpected result {result}"
            ))),
        }
    }

    /// Create a workspace without stealing focus; returns
    /// `(workspace_id, tab_id, root_pane_id)` of it and its first tab.
    pub async fn create_workspace(
        &self,
        label: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<(String, String, String)> {
        let result = self
            .request(
                "workspace.create",
                json!({ "label": label, "cwd": cwd, "focus": false }),
            )
            .await?;
        let ws = result
            .pointer("/workspace/workspace_id")
            .and_then(Value::as_str);
        let tab = result.pointer("/tab/tab_id").and_then(Value::as_str);
        let pane = result.pointer("/root_pane/pane_id").and_then(Value::as_str);
        match (ws, tab, pane) {
            (Some(ws), Some(tab), Some(pane)) => {
                Ok((ws.to_owned(), tab.to_owned(), pane.to_owned()))
            }
            _ => Err(Error::Protocol(format!(
                "workspace.create: unexpected result {result}"
            ))),
        }
    }

    pub async fn close_tab(&self, tab_id: &str) -> Result<()> {
        self.request("tab.close", json!({ "tab_id": tab_id }))
            .await
            .map(drop)
    }

    pub async fn close_workspace(&self, workspace_id: &str) -> Result<()> {
        self.request("workspace.close", json!({ "workspace_id": workspace_id }))
            .await
            .map(drop)
    }

    /// Start a managed agent (`kind` e.g. `claude`, `codex`) in a pane.
    /// `name` must be a lowercase slug (herdr rule).
    pub async fn start_agent(&self, pane_id: &str, kind: &str, name: &str) -> Result<()> {
        self.request(
            "agent.start",
            json!({ "pane_id": pane_id, "kind": kind, "name": name }),
        )
        .await
        .map(drop)
    }

    /// Start a `kind` agent under the first free name, `kind`, `kind-2`, `kind-3` …,
    /// and return that name. herdr names are unique across the server
    /// (`agent_name_taken`), and one `claude` is often running already.
    pub async fn start_agent_free_name(&self, pane_id: &str, kind: &str) -> Result<String> {
        let mut n = 1;
        loop {
            let name = if n == 1 {
                kind.to_owned()
            } else {
                format!("{kind}-{n}")
            };
            match self.start_agent(pane_id, kind, &name).await {
                Ok(()) => return Ok(name),
                Err(e) if e.herdr_code() == Some("agent_name_taken") && n < AGENT_NAME_TRIES => {
                    n += 1
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub async fn rename_workspace(&self, workspace_id: &str, label: &str) -> Result<()> {
        self.request(
            "workspace.rename",
            json!({ "workspace_id": workspace_id, "label": label }),
        )
        .await
        .map(drop)
    }

    pub async fn rename_tab(&self, tab_id: &str, label: &str) -> Result<()> {
        self.request("tab.rename", json!({ "tab_id": tab_id, "label": label }))
            .await
            .map(drop)
    }

    /// Move a tab to `index` (0-based) among its workspace's tabs; every
    /// client, the laptop included, shows the new order.
    pub async fn move_tab(&self, tab_id: &str, index: u32) -> Result<()> {
        self.request(
            "tab.move",
            json!({ "tab_id": tab_id, "insert_index": index }),
        )
        .await
        .map(drop)
    }

    /// Move a workspace to `index` (0-based) in the workspace list.
    pub async fn move_workspace(&self, workspace_id: &str, index: u32) -> Result<()> {
        self.request(
            "workspace.move",
            json!({ "workspace_id": workspace_id, "insert_index": index }),
        )
        .await
        .map(drop)
    }

    /// Read the last `lines` lines of a pane (scrollback included) as plain text.
    pub async fn read_pane(&self, pane_id: &str, lines: u32) -> Result<PaneRead> {
        self.read_pane_as(pane_id, lines, false).await
    }

    /// Like [`read_pane`](Self::read_pane); with `ansi` the text keeps its
    /// SGR colour codes (`format: "ansi"`).
    pub async fn read_pane_as(&self, pane_id: &str, lines: u32, ansi: bool) -> Result<PaneRead> {
        self.read_source(pane_id, "recent", Some(lines), ansi).await
    }

    /// Only what the pane shows right now. Unlike `recent`, this never makes
    /// herdr scroll a resting full-screen agent to collect its history.
    pub async fn read_visible(&self, pane_id: &str, ansi: bool) -> Result<PaneRead> {
        self.read_source(pane_id, "visible", None, ansi).await
    }

    async fn read_source(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
        ansi: bool,
    ) -> Result<PaneRead> {
        let mut params = if ansi {
            json!({ "pane_id": pane_id, "source": source, "format": "ansi", "strip_ansi": false })
        } else {
            json!({ "pane_id": pane_id, "source": source, "strip_ansi": true })
        };
        if let Some(lines) = lines {
            params["lines"] = json!(lines);
        }
        let mut result = self.request("pane.read", params).await?;
        let read = result
            .get_mut("read")
            .map(Value::take)
            .ok_or_else(|| Error::Protocol("pane.read: missing read".into()))?;
        Ok(serde_json::from_value(read)?)
    }
}

/// A live `events.subscribe` connection.
pub struct Subscription {
    stream: BufReader<BoxStream>,
    /// Line read so far; kept across calls so `next` is cancel-safe.
    buf: Vec<u8>,
    id: String,
}

impl Subscription {
    /// Next pushed event. `Ok(None)` means herdr closed the subscription.
    /// An `events_lost` error means the caller must resubscribe and resync.
    /// Cancel-safe (used in `select!`): a line cut off by a cancelled call is
    /// finished by the next one.
    pub async fn next(&mut self) -> Result<Option<Event>> {
        loop {
            let Some(line) = read_line(&mut self.stream, &mut self.buf).await? else {
                return Ok(None);
            };
            let value: Value = serde_json::from_str(&line)?;
            if value.get("error").is_some() {
                parse_response(&line, &self.id)?;
            }
            if value.get("event").is_some() {
                return Ok(Some(serde_json::from_value(value)?));
            }
            // Anything else (e.g. a stray result) carries no event; skip it.
        }
    }
}

async fn write_request(
    stream: &mut BufReader<BoxStream>,
    id: &str,
    method: &str,
    params: Value,
) -> Result<()> {
    let mut line = serde_json::to_vec(&json!({ "id": id, "method": method, "params": params }))?;
    line.push(b'\n');
    let inner = stream.get_mut();
    inner.write_all(&line).await?;
    inner.flush().await?;
    Ok(())
}

/// Read one line into `buf` and return it without the newline; at the end of
/// the stream the rest, or `None`. Cancel-safe: bytes read so far stay in
/// `buf` and the next call continues the line.
async fn read_line(stream: &mut BufReader<BoxStream>, buf: &mut Vec<u8>) -> Result<Option<String>> {
    let limit = (MAX_LINE_BYTES + 1).saturating_sub(buf.len()) as u64;
    (&mut *stream).take(limit).read_until(b'\n', buf).await?;
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_LINE_BYTES {
        buf.clear();
        return Err(Error::Protocol("response line too long".into()));
    } else if buf.is_empty() {
        return Ok(None);
    }
    into_string(std::mem::take(buf)).map(Some)
}

fn into_string(buf: Vec<u8>) -> Result<String> {
    String::from_utf8(buf).map_err(|_| Error::Protocol("response is not UTF-8".into()))
}

/// Parse a response line; returns `result` or a typed herdr error.
fn parse_response(line: &str, expected_id: &str) -> Result<Value> {
    let mut value: Value = serde_json::from_str(line)?;
    if let Some(error) = value.get("error") {
        return Err(Error::Herdr {
            code: error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    if id != expected_id {
        return Err(Error::Protocol(format!(
            "response id {id:?} does not match {expected_id:?}"
        )));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| Error::Protocol(format!("response without result: {line}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_keys_are_xterm_sequences() {
        assert_eq!(raw_key("home").as_deref(), Some("\x1b[H"));
        assert_eq!(raw_key("end").as_deref(), Some("\x1b[F"));
        assert_eq!(raw_key("pageup").as_deref(), Some("\x1b[5~"));
        assert_eq!(raw_key("pagedown").as_deref(), Some("\x1b[6~"));
        assert_eq!(raw_key("delete").as_deref(), Some("\x1b[3~"));
        assert_eq!(raw_key("ctrl+home").as_deref(), Some("\x1b[1;5H"));
        assert_eq!(raw_key("shift+end").as_deref(), Some("\x1b[1;2F"));
        assert_eq!(raw_key("ctrl+alt+delete").as_deref(), Some("\x1b[3;7~"));
        // herdr names these itself.
        assert_eq!(raw_key("enter"), None);
        assert_eq!(raw_key("ctrl+c"), None);
        assert_eq!(raw_key("plus"), None);
        assert_eq!(raw_key("super+home"), None);
    }

    /// Mixed keys keep their order: named runs via `send_keys`, the others via `send_text`.
    #[cfg(unix)]
    #[tokio::test]
    async fn send_keys_keeps_order_across_raw_keys() {
        use std::sync::Mutex;
        use tokio::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("mc-test-{}-keys", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let result = if request["method"] == "ping" {
                    json!({ "type": "pong", "version": "0.9.5", "protocol": 22 })
                } else {
                    log.lock()
                        .unwrap()
                        .push(json!([request["method"], request["params"]]));
                    json!({ "type": "ok" })
                };
                let reply = format!("{}\n", json!({ "id": request["id"], "result": result }));
                reader
                    .into_inner()
                    .write_all(reply.as_bytes())
                    .await
                    .unwrap();
            }
        });

        let transport = Arc::new(Transport::Local(crate::transport::LocalTransport));
        let (client, _) = HerdrClient::connect(transport, socket.to_str(), None)
            .await
            .unwrap();
        client
            .send_keys("w1:p1", &["up", "home", "x", "ctrl+c", "end"])
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let keys = |k: &[&str]| json!(["pane.send_keys", { "pane_id": "w1:p1", "keys": k }]);
        let text = |t: &str| json!(["pane.send_text", { "pane_id": "w1:p1", "text": t }]);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                keys(&["up"]),
                text("\x1b[H"),
                keys(&["x", "ctrl+c"]),
                text("\x1b[F")
            ]
        );
    }

    #[test]
    fn candidates_prefer_xdg() {
        let env = RemoteEnv {
            home: "/home/u".into(),
            xdg_config_home: Some("/cfg/".into()),
        };
        assert_eq!(
            socket_candidates(&env, None),
            vec!["/cfg/herdr/herdr.sock", "/home/u/.config/herdr/herdr.sock"]
        );
        let env = RemoteEnv {
            home: "/home/u".into(),
            xdg_config_home: None,
        };
        assert_eq!(
            socket_candidates(&env, Some("work")),
            vec!["/home/u/.config/herdr/sessions/work/herdr.sock"]
        );
    }

    #[test]
    fn parses_errors_and_results() {
        let err = parse_response(
            r#"{"id":"a","error":{"code":"not_found","message":"pane not found"}}"#,
            "a",
        )
        .unwrap_err();
        assert_eq!(err.herdr_code(), Some("not_found"));
        let ok = parse_response(
            r#"{"id":"a","result":{"type":"pong","version":"0.9.5","protocol":22}}"#,
            "a",
        )
        .unwrap();
        assert_eq!(ok["protocol"], 22);
        assert!(parse_response(r#"{"id":"b","result":{}}"#, "a").is_err());
    }
}

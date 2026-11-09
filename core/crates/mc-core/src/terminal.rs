//! Live terminal view of one pane (the "TUI" mode of the app).
//!
//! herdr offers a documented bridge for third-party clients:
//! `herdr terminal session control <target> --cols N --rows N` (herdr docs,
//! "persistence-remote" and "cli-reference", Apache-2.0). It prints one JSON
//! record per line on stdout:
//!
//! - `{"type":"terminal.frame","seq","encoding":"ansi","width","height","full","bytes"}`
//!   with base64 ANSI bytes rendered for the controller's size
//! - `{"type":"terminal.closed","reason"}` when herdr ends the stream
//!
//! and reads JSON commands on stdin: `terminal.input` (`text` or base64
//! `bytes`), `terminal.resize`, `terminal.scroll`, `terminal.mouse`,
//! `terminal.release`. A controller owns input and size of the pane: while it
//! is attached herdr resizes the pane to the controller's size, and gives the
//! size back to herdr's own clients when it releases (checked against herdr
//! 0.9.5 on 16.10.2026).
//!
//! The command runs on the herdr machine through [`Transport::exec_stream`],
//! so the herdr binary there matches the server; the private binary client
//! protocol never has to be spoken here. The frames feed a [`vt100`] parser,
//! and [`Screen`] is what the app draws.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, mpsc};

use crate::error::{Error, Result};
use crate::transport::{Transport, shell_quote};

/// Where herdr may live when the login's `PATH` does not include it.
const HERDR_CANDIDATES: &[&str] = &[
    "herdr",
    "\"$HOME/.local/bin/herdr\"",
    "\"$HOME/.cargo/bin/herdr\"",
    "/usr/local/bin/herdr",
    "/opt/homebrew/bin/herdr",
];

/// One cell colour as herdr's renderer sent it. The app maps `Default` and
/// `Indexed` through its terminal scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for Color {
    fn from(c: vt100::Color) -> Self {
        match c {
            vt100::Color::Default => Color::Default,
            vt100::Color::Idx(i) => Color::Indexed(i),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// Neighbouring cells of one row with the same style. A wide character
/// (two cells) always gets a run of its own, so the app can place every run
/// at `col × cell width` and never drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub col: u16,
    /// Cells covered (a wide character counts two).
    pub width: u16,
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    /// One entry per row; blank cells with the default style are left out.
    pub lines: Vec<Vec<Run>>,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    /// Counts processed frames; changes whenever the picture may have.
    pub seq: u64,
}

impl Screen {
    pub fn from_vt(screen: &vt100::Screen, seq: u64) -> Self {
        let (rows, cols) = screen.size();
        let lines = (0..rows).map(|row| row_runs(screen, row, cols)).collect();
        let (cursor_row, cursor_col) = screen.cursor_position();
        Screen {
            cols,
            rows,
            lines,
            cursor_row,
            cursor_col,
            cursor_visible: !screen.hide_cursor(),
            seq,
        }
    }

    /// Plain text of every row, trailing blanks trimmed (tests, logging).
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|runs| {
                let mut line = String::new();
                let mut at = 0u16;
                for run in runs {
                    while at < run.col {
                        line.push(' ');
                        at += 1;
                    }
                    line.push_str(&run.text);
                    at = run.col + run.width;
                }
                line.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn style_of(cell: &vt100::Cell) -> Style {
    Style {
        fg: cell.fgcolor().into(),
        bg: cell.bgcolor().into(),
        bold: cell.bold(),
        dim: cell.dim(),
        italic: cell.italic(),
        underline: cell.underline(),
        inverse: cell.inverse(),
    }
}

fn row_runs(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    while col < cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            col += 1;
            continue;
        }
        let style = style_of(cell);
        let text = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        let width = if cell.is_wide() { 2 } else { 1 };
        // Blank default cells carry nothing to draw.
        if text == " " && style == Style::default() {
            col += width;
            continue;
        }
        match runs.last_mut() {
            Some(last)
                if width == 1
                    && last.style == style
                    && last.col + last.width == col
                    && last.width == last.text.chars().count() as u16 =>
            {
                last.text.push_str(text);
                last.width += 1;
            }
            _ => runs.push(Run {
                col,
                width,
                text: text.to_owned(),
                style,
            }),
        }
        col += width;
    }
    runs
}

/// Mouse wheel / page scroll inside the attached pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug)]
enum Command {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Scroll {
        direction: ScrollDirection,
        lines: u16,
    },
    Release,
}

impl Command {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Command::Input(bytes) => json!({
                "type": "terminal.input",
                "bytes": base64::engine::general_purpose::STANDARD.encode(bytes),
            }),
            Command::Resize { cols, rows } => {
                json!({"type": "terminal.resize", "cols": cols, "rows": rows})
            }
            Command::Scroll { direction, lines } => json!({
                "type": "terminal.scroll",
                "direction": match direction {
                    ScrollDirection::Up => "up",
                    ScrollDirection::Down => "down",
                },
                "lines": lines,
            }),
            Command::Release => json!({"type": "terminal.release"}),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Record {
    #[serde(rename = "terminal.frame")]
    Frame {
        width: u16,
        height: u16,
        bytes: String,
    },
    #[serde(rename = "terminal.closed")]
    Closed {
        #[serde(default)]
        reason: Option<String>,
    },
}

/// Why the session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// We released it ([`TerminalSession::release`] or drop).
    Released,
    /// herdr closed it (`reason` as herdr says it, e.g. "terminal attach
    /// taken over"), or the command failed with this output.
    Closed(String),
}

/// What the session reports to its owner. Called from the core's runtime;
/// keep it short.
pub trait TerminalSink: Send + Sync + 'static {
    fn screen(&self, screen: Screen);
    fn ended(&self, why: Ended);
}

/// A writable controller of one pane. Dropping it releases the pane.
pub struct TerminalSession {
    commands: mpsc::UnboundedSender<Command>,
}

/// Longest line read from herdr (a frame is one base64 line); longer ends the session.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// Largest terminal accepted from herdr.
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 1000;

/// The next line without its newline, `None` at the end. A line longer than
/// `max` bytes is an error: the reader must not grow without bound.
async fn next_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then(|| String::from_utf8_lossy(&line).into_owned()));
        }
        let (part, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (&available[..i], i + 1),
            None => (available, available.len()),
        };
        if line.len() + part.len() > max {
            return Err(std::io::Error::other("line too long"));
        }
        line.extend_from_slice(part);
        let newline = done > part.len();
        reader.consume(done);
        if newline {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
        }
    }
}

/// Shortest gap between two screens sent to the sink (about 30 per second).
const FRAME_GAP: std::time::Duration = std::time::Duration::from_millis(33);

impl TerminalSession {
    /// Attach to `pane_id` with a viewport of `cols × rows`, taking over
    /// from another controller if there is one. `socket_path` is the herdr
    /// API socket the host connection uses; herdr derives its client socket
    /// from it.
    pub async fn open(
        transport: &Transport,
        socket_path: &str,
        pane_id: &str,
        cols: u16,
        rows: u16,
        sink: Arc<dyn TerminalSink>,
    ) -> Result<Self> {
        if cols == 0 || rows == 0 {
            return Err(Error::Protocol("terminal size must not be zero".into()));
        }
        let stream = transport
            .exec_stream(&control_script(socket_path, pane_id, cols, rows))
            .await?;
        let (reader, mut writer) = tokio::io::split(stream);
        let (commands, mut rx) = mpsc::unbounded_channel::<Command>();
        // Set once we give the pane back; an end of the stream without it is a lost connection.
        let releasing = Arc::new(AtomicBool::new(false));

        // Commands → stdin of `herdr terminal session control`.
        {
            let releasing = releasing.clone();
            tokio::spawn(async move {
                loop {
                    let Some(command) = rx.recv().await else {
                        // The session was dropped: that is a release too.
                        releasing.store(true, Ordering::SeqCst);
                        break;
                    };
                    let release = matches!(command, Command::Release);
                    if release {
                        releasing.store(true, Ordering::SeqCst);
                    }
                    let mut line = command.to_json().to_string();
                    line.push('\n');
                    if writer.write_all(line.as_bytes()).await.is_err() || release {
                        break;
                    }
                    let _ = writer.flush().await;
                }
                // Closing stdin makes herdr release the pane as well.
                let _ = writer.shutdown().await;
            });
        }

        let parser = Arc::new(Mutex::new((vt100::Parser::new(rows, cols, 0), 0u64)));
        let dirty = Arc::new(Notify::new());
        let done = Arc::new(Notify::new());

        // Coalesce frames into at most one screen per FRAME_GAP.
        {
            let (parser, dirty, done, sink) =
                (parser.clone(), dirty.clone(), done.clone(), sink.clone());
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = dirty.notified() => {}
                        _ = done.notified() => break,
                    }
                    let screen = {
                        let guard = parser.lock().expect("terminal parser lock");
                        Screen::from_vt(guard.0.screen(), guard.1)
                    };
                    sink.screen(screen);
                    tokio::time::sleep(FRAME_GAP).await;
                }
            });
        }

        // stdout of herdr → parser.
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut other = String::new();
            let ended = loop {
                match next_line(&mut reader, MAX_LINE).await {
                    Ok(Some(line)) => match serde_json::from_str::<Record>(&line) {
                        Ok(Record::Frame {
                            width,
                            height,
                            bytes,
                        }) => {
                            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(bytes)
                            else {
                                continue;
                            };
                            // A sane terminal is far smaller; a bigger one would take all memory.
                            if width > MAX_COLS || height > MAX_ROWS {
                                break Ended::Closed(format!(
                                    "terminal too large: {width}×{height}"
                                ));
                            }
                            let mut guard = parser.lock().expect("terminal parser lock");
                            let (vt, seq) = &mut *guard;
                            if vt.screen().size() != (height, width) && width > 0 && height > 0 {
                                vt.screen_mut().set_size(height, width);
                            }
                            vt.process(&bytes);
                            *seq += 1;
                            drop(guard);
                            dirty.notify_one();
                        }
                        Ok(Record::Closed { reason }) => {
                            let reason = reason.unwrap_or_default();
                            break if reason == "detached" {
                                Ended::Released
                            } else {
                                Ended::Closed(reason)
                            };
                        }
                        // Error text from the script or herdr (stderr is
                        // redirected to stdout).
                        Err(_) => {
                            if other.len() < 2000 {
                                other.extend(line.trim().chars().take(2000 - other.len()));
                                other.push('\n');
                            }
                        }
                    },
                    Ok(None) | Err(_) => {
                        let text = other.trim();
                        break if !text.is_empty() {
                            Ended::Closed(text.to_owned())
                        } else if releasing.load(Ordering::SeqCst) {
                            Ended::Released
                        } else {
                            // Stream gone without a word (connection dropped): the owner
                            // should open a new session, not stop.
                            Ended::Closed("connection to the terminal lost".into())
                        };
                    }
                }
            };
            tracing::debug!(?ended, "terminal session ended");
            done.notify_one();
            sink.ended(ended);
        });

        Ok(Self { commands })
    }

    /// Raw bytes for the pane, as a terminal would send them (text, `\r`,
    /// escape sequences).
    pub fn input(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.commands.send(Command::Input(bytes));
        }
    }

    /// New viewport size; herdr resizes the pane and sends a full frame.
    pub fn resize(&self, cols: u16, rows: u16) {
        if cols > 0 && rows > 0 {
            let _ = self.commands.send(Command::Resize { cols, rows });
        }
    }

    /// Scroll like a mouse wheel: herdr forwards it to an application that
    /// asked for the mouse and scrolls its own history otherwise.
    pub fn scroll(&self, direction: ScrollDirection, lines: u16) {
        if lines > 0 {
            let _ = self.commands.send(Command::Scroll { direction, lines });
        }
    }

    /// Give the pane back; herdr restores its size for the other clients.
    pub fn release(&self) {
        let _ = self.commands.send(Command::Release);
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.release();
    }
}

/// `sh` script that finds herdr and runs the controller against the same
/// socket the API connection uses. No backslashes (see `exec_stream`).
fn control_script(socket_path: &str, pane_id: &str, cols: u16, rows: u16) -> String {
    format!(
        concat!(
            "h=; for c in {candidates}; do ",
            "if command -v \"$c\" >/dev/null 2>&1; then h=$c; break; fi; done; ",
            "if [ -z \"$h\" ]; then echo 'herdr not found on the server'; exit 127; fi; ",
            "HERDR_SOCKET_PATH={socket} exec \"$h\" terminal session control {pane} ",
            "--takeover --cols {cols} --rows {rows} 2>&1"
        ),
        candidates = HERDR_CANDIDATES.join(" "),
        socket = shell_quote(socket_path),
        pane = shell_quote(pane_id),
        cols = cols,
        rows = rows,
    )
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn lines_are_capped() {
        let mut r = BufReader::new(&b"eins\nzwei\r\ndrei"[..]);
        assert_eq!(
            next_line(&mut r, 10).await.unwrap().as_deref(),
            Some("eins")
        );
        assert_eq!(
            next_line(&mut r, 10).await.unwrap().as_deref(),
            Some("zwei")
        );
        assert_eq!(
            next_line(&mut r, 10).await.unwrap().as_deref(),
            Some("drei")
        );
        assert_eq!(next_line(&mut r, 10).await.unwrap(), None);
        let mut long = BufReader::new(&b"0123456789abc\n"[..]);
        assert!(next_line(&mut long, 10).await.is_err());
    }

    use super::*;

    fn screen(bytes: &[u8], rows: u16, cols: u16) -> Screen {
        let mut vt = vt100::Parser::new(rows, cols, 0);
        vt.process(bytes);
        Screen::from_vt(vt.screen(), 1)
    }

    #[test]
    fn groups_cells_into_runs() {
        let s = screen(b"\x1b[1;1Hab\x1b[31mcd\x1b[0m  e", 2, 10);
        let runs = &s.lines[0];
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!((runs[0].col, runs[0].text.as_str()), (0, "ab"));
        assert_eq!(runs[1].text, "cd");
        assert_eq!(runs[1].style.fg, Color::Indexed(1));
        // Blank default cells between runs are skipped.
        assert_eq!((runs[2].col, runs[2].text.as_str()), (6, "e"));
        assert_eq!(s.text(), "abcd  e\n");
        assert!(s.lines[1].is_empty());
    }

    #[test]
    fn wide_characters_get_their_own_run() {
        let s = screen("a日b".as_bytes(), 1, 10);
        let runs = &s.lines[0];
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!((runs[1].col, runs[1].width), (1, 2));
        assert_eq!(runs[2].col, 3);
        assert_eq!(s.text(), "a日b");
    }

    #[test]
    fn keeps_cursor_and_colours() {
        let s = screen(b"\x1b[48;2;1;2;3m \x1b[0m\x1b[?25l\x1b[2;4H", 3, 10);
        assert_eq!(s.lines[0][0].style.bg, Color::Rgb(1, 2, 3));
        assert_eq!((s.cursor_row, s.cursor_col), (1, 3));
        assert!(!s.cursor_visible);
    }

    #[test]
    fn script_quotes_its_arguments() {
        let script = control_script("/tmp/a b/herdr.sock", "w1:p1", 60, 20);
        assert!(script.contains("HERDR_SOCKET_PATH='/tmp/a b/herdr.sock' exec"));
        assert!(script.contains("control 'w1:p1' --takeover --cols 60 --rows 20 2>&1"));
        assert!(!script.contains('\\'));
    }

    #[test]
    fn commands_match_herdrs_json() {
        assert_eq!(
            Command::Input(b"ls\r".to_vec()).to_json(),
            json!({"type": "terminal.input", "bytes": "bHMN"})
        );
        assert_eq!(
            Command::Scroll {
                direction: ScrollDirection::Up,
                lines: 3
            }
            .to_json()["direction"],
            "up"
        );
    }
}

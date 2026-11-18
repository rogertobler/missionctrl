//! `terminal.attach` on herdr-core: the live terminal of the app, typing text
//! without Enter, and reading a coloured history.
//!
//! An attach answers with a channel (framed mode, see [`super::framed`]): the
//! core sends a first paint (screen, optional history) and then the program's
//! raw output; the viewer sends keys as DATA and controls the stream with
//! RESIZE, CREDIT and CLOSE (herdr-core `schema/README.md`, "Attach streams").
//!
//! Size ("watch, take over when typing"): the core gives the terminal the
//! size of its last *active* viewer (one that attached interactively, typed,
//! resized or gained focus). The live session shows a watch-only attach
//! (`observe: true`, `resize_notifications: true`) the whole time: it never
//! types or resizes, and the core sends it a RESIZE and a fresh paint for every
//! change of the grid, so the parser always has the terminal's real size.
//!
//! The first input opens a second, interactive attach at the app's size on the
//! same connection; attaching makes it the active viewer, so it takes the size
//! by itself. It carries the input while the app is the active viewer. The
//! app's own size changes (keyboard, rotation) go to the core only together
//! with the next input: the core gives no signal when another viewer of the
//! same size becomes active, so a resize on its own could take the terminal
//! back from the desktop without anyone typing. When the watched RESIZE shows
//! a size the app did not set, another viewer (the desktop) took over: the
//! interactive attach is closed and the session only watches, until the next
//! input attaches again. Sizes the app sent are remembered in order until the
//! core echoes them, so a late echo of an earlier size is not mistaken for
//! another viewer. The core coalesces notifications it has not sent yet (only
//! the final grid of quick changes may be announced), so the bookkeeping errs
//! on one side: a size it set is expected for at most a few seconds, and a
//! return to the announced grid expects nothing. A size of its own taken for
//! another viewer's costs one more attach at the next key; the other way
//! round would leave the app typing at the desktop's size.
//!
//! The mouse wheel while only watching goes through a short attach at the
//! terminal's own size that is closed right after. The protocol has no input
//! that leaves the active viewer alone (DATA is human input; AUTOMATIC_INPUT
//! is reserved for the terminal's own replies), so the wheel attach is the
//! active viewer for a moment: at the grid the core announced last, so the
//! size stays as it is. When the grid changes while the attach is on its way,
//! it is given back unused and asked anew at the new grid. Notches that come
//! meanwhile are added to it (up to [`WHEEL_MAX`]), so a long gesture on a
//! slow link scrolls as far as it should.
//!
//! A core that cannot send these notifications (older ones, or a relay path
//! that drops them) gets one interactive attach at the app's size instead,
//! like the herdr TUI: the app then always sets the size.
//!
//! Named keys (`terminal.keys`, encoded by the core for the program's
//! keyboard mode) go on channel 0 and do not make a viewer active, so each
//! goes after a RESIZE of the interactive attach to the app's size, even an
//! unchanged one and also without notifications: the desktop may have taken
//! the size without its notification having arrived. Such a notification
//! arriving while the keys wait does not end typing; the size is taken back
//! once more instead (only once per keys, so someone typing at the desktop
//! is not fought over). The core holds such keys until people have stopped
//! typing for a moment and cannot be asked to drop them, so input typed after
//! them waits for their answer (see [`KEYS_DEADLINE`]). Letting go waits
//! (bounded) for them and for input still on its way to an interactive
//! attach, so nothing typed before leaving is lost. The session ends at the
//! latest [`LEAVE_HARD`] after its owner let go, so the owner can wait for
//! that end before it opens the next session of the same terminal. The core
//! does not cancel `terminal.keys` when the connection closes: keys still
//! unanswered then may be typed later, after input of the next session.
//!
//! Input typed but not yet out (in the command channel, or queued behind the
//! interactive attach or keys) is bounded by an [`InputBudget`]: past it,
//! input is refused and the screens count it.
//!
//! MissionCtrl never sends FOCUS, FACTS or AUTOMATIC_INPUT: attaching,
//! typing and resizing already make a viewer active, FOCUS would also reach
//! programs that track focus, and the parser answers no terminal queries.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use super::client::Endpoint;
use super::framed::{CLOSE, DATA, Frame, Framed, FramedWriter, RESIZE, read_frame, write_frame};
use super::model::AttachResult;
use crate::error::{Error, Result};
use crate::terminal::{
    Command, Ended, FRAME_GAP, InputBudget, MAX_COLS, MAX_ROWS, Screen, ScrollDirection,
    TerminalSession, TerminalSink, ansi_contents, key_bytes,
};
use crate::transport::Transport;

/// Lines of output the live view keeps for scrolling back by hand.
const SCROLLBACK: usize = 5000;
/// History the live view asks for when it opens, so scrolling back works
/// from the start (main screen only).
const OPEN_HISTORY: usize = 1000;
/// Unreported consumed bytes before a CREDIT frame. The core starts every
/// stream with 256 KiB of credit, so reporting in steps far below that never
/// stalls it.
const CREDIT_STEP: usize = 64 * 1024;
/// Longest wait to open a framed connection and get an attach answered.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest wait for one frame to be written: a peer that stops reading must
/// not hold the session (and its connection) forever.
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// Most sizes remembered while waiting for their echoes.
const SENT_MAX: usize = 16;
/// Longest wait for the echo of a size the app set (see the module notes).
const ECHO_WAIT: Duration = Duration::from_secs(3);
/// Longest wait for the core's CLOSE after the session let go.
const RELEASE_WAIT: Duration = Duration::from_secs(2);
/// Longest wait for a `terminal.keys` answer. The core holds named keys
/// until people have stopped typing for a moment (about 3 s, also on the
/// desktop) and cannot be asked to drop them, so input typed after keys
/// waits for their answer rather than overtake them; without one in this
/// time the session ends ("did not answer") instead of sending it.
#[cfg(not(test))]
const KEYS_DEADLINE: Duration = Duration::from_secs(60);
#[cfg(test)]
const KEYS_DEADLINE: Duration = Duration::from_millis(1500);
/// Letting go with keys still unanswered: how long to wait for an answer (and
/// to send what was typed after it) before closing anyway. Each answer starts
/// the wait anew, up to [`LEAVE_MAX`] in all.
#[cfg(not(test))]
const KEYS_LEAVE_WAIT: Duration = Duration::from_secs(6);
#[cfg(test)]
const KEYS_LEAVE_WAIT: Duration = Duration::from_millis(1000);
/// Longest wait, from letting go, for keys and the input after them.
#[cfg(not(test))]
const LEAVE_MAX: Duration = Duration::from_secs(12);
#[cfg(test)]
const LEAVE_MAX: Duration = Duration::from_millis(2500);
/// A session ends at the latest this long after its owner let go, even in
/// the middle of a write: the owner opens the next session only after this
/// one ended, so nothing typed there can overtake what this one still sends.
#[cfg(not(test))]
pub const LEAVE_HARD: Duration = Duration::from_secs(15);
#[cfg(test)]
pub const LEAVE_HARD: Duration = Duration::from_millis(3500);
/// Longest time for a whole coloured read (attach, paint, detach) over a slow
/// link.
const PAINT_DEADLINE: Duration = Duration::from_secs(10);
/// The core ends every first paint with the program's win32-input mode,
/// right after the screen (herdr-core 0.1.20 `viewers.rs`, `paint_state`):
/// the marker that the screen is complete. Only the final byte differs
/// (`h`/`l`). This is the position in 0.1.20's code, not a documented
/// boundary (`schema/README.md` lists the mode among the first paint's
/// parts); a core without it is noticed by the client
/// ([`super::CoreClient::read_painted`]) and read without colours.
const PAINT_END: &[u8] = b"\x1b[?9001";
/// The error of a paint that came but never ended within [`PAINT_DEADLINE`]
/// (see [`is_paint_without_end`]); a link that brought nothing at all is
/// [`Error::Timeout`] instead.
const NO_PAINT_END: &str = "paint without its end marker";

/// Whether `e` is a paint that came but never reached its end marker: the
/// sign of a core that paints without it (or a link that stalled mid-paint).
pub fn is_paint_without_end(e: &Error) -> bool {
    matches!(e, Error::Protocol(m) if m == NO_PAINT_END)
}
/// Longest time for typing text without Enter (attach, input, detach).
const TYPE_DEADLINE: Duration = Duration::from_secs(15);
/// Most wheel events collected while a wheel attach is on its way.
const WHEEL_MAX: usize = 64;

const LOST: &str = "connection to the terminal lost";

fn sane_size(cols: u16, rows: u16) -> Result<()> {
    if cols == 0 || rows == 0 {
        return Err(Error::Protocol("terminal size must not be zero".into()));
    }
    if cols > MAX_COLS || rows > MAX_ROWS {
        return Err(Error::Protocol(format!(
            "terminal too large: {cols}×{rows}"
        )));
    }
    Ok(())
}

/// Open a framed connection and attach, within [`ATTACH_TIMEOUT`].
async fn attach(
    transport: &Transport,
    endpoint: &Endpoint,
    params: serde_json::Value,
) -> Result<(Framed, AttachResult)> {
    tokio::time::timeout(ATTACH_TIMEOUT, async {
        let mut framed = Framed::open(transport, endpoint).await?;
        let result: AttachResult =
            serde_json::from_value(framed.call("terminal.attach", params).await?)?;
        Ok((framed, result))
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// Mouse wheel events for `lines`, in the encoding the program asked for,
/// at the middle of a `cols × rows` grid. About three lines per notch, like
/// most terminals.
fn wheel_bytes(
    screen: &vt100::Screen,
    (cols, rows): (u16, u16),
    direction: ScrollDirection,
    lines: u16,
) -> Vec<u8> {
    let button: u8 = match direction {
        ScrollDirection::Up => 64,
        ScrollDirection::Down => 65,
    };
    let (x, y) = (cols / 2 + 1, rows / 2 + 1);
    let one: Vec<u8> = match screen.mouse_protocol_encoding() {
        vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}M").into_bytes(),
        _ => {
            let byte = |v: u16| (32 + v.min(223)) as u8;
            vec![0x1b, b'[', b'M', 32 + button, byte(x), byte(y)]
        }
    };
    let notches = lines.div_ceil(3).clamp(1, 20);
    one.repeat(notches as usize)
}

/// Named keys as the screens report them ([`Screen::keys_pending`]).
#[derive(Default)]
struct KeyStatus {
    pending: bool,
    errors: u64,
    error: Option<String>,
}

/// Control terminal `terminal` live (see the module notes on size). `cols ×
/// rows` is the app's view; the session watches at the terminal's own size
/// until the first input.
pub async fn open_session(
    transport: &Transport,
    endpoint: &Endpoint,
    terminal: &str,
    cols: u16,
    rows: u16,
    sink: Arc<dyn TerminalSink>,
) -> Result<TerminalSession> {
    sane_size(cols, rows)?;
    let watch = json!({
        "terminal": terminal, "cols": cols, "rows": rows,
        "observe": true, "resize_notifications": true, "history": OPEN_HISTORY,
    });
    let interactive = json!({ "terminal": terminal, "cols": cols, "rows": rows });
    let (mut framed, mut shown) = attach(transport, endpoint, watch).await?;
    let watching = shown.resize_notifications;
    if !watching {
        // Without RESIZE frames the watched picture would garble whenever the
        // desktop resizes: show an interactive attach at the app's size.
        tracing::debug!("core did not confirm resize notifications; the app sets the size");
        let result = tokio::time::timeout(ATTACH_TIMEOUT, async {
            write_frame(&mut framed.writer, &Frame::close(shown.channel)).await?;
            framed.call("terminal.attach", interactive).await
        })
        .await
        .map_err(|_| Error::Timeout)??;
        shown = serde_json::from_value(result)?;
    }
    let grid = if watching {
        match (shown.terminal.cols, shown.terminal.rows) {
            (c, r) if sane_size(c, r).is_ok() => (c, r),
            _ => (cols, rows),
        }
    } else {
        (cols, rows)
    };
    let Framed {
        reader: mut frames_in,
        writer,
        ..
    } = framed;

    let parser = Arc::new(Mutex::new((
        vt100::Parser::new(grid.1, grid.0, SCROLLBACK),
        0u64,
    )));
    let dirty = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());
    let typing = Arc::new(AtomicBool::new(!watching));
    let keys = Arc::new(Mutex::new(KeyStatus::default()));
    let budget = Arc::new(InputBudget::new(dirty.clone()));
    let (commands, commands_rx) = mpsc::unbounded_channel::<Command>();
    let (let_go, let_go_rx) = tokio::sync::watch::channel(None);
    // Frames are read in a task of their own: reading one is not cancel-safe.
    // The task is aborted when the session ends, which drops the connection.
    let (frames_tx, frames_rx) = mpsc::unbounded_channel::<Option<Frame>>();
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut frames_in).await.ok().flatten();
            let end = frame.is_none();
            if frames_tx.send(frame).is_err() || end {
                break;
            }
        }
    });

    // At most one screen per FRAME_GAP to the sink.
    {
        let (parser, dirty, done, sink, typing, keys, budget) = (
            parser.clone(),
            dirty.clone(),
            done.clone(),
            sink.clone(),
            typing.clone(),
            keys.clone(),
            budget.clone(),
        );
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = dirty.notified() => {}
                    _ = done.notified() => break,
                }
                let screen = {
                    let guard = parser.lock().expect("terminal parser lock");
                    let mut screen = Screen::from_vt(guard.0.screen(), guard.1);
                    // Scrolled back: the cursor is somewhere below.
                    if guard.0.screen().scrollback() > 0 {
                        screen.cursor_visible = false;
                    }
                    screen.watching = !typing.load(Ordering::Relaxed);
                    let keys = keys.lock().expect("key status lock");
                    screen.keys_pending = keys.pending;
                    screen.key_errors = keys.errors;
                    screen.key_error = keys.error.clone();
                    screen.input_refused = budget.refused();
                    screen
                };
                sink.screen(screen);
                tokio::time::sleep(FRAME_GAP).await;
            }
        });
    }

    let controller = Controller {
        typing,
        keys,
        budget: budget.clone(),
        terminal: terminal.to_owned(),
        writer,
        parser,
        dirty,
        shown: shown.channel,
        watching,
        input: (!watching).then_some(shown.channel),
        input_size: (cols, rows),
        input_unreported: 0,
        pending_attach: None,
        pending_wheel: None,
        pending_keys: None,
        keys_reclaimed: false,
        echo_wait: ECHO_WAIT,
        abandoned: Vec::new(),
        queued: Vec::new(),
        want: (cols, rows),
        grid,
        sent: VecDeque::new(),
        unreported: 0,
        next_id: 1000,
        leaving: None,
        leave_wait: None,
        releasing: None,
    };
    tokio::spawn(async move {
        // Also when the task is torn down or the controller panics: the
        // owner may be waiting for this end before it opens the next session.
        let mut end = EndOnDrop {
            sink,
            done,
            reader,
            ended: Some(Ended::Closed(LOST.into())),
        };
        end.ended = Some(controller.run(commands_rx, frames_rx, let_go_rx).await);
    });
    Ok(TerminalSession::from_commands_with_let_go(
        commands, let_go, budget,
    ))
}

/// Reports the session's end to its owner when dropped, however the
/// controller's task ends.
struct EndOnDrop {
    sink: Arc<dyn TerminalSink>,
    done: Arc<Notify>,
    reader: tokio::task::JoinHandle<()>,
    ended: Option<Ended>,
}

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        // Ends the connection: the reader holds its other half.
        self.reader.abort();
        let ended = self.ended.take().unwrap_or(Ended::Released);
        tracing::debug!(?ended, "core terminal session ended");
        self.done.notify_one();
        self.sink.ended(ended);
    }
}

/// Owns the connection's writing side and decides which channel takes input.
struct Controller {
    /// What the screens report: the app types (interactive attach open or on
    /// its way) rather than only watches.
    typing: Arc<AtomicBool>,
    /// What the screens report about named keys.
    keys: Arc<Mutex<KeyStatus>>,
    /// Bounds input not yet out; given back as it goes out or is dropped.
    budget: Arc<InputBudget>,
    terminal: String,
    writer: FramedWriter,
    parser: Arc<Mutex<(vt100::Parser, u64)>>,
    dirty: Arc<Notify>,
    /// The attach whose bytes the app shows: watch-only (`watching`), else the
    /// one interactive attach.
    shown: u32,
    /// `shown` watches and gets the core's RESIZE notifications.
    watching: bool,
    /// The attach that carries input: the interactive one while the app is the
    /// active viewer. Equal to `shown` without notifications.
    input: Option<u32>,
    /// The size `input` last set (by attaching or a RESIZE).
    input_size: (u16, u16),
    /// Bytes of an interactive `input` consumed but not yet reported as credit.
    input_unreported: usize,
    /// Id, send time and size of the interactive `terminal.attach`, until
    /// answered.
    pending_attach: Option<(u64, Instant, (u16, u16))>,
    /// A wheel attach while only watching, until answered.
    pending_wheel: Option<Wheel>,
    /// A `terminal.keys` request, until answered: input typed after it waits
    /// (the core may hold the keys for a quiet moment first).
    pending_keys: Option<(u64, Instant)>,
    /// The size was taken back once already for the pending keys, after a
    /// notification of another size ([`Self::shown_frame`]).
    keys_reclaimed: bool,
    /// Attaches given up on (a wheel that took too long): closed when their
    /// answer still comes.
    abandoned: Vec<u64>,
    /// Input typed while the interactive attach is on its way.
    queued: Vec<Typed>,
    /// The app's view size.
    want: (u16, u16),
    /// The terminal's grid as the core last announced it.
    grid: (u16, u16),
    /// Sizes the app set that the core has not echoed yet, oldest first.
    sent: VecDeque<((u16, u16), Instant)>,
    /// How long an echo may take: [`ECHO_WAIT`], longer on a slow link
    /// (twice the last interactive attach's round trip).
    echo_wait: Duration,
    unreported: usize,
    next_id: u64,
    /// Since when the session waits for unanswered keys before letting go
    /// (commands no longer count).
    leaving: Option<Instant>,
    /// Since the last answer while leaving (or since leaving): the wait for
    /// the next starts there.
    leave_wait: Option<Instant>,
    /// Since when the session lets go (CLOSE sent, waiting for the core's).
    releasing: Option<Instant>,
}

/// A wheel attach on its way ([`Controller::wheel`]).
struct Wheel {
    id: u64,
    at: Instant,
    /// Direction and lines of each wheel event, encoded when sent.
    events: Vec<(ScrollDirection, u16)>,
    /// The grid it attaches at.
    grid: (u16, u16),
    /// Already asked anew once after the grid changed.
    retried: bool,
}

/// What the run loop does next.
enum Step {
    Go,
    End(Ended),
}

impl Controller {
    /// Write a frame within [`WRITE_TIMEOUT`]; a failed or stuck write means
    /// the connection is gone.
    async fn send(&mut self, frame: Frame) -> Step {
        match self.write(&frame).await {
            Ok(()) => Step::Go,
            Err(_) => Step::End(Ended::Closed(LOST.into())),
        }
    }

    async fn write(&mut self, frame: &Frame) -> Result<()> {
        tokio::time::timeout(WRITE_TIMEOUT, write_frame(&mut self.writer, frame))
            .await
            .map_err(|_| Error::Timeout)?
    }

    fn with_screen(&self, f: impl FnOnce(&mut vt100::Screen)) {
        let mut guard = self.parser.lock().expect("terminal parser lock");
        f(guard.0.screen_mut());
    }

    async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut frames: mpsc::UnboundedReceiver<Option<Frame>>,
        mut let_go: tokio::sync::watch::Receiver<Option<Instant>>,
    ) -> Ended {
        loop {
            // [`LEAVE_HARD`] after the owner let go, whatever this step is
            // doing (a write may wait up to [`WRITE_TIMEOUT`]).
            let step = tokio::select! {
                biased;
                _ = hard_stop(&mut let_go) => Step::End(Ended::Released),
                step = self.step(&mut commands, &mut frames) => step,
            };
            if let Step::End(ended) = step {
                return ended;
            }
            self.report_keys();
            let typing = self.input.is_some() || self.pending_attach.is_some();
            if self.typing.swap(typing, Ordering::Relaxed) != typing {
                self.dirty.notify_one();
            }
        }
    }

    /// Wait for the next command, frame or timer and handle it.
    async fn step(
        &mut self,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        frames: &mut mpsc::UnboundedReceiver<Option<Frame>>,
    ) -> Step {
        let releasing = self.releasing;
        let pending = self.pending_attach.map(|(_, at, _)| at);
        let wheel = self.pending_wheel.as_ref().map(|w| w.at);
        let keys = self.pending_keys.map(|(_, at)| at);
        let leaving = self.leaving;
        let leave_until = self.leaving.map(|at| {
            let wait = self.leave_wait.unwrap_or(at) + KEYS_LEAVE_WAIT;
            wait.min(at + LEAVE_MAX)
        });
        let step = tokio::select! {
            // Once letting go, commands no longer matter (and a closed
            // command channel would answer at once, forever).
            command = commands.recv(), if releasing.is_none() && leaving.is_none() => match command {
                None | Some(Command::Release) => self.let_go().await,
                Some(command) => self.command(command).await,
            },
            frame = frames.recv() => match frame.flatten() {
                Some(frame) => self.frame(frame).await,
                None => Step::End(if releasing.is_some() {
                    Ended::Released
                } else {
                    // Gone without a word (connection dropped): the owner
                    // should open a new session, not stop.
                    Ended::Closed(LOST.into())
                }),
            },
            _ = tokio::time::sleep_until(releasing.unwrap_or_else(Instant::now) + RELEASE_WAIT),
                if releasing.is_some() => Step::End(Ended::Released),
            // Unanswered while leaving: the user left anyway, so the
            // session lets go as asked.
            _ = tokio::time::sleep_until(pending.unwrap_or_else(Instant::now) + ATTACH_TIMEOUT),
                if pending.is_some() && releasing.is_none() => self.unanswered(leaving).await,
            // Keys never answered: input after them must not overtake
            // them (the core may still type them), so it is not sent.
            _ = tokio::time::sleep_until(keys.unwrap_or_else(Instant::now) + KEYS_DEADLINE),
                if keys.is_some() && releasing.is_none() => self.unanswered(leaving).await,
            _ = tokio::time::sleep_until(leave_until.unwrap_or_else(Instant::now)),
                if leaving.is_some() && releasing.is_none() => self.release().await,
            // A wheel that never got its attach: drop it, close it if
            // the answer still comes.
            _ = tokio::time::sleep_until(wheel.unwrap_or_else(Instant::now) + ATTACH_TIMEOUT),
                if wheel.is_some() && releasing.is_none() => {
                if let Some(w) = self.pending_wheel.take() {
                    self.abandoned.push(w.id);
                }
                Step::Go
            }
        };
        match step {
            // Leaving, and everything typed is out: let go now.
            Step::Go
                if self.leaving.is_some()
                    && self.releasing.is_none()
                    && self.pending_keys.is_none()
                    && self.pending_attach.is_none()
                    && self.queued.is_empty() =>
            {
                self.release().await
            }
            step => step,
        }
    }

    /// The app lets go. With keys still unanswered, or input still waiting
    /// for the interactive attach, wait ([`KEYS_LEAVE_WAIT`] per answer, at
    /// most [`LEAVE_MAX`]) until it is out, so nothing typed is lost; then release. Input queued behind
    /// keys after the desktop took over attaches anew for it.
    async fn let_go(&mut self) -> Step {
        let waiting = self.pending_keys.is_some()
            || (self.pending_attach.is_some() && !self.queued.is_empty());
        if waiting {
            let now = Instant::now();
            self.leaving = Some(now);
            self.leave_wait = Some(now);
            return Step::Go;
        }
        self.release().await
    }

    /// A request went unanswered: an error while the user is here, letting
    /// go when they already left.
    async fn unanswered(&mut self, leaving: Option<Instant>) -> Step {
        if leaving.is_some() {
            return self.release().await;
        }
        Step::End(Ended::Closed("the terminal did not answer".into()))
    }

    /// Bring the screens' key status up to date: keys are pending from the
    /// moment they are typed until herdr answers.
    fn report_keys(&self) {
        let pending = self.releasing.is_none()
            && (self.pending_keys.is_some()
                || self.queued.iter().any(|t| matches!(t, Typed::Keys(_))));
        let mut keys = self.keys.lock().expect("key status lock");
        if keys.pending != pending {
            keys.pending = pending;
            self.dirty.notify_one();
        }
    }

    /// herdr refused named keys: the screens report it once.
    fn key_failed(&self, why: String) {
        let mut keys = self.keys.lock().expect("key status lock");
        keys.errors += 1;
        keys.error = Some(why);
        self.dirty.notify_one();
    }

    /// Let go: close the attaches and wait (bounded) for the core's CLOSE.
    /// Both CLOSE frames together get one [`WRITE_TIMEOUT`], so a stuck
    /// connection holds letting go at most that plus [`RELEASE_WAIT`].
    async fn release(&mut self) -> Step {
        self.releasing = Some(Instant::now());
        self.drop_queued();
        self.pending_wheel = None;
        self.pending_keys = None;
        let mut closes = Vec::new();
        if let Some(input) = self.input.take().filter(|i| *i != self.shown) {
            closes.push(Frame::close(input));
        }
        closes.push(Frame::close(self.shown));
        let writer = &mut self.writer;
        let written = tokio::time::timeout(WRITE_TIMEOUT, async {
            for frame in &closes {
                write_frame(writer, frame).await?;
            }
            Ok::<_, Error>(())
        })
        .await;
        match written {
            Ok(Ok(())) => Step::Go,
            // A broken or stuck connection ends the session as released all
            // the same.
            _ => Step::End(Ended::Released),
        }
    }

    /// Handle one app command.
    async fn command(&mut self, command: Command) -> Step {
        match command {
            Command::Input(bytes) => {
                self.with_screen(|s| s.set_scrollback(0));
                self.dirty.notify_one();
                self.type_bytes(bytes).await
            }
            Command::Resize { cols, rows } => {
                self.want = (cols, rows);
                match self.input {
                    // No notifications: the app always sets the size, like
                    // the herdr TUI, and its size is the grid.
                    Some(input) if !self.watching => {
                        self.input_size = (cols, rows);
                        let step = self.send(Frame::resize(input, cols, rows)).await;
                        self.with_screen(|s| s.set_size(rows, cols));
                        self.grid = (cols, rows);
                        self.dirty.notify_one();
                        step
                    }
                    // Watching (or typing): the new size goes with the next
                    // input (see the module notes).
                    _ => Step::Go,
                }
            }
            Command::Scroll { direction, lines } => {
                // Typing: the next input sets the app's size first. Only
                // watching: the wheel goes in at the terminal's own size.
                let size = if self.input.is_some() {
                    self.want
                } else {
                    self.grid
                };
                let wheel = {
                    let mut guard = self.parser.lock().expect("terminal parser lock");
                    let screen = guard.0.screen_mut();
                    if screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None {
                        Some(wheel_bytes(screen, size, direction, lines))
                    } else {
                        // No mouse: scroll the view's own history.
                        let at = screen.scrollback();
                        screen.set_scrollback(match direction {
                            ScrollDirection::Up => at.saturating_add(lines as usize),
                            ScrollDirection::Down => at.saturating_sub(lines as usize),
                        });
                        None
                    }
                };
                match wheel {
                    Some(bytes) if self.input.is_some() || self.pending_attach.is_some() => {
                        // Counted like typed input, as it goes out the same way.
                        self.budget.charge(bytes.len());
                        self.type_bytes(bytes).await
                    }
                    Some(_) => self.wheel((direction, lines)).await,
                    None => {
                        self.dirty.notify_one();
                        Step::Go
                    }
                }
            }
            Command::Keys(keys) => {
                self.with_screen(|s| s.set_scrollback(0));
                self.dirty.notify_one();
                self.type_input(Typed::Keys(keys)).await
            }
            Command::Release => self.release().await,
        }
    }

    /// The core will announce `size` to the watching attach, unless it is
    /// the size the terminal will have anyway. Back at the announced grid
    /// (A, B, A), the core may announce the steps or coalesce them into no
    /// change at all: nothing is expected then, so a later B from the
    /// desktop is never taken for an echo (an echo that does come counts as
    /// another viewer: one more attach at the next key).
    fn expect_echo(&mut self, size: (u16, u16)) {
        if !self.watching {
            return;
        }
        if size == self.grid {
            self.sent.clear();
            return;
        }
        if self.sent.back().map(|(s, _)| *s) != Some(size) {
            self.sent.push_back((size, Instant::now()));
            if self.sent.len() > SENT_MAX {
                self.sent.pop_front();
            }
        }
    }

    /// Make `input` the active viewer at the app's size with a RESIZE, even
    /// an unchanged one. The core may be at another viewer's size already
    /// (its notification still on the way), so the app's size is expected
    /// back even when it equals the announced grid.
    async fn reclaim(&mut self, input: u32) -> Step {
        let (cols, rows) = self.want;
        self.expect_echo((cols, rows));
        if self.watching && self.sent.back().map(|(s, _)| *s) != Some((cols, rows)) {
            self.sent.push_back(((cols, rows), Instant::now()));
            if self.sent.len() > SENT_MAX {
                self.sent.pop_front();
            }
        }
        self.input_size = (cols, rows);
        self.send(Frame::resize(input, cols, rows)).await
    }

    /// Before input: bring the app's current size to the terminal if it
    /// changed since `input` set it.
    async fn apply_want(&mut self, input: u32) -> Step {
        if !self.watching || self.want == self.input_size {
            return Step::Go;
        }
        let (cols, rows) = self.want;
        self.expect_echo((cols, rows));
        self.input_size = (cols, rows);
        self.send(Frame::resize(input, cols, rows)).await
    }

    /// A wheel event while only watching: a short attach at the terminal's
    /// own size, closed right after the bytes ([`Self::answer`]).
    async fn wheel(&mut self, event: (ScrollDirection, u16)) -> Step {
        if let Some(w) = &mut self.pending_wheel {
            // On its way: the notches go with it (in order, so a change of
            // direction stays one).
            if w.events.len() < WHEEL_MAX {
                w.events.push(event);
            }
            return Step::Go;
        }
        self.ask_wheel(vec![event], false).await
    }

    /// The wheel `events` encoded for `size` in the program's mouse encoding
    /// as it is now: encoded when sent, so a grid that changed on the way
    /// never gets positions of the old one.
    fn wheel_input(&self, events: &[(ScrollDirection, u16)], size: (u16, u16)) -> Vec<u8> {
        let guard = self.parser.lock().expect("terminal parser lock");
        let screen = guard.0.screen();
        events
            .iter()
            .flat_map(|(direction, lines)| wheel_bytes(screen, size, *direction, *lines))
            .collect()
    }

    async fn ask_wheel(&mut self, events: Vec<(ScrollDirection, u16)>, retried: bool) -> Step {
        let id = self.next_id;
        self.next_id += 1;
        let (cols, rows) = self.grid;
        self.pending_wheel = Some(Wheel {
            id,
            at: Instant::now(),
            events,
            grid: (cols, rows),
            retried,
        });
        let request = json!({
            "jsonrpc": "2.0", "id": id, "method": "terminal.attach",
            "params": { "terminal": self.terminal, "cols": cols, "rows": rows, "resumed": true },
        });
        match serde_json::to_vec(&request) {
            Ok(message) => self.send(Frame::data(0, message)).await,
            Err(e) => Step::End(Ended::Closed(e.to_string())),
        }
    }

    /// Input goes to the interactive attach; without one, it asks for it.
    async fn type_bytes(&mut self, bytes: Vec<u8>) -> Step {
        self.type_input(Typed::Bytes(bytes)).await
    }

    /// Bytes or keys, in the order typed. Keys go as `terminal.keys` only
    /// once this session's interactive attach holds the size, so the core
    /// encodes them for the program at the app's size and after the bytes
    /// typed before them (the core reads one connection in order).
    async fn type_input(&mut self, typed: Typed) -> Step {
        if let Some(input) = self.input {
            if self.pending_keys.is_some() {
                self.queued.push(typed);
                return Step::Go;
            }
            if let Step::End(ended) = self.apply_want(input).await {
                return Step::End(ended);
            }
            return self.send_typed(input, typed).await;
        }
        self.queued.push(typed);
        if self.pending_attach.is_some() {
            return Step::Go;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (cols, rows) = self.want;
        self.pending_attach = Some((id, Instant::now(), (cols, rows)));
        // Attaching interactively takes the size.
        self.expect_echo((cols, rows));
        let request = json!({
            "jsonrpc": "2.0", "id": id, "method": "terminal.attach",
            "params": { "terminal": self.terminal, "cols": cols, "rows": rows },
        });
        match serde_json::to_vec(&request) {
            Ok(message) => self.send(Frame::data(0, message)).await,
            Err(e) => Step::End(Ended::Closed(e.to_string())),
        }
    }

    /// Bytes as DATA on the interactive attach; keys as a `terminal.keys`
    /// request. Until its answer, later input waits in the queue; a failed
    /// key is not worth ending the view.
    async fn send_typed(&mut self, input: u32, typed: Typed) -> Step {
        self.gone(&typed);
        match typed {
            Typed::Bytes(bytes) => self.send(Frame::data(input, bytes)).await,
            Typed::Keys(keys) => {
                // `terminal.keys` does not make this viewer active, and the
                // desktop may have taken the size without its RESIZE having
                // arrived yet: a RESIZE (even of the same size) makes the
                // app the active viewer again before the keys. Also without
                // notifications, where the app always sets the size.
                if let Step::End(ended) = self.reclaim(input).await {
                    return Step::End(ended);
                }
                let id = self.next_id;
                self.next_id += 1;
                self.pending_keys = Some((id, Instant::now()));
                self.keys_reclaimed = false;
                let request = json!({
                    "jsonrpc": "2.0", "id": id, "method": "terminal.keys",
                    "params": { "terminal": self.terminal, "keys": keys },
                });
                match serde_json::to_vec(&request) {
                    Ok(message) => self.send(Frame::data(0, message)).await,
                    Err(e) => Step::End(Ended::Closed(e.to_string())),
                }
            }
        }
    }

    /// `typed` leaves the session (sent now): its room in the budget is free.
    fn gone(&self, typed: &Typed) {
        match typed {
            Typed::Bytes(b) => self.budget.give(b.len(), 0),
            Typed::Keys(k) => self.budget.give(key_bytes(k), 1),
        }
    }

    /// Drop what waited (the session lets go).
    fn drop_queued(&mut self) {
        for typed in std::mem::take(&mut self.queued) {
            self.gone(&typed);
        }
    }

    /// Handle one frame from the core.
    async fn frame(&mut self, frame: Frame) -> Step {
        if frame.channel == 0 {
            if frame.kind == DATA {
                return self.answer(&frame.payload).await;
            }
            return Step::Go;
        }
        if frame.channel == self.shown {
            return self.shown_frame(frame).await;
        }
        if Some(frame.channel) == self.input {
            return self.input_frame(frame).await;
        }
        // An attach already closed by us: its last frames mean nothing.
        Step::Go
    }

    async fn shown_frame(&mut self, frame: Frame) -> Step {
        match frame.kind {
            DATA => {
                {
                    let mut guard = self.parser.lock().expect("terminal parser lock");
                    let (vt, seq) = &mut *guard;
                    vt.process(&frame.payload);
                    *seq += 1;
                }
                self.dirty.notify_one();
                self.unreported += frame.payload.len();
                if self.unreported >= CREDIT_STEP {
                    let credit = Frame::credit(self.shown, self.unreported as u32);
                    self.unreported = 0;
                    return self.send(credit).await;
                }
                Step::Go
            }
            RESIZE => {
                // The terminal's real grid, before the paint that fits it.
                let Some(size) = frame.size().filter(|(c, r)| sane_size(*c, *r).is_ok()) else {
                    return Step::Go;
                };
                tracing::debug!(cols = size.0, rows = size.1, "core terminal resized");
                // The first notification after attaching repeats the grid
                // the answer gave: no change, so no other viewer either.
                let unchanged = size == self.grid;
                self.with_screen(|s| s.set_size(size.1, size.0));
                self.grid = size;
                self.dirty.notify_one();
                if !self.watching {
                    // Not asked for: no viewers to tell apart.
                    return Step::Go;
                }
                // The echo of a size the app set, and of those before it (in
                // order: the core announces changes in order, skipping only
                // those it coalesced). An echo comes within [`ECHO_WAIT`]; an
                // older entry is one the core coalesced away, and must not be
                // taken for an echo later.
                let wait = self.echo_wait;
                self.sent.retain(|(_, at)| at.elapsed() < wait);
                // Through the first match. A size set twice (B, C, B) is
                // ambiguous: the core may announce B, C, B or coalesce into
                // one B, and a later C is then either the own echo or another
                // viewer. The first match keeps the app typing: a takeover
                // missed this way only leaves the idle typing attach open,
                // and the next key would take the size anyway.
                if let Some(at) = self.sent.iter().position(|(s, _)| *s == size) {
                    self.sent.drain(..=at);
                    return Step::Go;
                }
                if unchanged {
                    // No change: no other viewer either.
                    return Step::Go;
                }
                self.sent.clear();
                // An interactive attach still on its way sets its size after
                // this: its echo is still to come.
                if let Some((_, _, size)) = self.pending_attach {
                    self.sent.push_back((size, Instant::now()));
                }
                // Keys wait at the core for a quiet moment and are encoded
                // at the size it has then. This notification may be one the
                // RESIZE before the keys already overtook (sent before it,
                // still on its way): keep the interactive attach and take the
                // size back once more, so the keys still meet the app's size.
                // Once per keys: a desktop that keeps the size (someone
                // typing there) is not fought over.
                if self.pending_keys.is_some()
                    && !self.keys_reclaimed
                    && self.releasing.is_none()
                    && let Some(input) = self.input
                {
                    self.keys_reclaimed = true;
                    return self.reclaim(input).await;
                }
                // Another viewer (the desktop) took the size: stop typing
                // there and watch until the user types again.
                match self.input.take() {
                    Some(input) if self.releasing.is_none() => {
                        self.input_unreported = 0;
                        self.send(Frame::close(input)).await
                    }
                    _ => Step::Go,
                }
            }
            CLOSE => Step::End(if self.releasing.is_some() {
                Ended::Released
            } else if frame.payload.is_empty() {
                // Not asked for: the core let go of the view. The owner may
                // open a new one.
                Ended::Closed("closed by herdr-core".into())
            } else {
                Ended::Closed(close_reason(&frame.payload))
            }),
            _ => Step::Go,
        }
    }

    /// A frame of the interactive attach: its output is dropped (the watched
    /// attach shows the same), but credited so the core keeps it flowing.
    async fn input_frame(&mut self, frame: Frame) -> Step {
        match frame.kind {
            DATA => {
                self.input_unreported += frame.payload.len();
                if self.input_unreported >= CREDIT_STEP {
                    let credit = Frame::credit(frame.channel, self.input_unreported as u32);
                    self.input_unreported = 0;
                    return self.send(credit).await;
                }
                Step::Go
            }
            // The core ended the interactive attach (the program exited: the
            // watched attach gets its own CLOSE). Type through a new one.
            CLOSE => {
                self.input = None;
                Step::Go
            }
            _ => Step::Go,
        }
    }

    /// A JSON-RPC message on channel 0: the answer to an attach this session
    /// asked for (interactive, wheel, or one given up on).
    async fn answer(&mut self, payload: &[u8]) -> Step {
        let Some(id) = serde_json::from_slice::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| v.get("id").and_then(serde_json::Value::as_u64))
        else {
            return Step::Go;
        };
        let text = String::from_utf8_lossy(payload);
        let parsed = || -> Result<AttachResult> {
            let result = super::client::parse_message(&text, id)?
                .ok_or_else(|| Error::Protocol("attach: no result".into()))?;
            Ok(serde_json::from_value(result)?)
        };
        if self.leaving.is_some()
            && (self.pending_attach.is_some_and(|(p, _, _)| p == id)
                || self.pending_keys.is_some_and(|(k, _)| k == id))
        {
            // An answer while leaving: the wait for the next starts anew.
            self.leave_wait = Some(Instant::now());
        }
        if self.pending_attach.is_some_and(|(p, _, _)| p == id) {
            let (_, asked, size) = self.pending_attach.take().expect("pending attach");
            // The echo of the attach's size comes after the answer: its wait
            // starts now, and a slow link waits longer for every echo.
            let round_trip = asked.elapsed();
            self.echo_wait = ECHO_WAIT.max(round_trip * 2);
            let now = Instant::now();
            for (_, at) in self.sent.iter_mut() {
                *at = now;
            }
            let attached = match parsed() {
                Ok(a) => a,
                Err(e) => return Step::End(Ended::Closed(e.to_string())),
            };
            return self.attached(attached.channel, size).await;
        }
        if self.pending_wheel.as_ref().is_some_and(|w| w.id == id) {
            let wheel = self.pending_wheel.take().expect("pending wheel");
            let Ok(attached) = parsed() else {
                // A lost wheel event is no reason to end the view.
                return Step::Go;
            };
            let channel = attached.channel;
            // The user started typing meanwhile: the typing attach owns the
            // size, so the wheel goes there (at the app's size) and this one
            // is given back unused; typing at the old grid would flap the
            // size.
            let typing = self.input.is_some() || self.pending_attach.is_some();
            // The grid changed on the way: typing at the old size would set
            // it back. Give it back unused and ask anew (once).
            let stale = wheel.grid != self.grid;
            if self.releasing.is_none() && !typing && !stale {
                let bytes = self.wheel_input(&wheel.events, wheel.grid);
                if let Step::End(ended) = self.send(Frame::data(channel, bytes)).await {
                    return Step::End(ended);
                }
            }
            if let Step::End(ended) = self.send(Frame::close(channel)).await {
                return Step::End(ended);
            }
            if self.releasing.is_none() && typing {
                let bytes = self.wheel_input(&wheel.events, self.want);
                return self.type_bytes(bytes).await;
            }
            if stale && !wheel.retried && self.releasing.is_none() {
                return self.ask_wheel(wheel.events, true).await;
            }
            return Step::Go;
        }
        if self.pending_keys.is_some_and(|(k, _)| k == id) {
            self.pending_keys = None;
            if let Err(e) = parsed_any(&text, id) {
                tracing::debug!(%e, "terminal.keys failed");
                self.key_failed(e.to_string());
            }
            if self.releasing.is_some() {
                return Step::Go;
            }
            return self.flush_queued().await;
        }
        if let Some(at) = self.abandoned.iter().position(|a| *a == id) {
            self.abandoned.swap_remove(at);
            if let Ok(attached) = parsed() {
                return self.send(Frame::close(attached.channel)).await;
            }
        }
        Step::Go
    }

    /// The interactive attach is answered: type the queued input there, at
    /// the app's current size (it may have changed on the way).
    async fn attached(&mut self, input: u32, size: (u16, u16)) -> Step {
        if self.releasing.is_some() {
            // Answered after the user left: give it straight back, unused.
            self.drop_queued();
            let _ = self.write(&Frame::close(input)).await;
            return Step::Go;
        }
        self.input = Some(input);
        self.input_size = size;
        self.input_unreported = 0;
        if let Step::End(ended) = self.apply_want(input).await {
            return Step::End(ended);
        }
        self.flush_queued().await
    }

    /// Send what waited: bytes in as few frames as possible (keys in one
    /// frame are the same bytes to the program), up to and including the
    /// next keys, whose answer the rest waits for. Without an interactive
    /// attach (the desktop took the size meanwhile) it is asked for anew.
    async fn flush_queued(&mut self) -> Step {
        if self.pending_keys.is_some() {
            return Step::Go;
        }
        let Some(input) = self.input else {
            let queued = std::mem::take(&mut self.queued);
            for typed in queued {
                if let Step::End(ended) = self.type_input(typed).await {
                    return Step::End(ended);
                }
            }
            return Step::Go;
        };
        if self.queued.is_empty() {
            return Step::Go;
        }
        if let Step::End(ended) = self.apply_want(input).await {
            return Step::End(ended);
        }
        let mut bytes = Vec::new();
        let mut queued = std::mem::take(&mut self.queued).into_iter();
        for typed in queued.by_ref() {
            match typed {
                Typed::Bytes(b) => {
                    self.budget.give(b.len(), 0);
                    bytes.extend(b)
                }
                keys => {
                    if !bytes.is_empty() {
                        let data = Frame::data(input, std::mem::take(&mut bytes));
                        if let Step::End(ended) = self.send(data).await {
                            return Step::End(ended);
                        }
                    }
                    if let Step::End(ended) = self.send_typed(input, keys).await {
                        return Step::End(ended);
                    }
                    break;
                }
            }
        }
        self.queued = queued.collect();
        if bytes.is_empty() {
            return Step::Go;
        }
        self.send(Frame::data(input, bytes)).await
    }
}

/// Ready [`LEAVE_HARD`] after the owner let go; never before.
async fn hard_stop(let_go: &mut tokio::sync::watch::Receiver<Option<Instant>>) {
    loop {
        let at = *let_go.borrow_and_update();
        if let Some(at) = at {
            tokio::time::sleep_until(at + LEAVE_HARD).await;
            return;
        }
        if let_go.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// An answer's error, if it is one.
fn parsed_any(text: &str, id: u64) -> Result<()> {
    super::client::parse_message(text, id).map(drop)
}

/// Input waiting for the interactive attach (or for keys sent before it).
#[derive(Debug)]
enum Typed {
    Bytes(Vec<u8>),
    Keys(Vec<String>),
}

/// `{"reason":"exited","exit_code":0,…}` → "exited (exit code 0)".
fn close_reason(payload: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return String::from_utf8_lossy(payload).into_owned();
    };
    let reason = value
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("closed")
        .to_owned();
    match value.get("exit_code").and_then(|v| v.as_i64()) {
        Some(code) => format!("{reason} (exit code {code})"),
        None => reason,
    }
}

/// Type `bytes` into a terminal without anything after them (no Enter): a
/// short attach at the terminal's own size, so nothing is resized. Returns
/// once the core confirmed the detach with an empty CLOSE (the input is in);
/// otherwise an error (a timeout, a lost connection, or the terminal ended),
/// and the input may or may not have arrived: callers must not repeat it.
/// Everything, writes included, happens within [`TYPE_DEADLINE`].
pub async fn type_bytes(
    transport: &Transport,
    endpoint: &Endpoint,
    terminal: &str,
    cols: u16,
    rows: u16,
    bytes: &[u8],
) -> Result<()> {
    let (cols, rows) = if sane_size(cols, rows).is_ok() {
        (cols, rows)
    } else {
        (80, 24)
    };
    tokio::time::timeout(TYPE_DEADLINE, async {
        let (mut framed, attached) = attach(
            transport,
            endpoint,
            json!({ "terminal": terminal, "cols": cols, "rows": rows, "resumed": true }),
        )
        .await?;
        let channel = attached.channel;
        write_frame(&mut framed.writer, &Frame::data(channel, bytes.to_vec())).await?;
        write_frame(&mut framed.writer, &Frame::close(channel)).await?;
        loop {
            match read_frame(&mut framed.reader).await? {
                Some(frame) if frame.channel == channel && frame.kind == CLOSE => {
                    return if frame.payload.is_empty() {
                        Ok(())
                    } else {
                        // The terminal ended meanwhile: the input may not be in.
                        Err(Error::Protocol(close_reason(&frame.payload)))
                    };
                }
                Some(_) => {}
                None => return Err(Error::Protocol(LOST.into())),
            }
        }
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// A coloured read: the text and whether the paint was complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Painted {
    pub text: String,
    pub complete: bool,
}

/// Finds [`PAINT_END`] in a stream that arrives in pieces.
#[derive(Default)]
struct EndMark {
    /// The last bytes of the previous piece, for a marker split across two.
    tail: Vec<u8>,
}

impl EndMark {
    /// Where the paint ends in `piece` (the index after the marker), if it
    /// does.
    fn find(&mut self, piece: &[u8]) -> Option<usize> {
        let mut joined = std::mem::take(&mut self.tail);
        let carried = joined.len();
        joined.extend_from_slice(piece);
        let full = PAINT_END.len() + 1;
        let found = joined
            .windows(full)
            .position(|w| w.starts_with(PAINT_END) && matches!(w[full - 1], b'h' | b'l'))
            .map(|at| (at + full).saturating_sub(carried));
        if found.is_none() {
            let keep = joined.len().min(full - 1);
            self.tail = joined[joined.len() - keep..].to_vec();
        }
        found
    }
}

/// The screen and up to `lines` lines of history with their colours (SGR
/// codes, one line per row), from a watch-only attach that never types or
/// resizes. The read ends at the paint's own end marker ([`PAINT_END`]), so
/// gaps on a slow link or a program that keeps drawing never cut it short.
/// No complete paint within [`PAINT_DEADLINE`] (or the stream ending before
/// it) is an error, never a partial text.
pub async fn read_painted(
    transport: &Transport,
    endpoint: &Endpoint,
    terminal: &str,
    lines: u32,
) -> Result<Painted> {
    let deadline = Instant::now() + PAINT_DEADLINE;
    let mut painting = false;
    let read = tokio::time::timeout_at(deadline, async {
        // An observer's own size is ignored by the core; the answer and the
        // first RESIZE give the terminal's grid.
        let (mut framed, attached) = attach(
            transport,
            endpoint,
            json!({
                "terminal": terminal, "cols": 80, "rows": 24, "observe": true,
                "history": lines, "resize_notifications": true,
            }),
        )
        .await?;
        let channel = attached.channel;
        let (c, r) = match (attached.terminal.cols, attached.terminal.rows) {
            (c, r) if sane_size(c, r).is_ok() => (c, r),
            _ => (80, 24),
        };
        let mut parser = vt100::Parser::new(r, c, lines as usize);
        let mut end = EndMark::default();
        let mut unreported = 0usize;
        loop {
            let frame = read_frame(&mut framed.reader)
                .await?
                .ok_or_else(|| Error::Protocol("paint incomplete: connection closed".into()))?;
            if frame.channel != channel {
                continue;
            }
            match frame.kind {
                DATA => {
                    painting = true;
                    if let Some(at) = end.find(&frame.payload) {
                        parser.process(&frame.payload[..at]);
                        break;
                    }
                    parser.process(&frame.payload);
                    unreported += frame.payload.len();
                    if unreported >= CREDIT_STEP {
                        write_frame(
                            &mut framed.writer,
                            &Frame::credit(channel, unreported as u32),
                        )
                        .await?;
                        unreported = 0;
                    }
                }
                // A grid change mid-paint: the core drops what it had not
                // sent and paints anew without history (`queue_resize_paints`),
                // so the picture would be half old, half new. Read it plain.
                RESIZE if painting => {
                    let _ = write_frame(&mut framed.writer, &Frame::close(channel)).await;
                    return Err(Error::Protocol("paint superseded by a resize".into()));
                }
                RESIZE => {
                    if let Some((c, r)) = frame.size().filter(|(c, r)| sane_size(*c, *r).is_ok()) {
                        parser.screen_mut().set_size(r, c);
                    }
                }
                CLOSE => {
                    return Err(Error::Protocol(format!(
                        "paint incomplete: {}",
                        close_reason(&frame.payload)
                    )));
                }
                _ => {}
            }
        }
        let _ = write_frame(&mut framed.writer, &Frame::close(channel)).await;
        Ok(Painted {
            text: ansi_contents(&mut parser),
            complete: true,
        })
    })
    .await;
    match read {
        Ok(result) => result,
        Err(_) if painting => Err(Error::Protocol(NO_PAINT_END.into())),
        Err(_) => Err(Error::Timeout),
    }
}

#[cfg(test)]
mod tests;

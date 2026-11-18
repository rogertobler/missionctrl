//! The live session against a framed fake core: attaches, size ownership,
//! release and the connection's end.

use super::super::framed::CREDIT;
use super::*;
use crate::transport::LocalTransport;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[test]
fn wheel_follows_the_programs_encoding() {
    let mut vt = vt100::Parser::new(24, 80, 0);
    vt.process(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(
        wheel_bytes(vt.screen(), (80, 24), ScrollDirection::Up, 3),
        b"\x1b[<64;41;13M"
    );
    let mut vt = vt100::Parser::new(24, 80, 0);
    vt.process(b"\x1b[?1000h");
    assert_eq!(
        wheel_bytes(vt.screen(), (80, 24), ScrollDirection::Down, 6),
        [0x1b, b'[', b'M', 97, 73, 45].repeat(2)
    );
}

#[test]
fn close_reasons_read_well() {
    assert_eq!(
        close_reason(br#"{"reason":"exited","exit_code":0,"signal":null}"#),
        "exited (exit code 0)"
    );
    assert_eq!(close_reason(br#"{"reason":"killed"}"#), "killed");
}

/// How the fake core behaves.
#[derive(Clone)]
struct Config {
    /// Advertise and confirm RESIZE notifications for observers.
    notifications: bool,
    /// The terminal's grid before anyone types.
    grid: (u16, u16),
    /// Delay before answering an interactive attach.
    attach_delay: Duration,
    /// Paint of an observer, in pieces sent this far apart.
    paint: Vec<Vec<u8>>,
    paint_gap: Duration,
    /// End the paint with the core's marker and status records, as
    /// herdr-core's `first_paint` does.
    paint_end: bool,
    /// After the paint, close the observer with this reason.
    close_after_paint: Option<&'static [u8]>,
    /// Keep drawing on observers (a spinner) until they close.
    spinner: bool,
    /// Answer a viewer's CLOSE with a CLOSE carrying this payload (empty: a
    /// clean detach).
    answer_close: Option<&'static [u8]>,
    /// Delay of the RESIZE notifications to observers.
    echo_delay: Duration,
    /// Change the grid after the first piece of an observer's paint.
    resize_mid_paint: bool,
    /// Announce only the latest grid, and only when it differs from the one
    /// announced last, as the core's outbox does with notifications it has
    /// not sent yet.
    coalesce: bool,
    /// Delay before answering `terminal.keys` (the core waits for a quiet
    /// moment after typing).
    keys_delay: Duration,
    /// Refuse every `terminal.keys` with this message.
    keys_error: Option<&'static str>,
    /// Stop reading from the viewer after the first `terminal.keys` (still
    /// answering it), so the viewer's writes stall.
    stall_after_keys: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notifications: true,
            grid: (200, 50),
            attach_delay: Duration::ZERO,
            paint: vec![b"hello".to_vec()],
            paint_gap: Duration::ZERO,
            paint_end: true,
            close_after_paint: None,
            spinner: false,
            answer_close: Some(b""),
            echo_delay: Duration::ZERO,
            resize_mid_paint: false,
            coalesce: false,
            keys_delay: Duration::ZERO,
            keys_error: None,
            stall_after_keys: false,
        }
    }
}

/// The core's flow control: 256 KiB per channel to start, more with each
/// CREDIT; DATA waits while a channel has none left.
#[derive(Clone, Default)]
struct Credit {
    used: Arc<Mutex<std::collections::HashMap<u32, i64>>>,
    more: Arc<Notify>,
}

impl Credit {
    const INITIAL: i64 = 256 * 1024;

    async fn take(&self, channel: u32, len: usize) {
        loop {
            let more = self.more.notified();
            {
                let mut used = self.used.lock().unwrap();
                let used = used.entry(channel).or_default();
                if *used + len as i64 <= Self::INITIAL {
                    *used += len as i64;
                    return;
                }
            }
            more.await;
        }
    }

    fn give(&self, channel: u32, payload: &[u8]) {
        let Ok(bytes) = <[u8; 4]>::try_from(payload) else {
            return;
        };
        *self.used.lock().unwrap().entry(channel).or_default() -= u32::from_le_bytes(bytes) as i64;
        self.more.notify_waiters();
    }
}

#[derive(Clone)]
struct Fake {
    endpoint: Endpoint,
    /// Every frame a viewer sent (channel 0 included).
    got: Arc<Mutex<Vec<Frame>>>,
    /// Params of every `terminal.attach`, in order, with the channel given.
    attaches: Arc<Mutex<Vec<(u32, Value)>>>,
    /// Frames to the newest connection.
    push: Arc<Mutex<Option<mpsc::UnboundedSender<Frame>>>>,
    /// Connections still open.
    open: Arc<AtomicUsize>,
    /// Ends the newest connection from the core's side.
    kill: Arc<Notify>,
}

impl Fake {
    fn start(name: &str, config: Config) -> Self {
        let dir = std::env::temp_dir().join(format!("mc-test-{}-att-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("default.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let fake = Fake {
            endpoint: Endpoint::Socket(socket.to_string_lossy().into_owned()),
            got: Arc::default(),
            attaches: Arc::default(),
            push: Arc::default(),
            open: Arc::default(),
            kill: Arc::default(),
        };
        let accepting = fake.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (fake, config) = (accepting.clone(), config.clone());
                fake.open.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    fake.serve(stream, config).await;
                    fake.open.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        fake
    }

    async fn serve(&self, stream: tokio::net::UnixStream, config: Config) {
        let mut stream = BufReader::new(stream);
        let mut hello = String::new();
        if stream.read_line(&mut hello).await.unwrap_or(0) == 0 {
            return;
        }
        let features: Vec<&str> = if config.notifications {
            vec!["framed", "attach", "resize_notifications"]
        } else {
            vec!["framed", "attach"]
        };
        let reply = json!({"jsonrpc": "2.0", "id": 0, "result": {
            "framed": true, "generation": 1, "features": features}});
        if stream
            .get_mut()
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
        *self.push.lock().unwrap() = Some(tx.clone());
        let writing = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
        });
        // RESIZE notifications go out in order, each `echo_delay` late.
        let latest: Arc<Mutex<(u16, u16)>> = Arc::new(Mutex::new(config.grid));
        let announced: Arc<Mutex<(u16, u16)>> = Arc::new(Mutex::new(config.grid));
        let (echo_tx, mut echo_rx) = mpsc::unbounded_channel::<(Instant, (u16, u16), Vec<Frame>)>();
        let echoing = {
            let tx = tx.clone();
            let (latest, announced) = (latest.clone(), announced.clone());
            let coalesce = config.coalesce;
            tokio::spawn(async move {
                while let Some((due, grid, frames)) = echo_rx.recv().await {
                    tokio::time::sleep_until(due).await;
                    if coalesce {
                        let mut announced = announced.lock().unwrap();
                        if grid != *latest.lock().unwrap() || grid == *announced {
                            continue;
                        }
                        *announced = grid;
                    }
                    for frame in frames {
                        let _ = tx.send(frame);
                    }
                }
            })
        };
        let credit = Credit::default();
        let mut next = 7u32;
        let mut grid = config.grid;
        let observers: Arc<Mutex<Vec<u32>>> = Arc::default();
        let mut interactive: Vec<u32> = Vec::new();
        let announce = |observers: &Arc<Mutex<Vec<u32>>>, grid: (u16, u16)| {
            let frames = observers
                .lock()
                .unwrap()
                .iter()
                .flat_map(|o| {
                    [
                        Frame::resize(*o, grid.0, grid.1),
                        Frame::data(*o, b"repaint".to_vec()),
                    ]
                })
                .collect();
            *latest.lock().unwrap() = grid;
            let _ = echo_tx.send((Instant::now() + config.echo_delay, grid, frames));
        };
        loop {
            let frame = tokio::select! {
                frame = read_frame(&mut reader) => match frame {
                    Ok(Some(frame)) => frame,
                    _ => break,
                },
                _ = self.kill.notified() => break,
            };
            self.got.lock().unwrap().push(frame.clone());
            match (frame.channel, frame.kind) {
                (0, DATA) => {
                    let request: Value = serde_json::from_slice(&frame.payload).unwrap();
                    if request["method"] == "terminal.keys" {
                        let reply = match config.keys_error {
                            None => json!({"jsonrpc": "2.0", "id": request["id"], "result": {}}),
                            Some(why) => json!({"jsonrpc": "2.0", "id": request["id"],
                                "error": {"code": -32602, "message": why}}),
                        };
                        let answer = Frame::data(0, serde_json::to_vec(&reply).unwrap());
                        let (tx, delay) = (tx.clone(), config.keys_delay);
                        tokio::spawn(async move {
                            tokio::time::sleep(delay).await;
                            let _ = tx.send(answer);
                        });
                        if config.stall_after_keys {
                            std::future::pending::<()>().await;
                        }
                        continue;
                    }
                    if request["method"] != "terminal.attach" {
                        continue;
                    }
                    let params = request["params"].clone();
                    let channel = next;
                    next += 1;
                    self.attaches
                        .lock()
                        .unwrap()
                        .push((channel, params.clone()));
                    let observe = params["observe"] == true;
                    let notified =
                        observe && config.notifications && params["resize_notifications"] == true;
                    if !observe {
                        interactive.push(channel);
                        let size = (
                            params["cols"].as_u64().unwrap() as u16,
                            params["rows"].as_u64().unwrap() as u16,
                        );
                        if size != grid {
                            grid = size;
                            announce(&observers, grid);
                        }
                    }
                    let answer = Frame::data(
                        0,
                        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": request["id"],
                            "result": {"channel": channel, "resize_notifications": notified,
                                "terminal": {"address": "t1", "cols": grid.0, "rows": grid.1}}}))
                        .unwrap(),
                    );
                    let (tx, config, observers) = (tx.clone(), config.clone(), observers.clone());
                    let credit = credit.clone();
                    let shown = grid;
                    tokio::spawn(async move {
                        if !observe {
                            tokio::time::sleep(config.attach_delay).await;
                            let _ = tx.send(answer);
                            let _ = tx.send(Frame::data(channel, b"paint".to_vec()));
                            return;
                        }
                        let _ = tx.send(answer);
                        if notified {
                            observers.lock().unwrap().push(channel);
                            let _ = tx.send(Frame::resize(channel, shown.0, shown.1));
                        }
                        let send = |bytes: Vec<u8>| {
                            let (tx, credit) = (tx.clone(), credit.clone());
                            async move {
                                credit.take(channel, bytes.len()).await;
                                tx.send(Frame::data(channel, bytes)).is_ok()
                            }
                        };
                        for piece in &config.paint {
                            // The core's chunks are at most 16 KiB.
                            for chunk in piece.chunks(16 * 1024) {
                                send(chunk.to_vec()).await;
                            }
                            tokio::time::sleep(config.paint_gap).await;
                            if config.resize_mid_paint {
                                let _ = tx.send(Frame::resize(channel, 120, 40));
                                send(b"\x18\x1b[0mnew screen".to_vec()).await;
                            }
                        }
                        if config.paint_end {
                            send(b"\x1b[?9001l\x1b]7501;status\x07".to_vec()).await;
                        }
                        if let Some(reason) = config.close_after_paint {
                            let _ = tx.send(Frame {
                                channel,
                                kind: CLOSE,
                                payload: reason.to_vec(),
                            });
                            return;
                        }
                        while config.spinner && !tx.is_closed() {
                            if !send(b"|".to_vec()).await {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(30)).await;
                        }
                    });
                }
                (channel, CREDIT) => {
                    credit.give(channel, frame.payload.as_slice());
                }
                (channel, RESIZE) if interactive.contains(&channel) => {
                    if let Some(size) = frame.size().filter(|s| *s != grid) {
                        grid = size;
                        announce(&observers, grid);
                    }
                }
                (channel, CLOSE) => {
                    observers.lock().unwrap().retain(|o| *o != channel);
                    interactive.retain(|i| *i != channel);
                    if let Some(reason) = config.answer_close {
                        let _ = tx.send(Frame {
                            channel,
                            kind: CLOSE,
                            payload: reason.to_vec(),
                        });
                    }
                }
                _ => {}
            }
        }
        writing.abort();
        echoing.abort();
    }

    fn inject(&self, frame: Frame) {
        let _ = self.push.lock().unwrap().as_ref().unwrap().send(frame);
    }

    fn frames(&self, channel: u32, kind: u8) -> Vec<Frame> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.channel == channel && f.kind == kind)
            .cloned()
            .collect()
    }

    fn attaches(&self) -> Vec<(u32, Value)> {
        self.attaches.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct Sink {
    last: Mutex<Option<Screen>>,
    ended: Mutex<Option<Ended>>,
    changed: Notify,
}

impl TerminalSink for Sink {
    fn screen(&self, screen: Screen) {
        *self.last.lock().unwrap() = Some(screen);
        self.changed.notify_waiters();
    }
    fn ended(&self, why: Ended) {
        *self.ended.lock().unwrap() = Some(why);
        self.changed.notify_waiters();
    }
}

/// Wait until `check` holds, at most three seconds.
async fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn open(fake: &Fake, sink: &Arc<Sink>) -> TerminalSession {
    let transport = Transport::Local(LocalTransport);
    open_session(&transport, &fake.endpoint, "t1", 40, 10, sink.clone())
        .await
        .unwrap()
}

fn grid_of(sink: &Sink) -> Option<(u16, u16)> {
    sink.last.lock().unwrap().as_ref().map(|s| (s.cols, s.rows))
}

#[tokio::test]
async fn watches_at_the_real_size_then_takes_over_when_typing() {
    let fake = Fake::start("watch", Config::default());
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("the desktop-sized picture", || {
        grid_of(&sink) == Some((200, 50))
    })
    .await;
    let attaches = fake.attaches();
    assert_eq!(attaches.len(), 1, "opening only watches");
    assert_eq!(attaches[0].1["observe"], true);
    assert!(attaches[0].1["history"].as_u64().unwrap() > 0);

    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let (input, params) = fake.attaches()[1].clone();
    assert_eq!(
        (params["cols"].clone(), params["rows"].clone()),
        (json!(40), json!(10))
    );
    assert_ne!(params["observe"], true);
    until("the typed byte", || !fake.frames(input, DATA).is_empty()).await;
    assert_eq!(fake.frames(input, DATA)[0].payload, b"a");
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    // Typing more goes to the same attach; nothing ever sends FOCUS.
    session.input(b"b".to_vec());
    until("the second byte", || fake.frames(input, DATA).len() == 2).await;
    assert_eq!(fake.attaches().len(), 2);
    assert!(
        fake.got
            .lock()
            .unwrap()
            .iter()
            .all(|f| f.kind != super::super::framed::FOCUS)
    );
}

#[tokio::test]
async fn another_viewer_taking_the_size_ends_typing_until_the_next_key() {
    let fake = Fake::start("takeover", Config::default());
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    let watching = || sink.last.lock().unwrap().as_ref().map(|s| s.watching);
    until("a watched picture", || watching() == Some(true)).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    until("the picture says typing", || watching() == Some(false)).await;

    // The desktop types: the core announces its size to the watcher.
    fake.inject(Frame::resize(shown, 120, 40));
    until("the interactive attach closed", || {
        !fake.frames(input, CLOSE).is_empty()
    })
    .await;
    until("the desktop-sized picture", || {
        grid_of(&sink) == Some((120, 40))
    })
    .await;
    until("the picture says watching again", || {
        watching() == Some(true)
    })
    .await;

    // The next key attaches again at the phone's size.
    session.input(b"c".to_vec());
    until("a new interactive attach", || fake.attaches().len() == 3).await;
    let (again, params) = fake.attaches()[2].clone();
    assert_eq!(params["cols"], 40);
    until("the key", || !fake.frames(again, DATA).is_empty()).await;
    assert!(sink.ended.lock().unwrap().is_none());
}

#[tokio::test]
async fn echoes_of_own_sizes_keep_the_app_typing() {
    // Late echoes: every RESIZE notification arrives after the next resize.
    let fake = Fake::start(
        "echo",
        Config {
            echo_delay: Duration::from_millis(150),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    // Answered: otherwise the resizes go with the attach's own size.
    until("the first key", || !fake.frames(input, DATA).is_empty()).await;
    // A, B, A: 40×10 (attach), 50×12, back to 40×10, each with a key.
    session.resize(50, 12);
    session.input(b"b".to_vec());
    session.resize(40, 10);
    session.input(b"c".to_vec());
    until("both resizes", || fake.frames(input, RESIZE).len() == 2).await;
    until("the last echo", || grid_of(&sink) == Some((40, 10))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    session.input(b"d".to_vec());
    until("the key on the same attach", || {
        fake.frames(input, DATA).len() == 4
    })
    .await;
    assert!(
        fake.frames(input, CLOSE).is_empty(),
        "own echo is no takeover"
    );
    assert_eq!(fake.attaches().len(), 2);
}

#[tokio::test]
async fn coalesced_echoes_never_hide_a_desktop_takeover() {
    // A, B, A within one echo delay: the core announces nothing (the grid
    // is back where it was). The desktop then takes B: that is no echo.
    let fake = Fake::start(
        "coalesce",
        Config {
            echo_delay: Duration::from_millis(150),
            coalesce: true,
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    session.resize(50, 12);
    session.input(b"b".to_vec());
    session.resize(40, 10);
    session.input(b"c".to_vec());
    until("both resizes", || fake.frames(input, RESIZE).len() == 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fake.frames(input, CLOSE).is_empty());
    fake.inject(Frame::resize(shown, 50, 12));
    until("the desktop's size taken as a takeover", || {
        !fake.frames(input, CLOSE).is_empty()
    })
    .await;
}

#[tokio::test]
async fn a_wheel_burst_on_a_slow_link_scrolls_all_its_notches() {
    let fake = Fake::start(
        "wheel-burst",
        Config {
            paint: vec![b"\x1b[?1000h\x1b[?1006hhello".to_vec()],
            attach_delay: Duration::from_millis(200),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink) == Some((200, 50))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    for _ in 0..3 {
        session.scroll(ScrollDirection::Up, 3);
    }
    session.scroll(ScrollDirection::Down, 3);
    until("a wheel attach", || fake.attaches().len() == 2).await;
    let wheel = fake.attaches()[1].0;
    until("the wheel closed", || !fake.frames(wheel, CLOSE).is_empty()).await;
    let data = fake.frames(wheel, DATA);
    assert_eq!(data.len(), 1);
    let text = String::from_utf8_lossy(&data[0].payload).into_owned();
    assert_eq!(text.matches("\x1b[<64;").count(), 3, "{text:?}");
    assert!(
        text.ends_with('M') && text.contains("\x1b[<65;"),
        "{text:?}"
    );
    assert_eq!(fake.attaches().len(), 2, "one attach for the burst");
}

#[tokio::test]
async fn a_wheel_whose_grid_changed_on_the_way_is_asked_anew() {
    let fake = Fake::start(
        "wheel-stale",
        Config {
            paint: vec![b"\x1b[?1000h\x1b[?1006hhello".to_vec()],
            attach_delay: Duration::from_millis(200),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    until("a picture", || grid_of(&sink) == Some((200, 50))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.scroll(ScrollDirection::Up, 3);
    until("a wheel attach", || fake.attaches().len() == 2).await;
    // The desktop resizes while the wheel attach is on its way.
    fake.inject(Frame::resize(shown, 120, 40));
    until("asked anew", || fake.attaches().len() == 3).await;
    let (first, _) = fake.attaches()[1].clone();
    let (again, params) = fake.attaches()[2].clone();
    assert_eq!(
        (params["cols"].clone(), params["rows"].clone()),
        (json!(120), json!(40))
    );
    until("the new wheel closed", || {
        !fake.frames(again, CLOSE).is_empty()
    })
    .await;
    assert!(
        fake.frames(first, DATA).is_empty(),
        "nothing typed at the old size"
    );
    let data = fake.frames(again, DATA);
    assert_eq!(data.len(), 1);
    // Encoded for the new grid: the middle of 120×40.
    assert_eq!(
        String::from_utf8_lossy(&data[0].payload),
        "\x1b[<64;61;21M",
        "positions of the new grid"
    );
}

#[tokio::test]
async fn an_echo_after_a_slow_attach_is_still_the_apps_own() {
    // The attach takes longer than the usual echo wait; its echo comes
    // right after the answer.
    let fake = Fake::start(
        "slow-attach",
        Config {
            attach_delay: Duration::from_millis(3200),
            echo_delay: Duration::from_millis(3300),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    tokio::time::sleep(Duration::from_millis(3000)).await;
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        fake.frames(input, CLOSE).is_empty(),
        "a late echo of the own attach is no takeover"
    );
}

#[tokio::test]
async fn a_takeover_while_attaching_keeps_the_attachs_echo() {
    // The desktop resizes while the phone's attach is on its way; the
    // attach's own echo comes after that.
    let fake = Fake::start(
        "takeover-pending",
        Config {
            attach_delay: Duration::from_millis(300),
            echo_delay: Duration::from_millis(400),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    session.input(b"a".to_vec());
    until("an interactive attach asked", || fake.attaches().len() == 2).await;
    fake.inject(Frame::resize(shown, 120, 40));
    let input = fake.attaches()[1].0;
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        fake.frames(input, CLOSE).is_empty(),
        "the attach's own echo is no takeover"
    );
}

#[tokio::test]
async fn a_wheel_answered_after_typing_began_goes_with_the_typing() {
    let fake = Fake::start(
        "wheel-typing",
        Config {
            paint: vec![b"\x1b[?1000h\x1b[?1006hhello".to_vec()],
            attach_delay: Duration::from_millis(200),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink) == Some((200, 50))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.scroll(ScrollDirection::Up, 3);
    until("a wheel attach", || fake.attaches().len() == 2).await;
    session.input(b"x".to_vec());
    until("an interactive attach", || fake.attaches().len() == 3).await;
    let wheel = fake.attaches()[1].0;
    let input = fake.attaches()[2].0;
    until("the wheel closed", || !fake.frames(wheel, CLOSE).is_empty()).await;
    until("the typing", || {
        let text: String = fake
            .frames(input, DATA)
            .iter()
            .map(|f| String::from_utf8_lossy(&f.payload).into_owned())
            .collect();
        text.contains('x') && text.contains("\x1b[<64;21;6M")
    })
    .await;
    assert!(
        fake.frames(wheel, DATA).is_empty(),
        "nothing typed at the old grid"
    );
}

#[tokio::test]
async fn a_resize_waits_for_the_next_input() {
    let fake = Fake::start("defer", Config::default());
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink) == Some((200, 50))).await;
    // Only watching: a resize changes nothing at the desktop.
    session.resize(50, 12);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(fake.got.lock().unwrap().iter().all(|f| f.kind != RESIZE));
    assert_eq!(grid_of(&sink), Some((200, 50)));
    assert_eq!(fake.attaches().len(), 1);

    // Typing attaches at the app's newest size.
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let (input, params) = fake.attaches()[1].clone();
    assert_eq!(
        (params["cols"].clone(), params["rows"].clone()),
        (json!(50), json!(12))
    );
    until("the key", || !fake.frames(input, DATA).is_empty()).await;

    // Typing: the next resize still waits, then goes right before the key.
    session.resize(60, 14);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(fake.frames(input, RESIZE).is_empty());
    session.input(b"b".to_vec());
    until("the key", || fake.frames(input, DATA).len() == 2).await;
    let order: Vec<u8> = fake
        .got
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f.channel == input && (f.kind == RESIZE || f.kind == DATA))
        .map(|f| f.kind)
        .collect();
    assert_eq!(order, [DATA, RESIZE, DATA]);
    assert_eq!(fake.frames(input, RESIZE)[0].size(), Some((60, 14)));
    until("the new picture", || grid_of(&sink) == Some((60, 14))).await;
}

#[tokio::test]
async fn a_resize_while_attaching_goes_before_the_queued_input() {
    let fake = Fake::start(
        "slow-attach",
        Config {
            attach_delay: Duration::from_millis(300),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("the attach request", || fake.attaches().len() == 2).await;
    session.resize(50, 12);
    let input = fake.attaches()[1].0;
    until("the key", || !fake.frames(input, DATA).is_empty()).await;
    let order: Vec<(u8, Option<(u16, u16)>)> = fake
        .got
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f.channel == input && (f.kind == RESIZE || f.kind == DATA))
        .map(|f| (f.kind, (f.kind == RESIZE).then(|| f.size()).flatten()))
        .collect();
    assert_eq!(order, [(RESIZE, Some((50, 12))), (DATA, None)]);
    assert!(fake.frames(input, CLOSE).is_empty());
}

#[tokio::test]
async fn the_wheel_while_watching_leaves_the_size_alone() {
    let fake = Fake::start(
        "wheel",
        Config {
            // A program that asks for the mouse (SGR encoding).
            paint: vec![b"\x1b[?1000h\x1b[?1006hhello".to_vec()],
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink) == Some((200, 50))).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.scroll(ScrollDirection::Up, 3);
    until("a wheel attach", || fake.attaches().len() == 2).await;
    let (wheel, params) = fake.attaches()[1].clone();
    assert_eq!(params["resumed"], true);
    assert_ne!(params["observe"], true);
    assert_eq!(
        (params["cols"].clone(), params["rows"].clone()),
        (json!(200), json!(50))
    );
    until("the wheel closed", || !fake.frames(wheel, CLOSE).is_empty()).await;
    let data = fake.frames(wheel, DATA);
    assert_eq!(data.len(), 1);
    assert!(
        data[0].payload.starts_with(b"\x1b[<64;"),
        "{:?}",
        data[0].payload
    );
    assert!(fake.got.lock().unwrap().iter().all(|f| f.kind != RESIZE));
    assert_eq!(grid_of(&sink), Some((200, 50)));
    assert!(sink.ended.lock().unwrap().is_none());
}

#[tokio::test]
async fn the_core_letting_go_of_the_view_ends_it() {
    let fake = Fake::start("let-go", Config::default());
    let sink = Arc::new(Sink::default());
    let _session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink).is_some()).await;
    fake.inject(Frame::close(fake.attaches()[0].0));
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(
        *sink.ended.lock().unwrap(),
        Some(Ended::Closed("closed by herdr-core".into()))
    );
}

#[tokio::test]
async fn release_ends_the_session_and_its_connection() {
    for answer_close in [Some(&b""[..]), None] {
        let fake = Fake::start(
            &format!("release-{}", answer_close.is_some()),
            Config {
                answer_close,
                ..Config::default()
            },
        );
        let sink = Arc::new(Sink::default());
        let session = open(&fake, &sink).await;
        session.input(b"a".to_vec());
        until("an interactive attach", || fake.attaches().len() == 2).await;
        assert_eq!(fake.open.load(Ordering::SeqCst), 1);
        let started = Instant::now();
        drop(session);
        until("the end", || sink.ended.lock().unwrap().is_some()).await;
        assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
        assert!(started.elapsed() < RELEASE_WAIT + Duration::from_millis(500));
        until("the connection closed", || {
            fake.open.load(Ordering::SeqCst) == 0
        })
        .await;
        let (shown, input) = (fake.attaches()[0].0, fake.attaches()[1].0);
        assert_eq!(fake.frames(shown, CLOSE).len(), 1);
        assert_eq!(fake.frames(input, CLOSE).len(), 1);
    }
}

#[tokio::test]
async fn twenty_sessions_leave_no_connection_open() {
    let fake = Fake::start("twenty", Config::default());
    for round in 1..=20 {
        let sink = Arc::new(Sink::default());
        let session = open(&fake, &sink).await;
        session.input(b"x".to_vec());
        until("an interactive attach", || {
            fake.attaches().len() == 2 * round
        })
        .await;
        session.release();
        until("the end", || sink.ended.lock().unwrap().is_some()).await;
    }
    until("all connections closed", || {
        fake.open.load(Ordering::SeqCst) == 0
    })
    .await;
}

#[tokio::test]
async fn an_attach_answered_after_release_is_given_back() {
    let fake = Fake::start(
        "late",
        Config {
            // Later than the session waits for it after letting go.
            attach_delay: Duration::from_millis(1300),
            answer_close: None,
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("the attach request", || fake.attaches().len() == 2).await;
    session.release();
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let late = fake.attaches()[1].0;
    assert!(fake.frames(late, DATA).is_empty(), "no input after leaving");
    assert_eq!(fake.frames(late, CLOSE).len(), 1, "handed straight back");
}

#[tokio::test]
async fn a_dropped_connection_is_reported_as_lost() {
    let fake = Fake::start("lost", Config::default());
    let sink = Arc::new(Sink::default());
    let _session = open(&fake, &sink).await;
    until("a picture", || grid_of(&sink).is_some()).await;
    fake.kill.notify_waiters();
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(
        *sink.ended.lock().unwrap(),
        Some(Ended::Closed(LOST.into()))
    );
}

#[tokio::test]
async fn without_notifications_the_app_sets_the_size() {
    let fake = Fake::start(
        "plain",
        Config {
            notifications: false,
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("two attaches", || fake.attaches().len() == 2).await;
    let (input, params) = fake.attaches()[1].clone();
    assert_ne!(params["observe"], true);
    assert_eq!(params["cols"], 40);
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    session.input(b"a".to_vec());
    until("the key", || !fake.frames(input, DATA).is_empty()).await;
    session.resize(50, 12);
    until("the resize", || !fake.frames(input, RESIZE).is_empty()).await;
    until("the new picture", || grid_of(&sink) == Some((50, 12))).await;
    assert_eq!(fake.attaches().len(), 2);
}

#[tokio::test]
async fn a_paint_in_pieces_is_read_whole() {
    let fake = Fake::start(
        "paint",
        Config {
            paint: vec![b"one ".to_vec(), b"two".to_vec()],
            paint_gap: Duration::from_millis(20),
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let painted = read_painted(&transport, &fake.endpoint, "t1", 10)
        .await
        .unwrap();
    assert!(painted.complete);
    assert!(painted.text.contains("one two"), "{}", painted.text);
    until("the connection closed", || {
        fake.open.load(Ordering::SeqCst) == 0
    })
    .await;
}

#[tokio::test]
async fn a_slow_paint_is_read_to_its_end() {
    let fake = Fake::start(
        "slow-paint",
        Config {
            paint: vec![b"one ".to_vec(), b"two ".to_vec(), b"three".to_vec()],
            paint_gap: Duration::from_millis(150),
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let painted = read_painted(&transport, &fake.endpoint, "t1", 10)
        .await
        .unwrap();
    assert!(painted.complete);
    assert!(painted.text.contains("one two three"), "{}", painted.text);
}

#[tokio::test]
async fn a_paint_beyond_the_first_credit_is_read_whole() {
    // 300 lines of 1 KiB: more than the core sends without CREDIT.
    let line = |n: usize| {
        let mut l = format!("{n:04}").into_bytes();
        l.resize(1024, b'x');
        l.extend_from_slice(b"\r\n");
        l
    };
    let paint: Vec<u8> = (0..300).flat_map(line).collect();
    assert!(paint.len() > 256 * 1024);
    let fake = Fake::start(
        "big-paint",
        Config {
            grid: (1100, 50),
            paint: vec![paint],
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let painted = read_painted(&transport, &fake.endpoint, "t1", 400)
        .await
        .unwrap();
    assert!(painted.complete);
    assert!(painted.text.contains("0299x"), "the last line");
    assert!(
        !fake.frames(fake.attaches()[0].0, CREDIT).is_empty(),
        "credit given"
    );
}

#[tokio::test]
async fn a_busy_program_is_read_at_the_paints_end() {
    let fake = Fake::start(
        "spinner",
        Config {
            spinner: true,
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let started = Instant::now();
    let painted = read_painted(&transport, &fake.endpoint, "t1", 10)
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(painted.complete);
    assert!(painted.text.starts_with("hello"));
    assert!(!painted.text.contains('|'), "nothing after the paint");
}

#[tokio::test]
async fn a_paint_without_its_end_is_an_error() {
    let fake = Fake::start(
        "no-end",
        Config {
            paint_end: false,
            close_after_paint: Some(br#"{"reason":"exited","exit_code":1}"#),
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let read = read_painted(&transport, &fake.endpoint, "t1", 10).await;
    assert!(
        matches!(&read, Err(Error::Protocol(m)) if m.contains("exited")),
        "{read:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_paint_that_never_ends_is_told_apart_from_silence() {
    let fake = Fake::start(
        "no-end-quiet",
        Config {
            paint_end: false,
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let read = read_painted(&transport, &fake.endpoint, "t1", 10).await;
    assert!(
        matches!(&read, Err(e) if is_paint_without_end(e)),
        "{read:?}"
    );
}

#[test]
fn the_paints_end_is_found_across_pieces() {
    let mut end = EndMark::default();
    // Split inside the marker, and right before its last byte.
    assert_eq!(end.find(b"screen\x1b[?90"), None);
    assert_eq!(end.find(b"01lrest"), Some(3));
    let mut end = EndMark::default();
    assert_eq!(end.find(b"x\x1b[?9001"), None);
    assert_eq!(end.find(b"h"), Some(1));
    // Another final byte is no end.
    let mut end = EndMark::default();
    assert_eq!(end.find(b"\x1b[?9001x"), None);
}

#[tokio::test]
async fn a_paint_split_at_its_end_is_read_whole() {
    let fake = Fake::start(
        "split-end",
        Config {
            paint: vec![b"done \x1b[?90".to_vec()],
            paint_end: false,
            close_after_paint: None,
            ..Config::default()
        },
    );
    // The rest of the marker comes in the next frame.
    let reading = tokio::spawn({
        let endpoint = fake.endpoint.clone();
        async move { read_painted(&Transport::Local(LocalTransport), &endpoint, "t1", 10).await }
    });
    until("attached", || !fake.attaches().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    fake.inject(Frame::data(fake.attaches()[0].0, b"01l".to_vec()));
    let painted = reading.await.unwrap().unwrap();
    assert!(painted.complete);
    assert!(painted.text.starts_with("done"), "{}", painted.text);
}

#[tokio::test]
async fn a_resize_in_the_middle_of_a_paint_is_an_error() {
    let fake = Fake::start(
        "resize-mid-paint",
        Config {
            paint: vec![b"old history ".to_vec(), b"more".to_vec()],
            resize_mid_paint: true,
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let read = read_painted(&transport, &fake.endpoint, "t1", 10).await;
    assert!(
        matches!(&read, Err(Error::Protocol(m)) if m.contains("resize")),
        "{read:?}"
    );
}

#[tokio::test]
async fn typing_into_a_terminal_that_ends_is_an_error() {
    let fake = Fake::start(
        "type-ended",
        Config {
            answer_close: Some(br#"{"reason":"exited","exit_code":0}"#),
            ..Config::default()
        },
    );
    let transport = Transport::Local(LocalTransport);
    let typed = type_bytes(&transport, &fake.endpoint, "t1", 80, 24, b"draft").await;
    assert!(matches!(typed, Err(Error::Protocol(_))), "{typed:?}");
}

#[tokio::test]
async fn typing_without_enter_waits_for_the_core() {
    let fake = Fake::start("type", Config::default());
    let transport = Transport::Local(LocalTransport);
    type_bytes(&transport, &fake.endpoint, "t1", 80, 24, b"draft")
        .await
        .unwrap();
    let (channel, params) = fake.attaches()[0].clone();
    assert_eq!(params["resumed"], true);
    assert_eq!(fake.frames(channel, DATA)[0].payload, b"draft");
}

#[tokio::test]
async fn named_keys_take_the_size_and_keep_their_place() {
    let fake = Fake::start(
        "keys",
        Config {
            attach_delay: Duration::from_millis(200),
            keys_delay: Duration::from_millis(300),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("the desktop-sized picture", || {
        grid_of(&sink) == Some((200, 50))
    })
    .await;
    session.input(b"a".to_vec());
    session.keys(vec!["ctrl+c".into()]);
    session.input(b"b".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let (input, params) = fake.attaches()[1].clone();
    assert_eq!(
        (params["cols"].clone(), params["rows"].clone()),
        (json!(40), json!(10))
    );
    until("the last byte", || {
        fake.frames(input, DATA).iter().any(|f| f.payload == b"b")
    })
    .await;
    // In order: the byte before, the keys (after the attach took the size),
    // the byte after (only once the keys were answered).
    let order: Vec<String> = fake
        .got
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f.kind == DATA && (f.channel == input || f.channel == 0))
        .filter_map(|f| {
            if f.channel == input {
                return Some(String::from_utf8_lossy(&f.payload).into_owned());
            }
            let v: Value = serde_json::from_slice(&f.payload).ok()?;
            (v["method"] == "terminal.keys").then(|| {
                assert_eq!(v["params"]["terminal"], "t1");
                format!("keys {}", v["params"]["keys"])
            })
        })
        .collect();
    assert_eq!(order, ["a", r#"keys ["ctrl+c"]"#, "b"]);
    // Keys typed with the attach open: later input still waits for them.
    session.keys(vec!["esc".into()]);
    session.input(b"c".to_vec());
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !fake.frames(input, DATA).iter().any(|f| f.payload == b"c"),
        "input after keys waits for their answer"
    );
    until("the byte after the second keys", || {
        fake.frames(input, DATA).iter().any(|f| f.payload == b"c")
    })
    .await;
    let got = fake.got.lock().unwrap().clone();
    let keys_at = got
        .iter()
        .rposition(|f| {
            f.channel == 0 && f.kind == DATA && f.payload.windows(4).any(|w| w == b"esc\"")
        })
        .expect("the second keys");
    let c_at = got
        .iter()
        .position(|f| f.channel == input && f.kind == DATA && f.payload == b"c")
        .unwrap();
    assert!(keys_at < c_at);
}

/// Every DATA the core got on `input` (as text) and each `terminal.keys`
/// (as `keys [...]`), in order.
fn typed_order(fake: &Fake, input: u32) -> Vec<String> {
    fake.got
        .lock()
        .unwrap()
        .iter()
        .filter(|f| {
            (f.kind == DATA && (f.channel == input || f.channel == 0))
                || (f.kind == RESIZE && f.channel == input)
        })
        .filter_map(|f| {
            if f.kind == RESIZE {
                return Some("resize".into());
            }
            if f.channel == input {
                return Some(String::from_utf8_lossy(&f.payload).into_owned());
            }
            let v: Value = serde_json::from_slice(&f.payload).ok()?;
            (v["method"] == "terminal.keys").then(|| format!("keys {}", v["params"]["keys"]))
        })
        .collect()
}

#[tokio::test]
async fn unanswered_keys_are_never_overtaken() {
    let fake = Fake::start(
        "keys-never",
        Config {
            keys_delay: Duration::from_secs(30),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.keys(vec!["esc".into()]);
    session.input(b"b".to_vec());
    until("the keys reported as pending", || {
        sink.last
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.keys_pending)
    })
    .await;
    // Past the deadline the session ends instead of sending what followed.
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(
        *sink.ended.lock().unwrap(),
        Some(Ended::Closed("the terminal did not answer".into()))
    );
    let order = typed_order(&fake, input);
    assert!(order.contains(&r#"keys ["esc"]"#.to_string()), "{order:?}");
    assert!(!order.contains(&"b".to_string()), "{order:?}");
}

#[tokio::test]
async fn letting_go_waits_for_pending_keys_and_sends_what_followed() {
    let fake = Fake::start(
        "keys-leave",
        Config {
            keys_delay: Duration::from_millis(400),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.keys(vec!["shift+enter".into()]);
    session.input(b"b".to_vec());
    drop(session);
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
    let order = typed_order(&fake, input);
    let keys_at = order.iter().position(|o| o == r#"keys ["shift+enter"]"#);
    let b_at = order.iter().position(|o| o == "b");
    assert!(
        keys_at.is_some() && b_at.is_some() && keys_at < b_at,
        "{order:?}"
    );
    // The byte after went out before the attach was closed.
    let got = fake.got.lock().unwrap().clone();
    let b_frame = got
        .iter()
        .position(|f| f.channel == input && f.kind == DATA && f.payload == b"b")
        .unwrap();
    let close = got
        .iter()
        .position(|f| f.channel == input && f.kind == CLOSE)
        .unwrap();
    assert!(b_frame < close);
}

#[tokio::test]
async fn keys_take_the_size_back_even_when_it_looks_unchanged() {
    let fake = Fake::start("keys-size", Config::default());
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    until("the byte", || {
        fake.frames(input, DATA).iter().any(|f| f.payload == b"a")
    })
    .await;
    session.keys(vec!["esc".into()]);
    until("the keys", || {
        typed_order(&fake, input)
            .iter()
            .any(|o| o.starts_with("keys"))
    })
    .await;
    let order = typed_order(&fake, input);
    assert_eq!(order, ["a", "resize", r#"keys ["esc"]"#]);
    let resize = fake.frames(input, RESIZE).pop().unwrap();
    let size = (
        u16::from_le_bytes([resize.payload[0], resize.payload[1]]),
        u16::from_le_bytes([resize.payload[2], resize.payload[3]]),
    );
    assert_eq!(size, (40, 10));
}

#[tokio::test]
async fn refused_keys_are_reported_once() {
    let fake = Fake::start(
        "keys-refused",
        Config {
            keys_error: Some("unknown key"),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    session.keys(vec!["nope".into()]);
    session.input(b"b".to_vec());
    until("the error", || {
        sink.last
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.key_errors == 1 && !s.keys_pending)
    })
    .await;
    let error = sink
        .last
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .key_error
        .clone()
        .unwrap();
    assert!(error.contains("unknown key"), "{error}");
    let input = fake.attaches()[1].0;
    until("the byte after", || {
        fake.frames(input, DATA).iter().any(|f| f.payload == b"b")
    })
    .await;
}

#[tokio::test]
async fn letting_go_before_the_attach_answers_still_types() {
    let fake = Fake::start(
        "leave-attaching",
        Config {
            attach_delay: Duration::from_millis(300),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    drop(session);
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
    let input = fake.attaches()[1].0;
    let got = fake.got.lock().unwrap().clone();
    let a = got
        .iter()
        .position(|f| f.channel == input && f.kind == DATA && f.payload == b"a")
        .expect("the byte typed before leaving");
    let close = got
        .iter()
        .position(|f| f.channel == input && f.kind == CLOSE)
        .unwrap();
    assert!(a < close);
}

#[tokio::test]
async fn letting_go_after_a_takeover_with_keys_pending_still_types() {
    let fake = Fake::start(
        "leave-takeover",
        Config {
            keys_delay: Duration::from_millis(400),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.keys(vec!["ctrl+enter".into()]);
    session.input(b"b".to_vec());
    until("the keys", || {
        typed_order(&fake, input)
            .iter()
            .any(|o| o.starts_with("keys"))
    })
    .await;
    // The desktop takes the size twice: once taken back for the keys, the
    // second time the typing attach is closed.
    fake.inject(Frame::resize(shown, 120, 40));
    until("the size taken back", || {
        fake.frames(input, RESIZE).len() == 2
    })
    .await;
    // Its echo (the fake core does not move off a size it never left).
    fake.inject(Frame::resize(shown, 40, 10));
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    fake.inject(Frame::resize(shown, 120, 40));
    until("the typing attach closed", || {
        !fake.frames(input, CLOSE).is_empty()
    })
    .await;
    drop(session);
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
    // What was typed after the keys went through a new attach.
    assert_eq!(fake.attaches().len(), 3);
    let again = fake.attaches()[2].0;
    assert!(fake.frames(again, DATA).iter().any(|f| f.payload == b"b"));
}

#[tokio::test]
async fn a_late_desktop_size_while_keys_wait_is_taken_back_once() {
    let fake = Fake::start(
        "keys-late-desktop",
        Config {
            keys_delay: Duration::from_millis(400),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    let shown = fake.attaches()[0].0;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.keys(vec!["ctrl+enter".into()]);
    until("the keys", || {
        typed_order(&fake, input)
            .iter()
            .any(|o| o.starts_with("keys"))
    })
    .await;
    // A desktop notification sent before the RESIZE for the keys arrives
    // after it: the size is taken back, the typing attach stays.
    fake.inject(Frame::resize(shown, 120, 40));
    until("the size taken back", || {
        fake.frames(input, RESIZE).len() == 2
    })
    .await;
    let resize = fake.frames(input, RESIZE).pop().unwrap();
    assert_eq!(resize.size(), Some((40, 10)));
    // Its echo (the fake core does not move off a size it never left).
    fake.inject(Frame::resize(shown, 40, 10));
    until("the phone-sized picture", || {
        grid_of(&sink) == Some((40, 10))
    })
    .await;
    session.input(b"b".to_vec());
    until("the byte after the keys", || {
        fake.frames(input, DATA).iter().any(|f| f.payload == b"b")
    })
    .await;
    assert!(fake.frames(input, CLOSE).is_empty());
    assert_eq!(fake.attaches().len(), 2);
}

#[tokio::test]
async fn without_notifications_keys_take_the_size_back() {
    let fake = Fake::start(
        "keys-plain",
        Config {
            notifications: false,
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    until("two attaches", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.input(b"a".to_vec());
    session.keys(vec!["ctrl+enter".into()]);
    until("the keys", || {
        typed_order(&fake, input)
            .iter()
            .any(|o| o.starts_with("keys"))
    })
    .await;
    assert_eq!(
        typed_order(&fake, input),
        ["a", "resize", r#"keys ["ctrl+enter"]"#]
    );
    assert_eq!(fake.frames(input, RESIZE)[0].size(), Some((40, 10)));
}

#[tokio::test]
async fn keys_unanswered_while_leaving_end_as_released() {
    let fake = Fake::start(
        "keys-leave-late",
        Config {
            keys_delay: Duration::from_secs(30),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    session.keys(vec!["ctrl+enter".into()]);
    // Left shortly before the keys' deadline, which then comes first.
    tokio::time::sleep(Duration::from_millis(800)).await;
    drop(session);
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
}

#[tokio::test]
async fn letting_go_waits_anew_for_each_answered_key() {
    // Two keys, each answered after a quiet moment: the second answer comes
    // after the first wait would have run out, and what followed it still
    // goes out.
    let fake = Fake::start(
        "keys-leave-each",
        Config {
            keys_delay: Duration::from_millis(700),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    let input = fake.attaches()[1].0;
    session.keys(vec!["ctrl+enter".into()]);
    session.input(b"b".to_vec());
    session.keys(vec!["ctrl+tab".into()]);
    session.input(b"c".to_vec());
    drop(session);
    until("the end", || sink.ended.lock().unwrap().is_some()).await;
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
    let order = typed_order(&fake, input);
    let at = |what: &str| order.iter().position(|o| o == what);
    assert!(
        at("b").is_some()
            && at(r#"keys ["ctrl+tab"]"#) > at("b")
            && at("c") > at(r#"keys ["ctrl+tab"]"#),
        "{order:?}"
    );
}

#[tokio::test]
async fn a_session_ends_soon_after_letting_go_even_while_a_write_stalls() {
    // The core stops reading after the keys: the input queued behind them
    // cannot be written. The session still ends within LEAVE_HARD of letting
    // go (not after the write timeout), so the owner can open the next one.
    let fake = Fake::start(
        "leave-hard",
        Config {
            keys_delay: Duration::from_millis(200),
            stall_after_keys: true,
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    session.keys(vec!["ctrl+enter".into()]);
    session.input(vec![b'x'; 3 << 20]);
    let let_go = Instant::now();
    drop(session);
    let deadline = let_go + super::LEAVE_HARD + Duration::from_millis(1500);
    while sink.ended.lock().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the session did not end in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*sink.ended.lock().unwrap(), Some(Ended::Released));
    assert!(let_go.elapsed() >= super::LEAVE_HARD - Duration::from_millis(100));
}

#[tokio::test]
async fn input_past_the_budget_is_refused_and_counted() {
    let fake = Fake::start(
        "budget",
        Config {
            keys_delay: Duration::from_secs(30),
            ..Config::default()
        },
    );
    let sink = Arc::new(Sink::default());
    let session = open(&fake, &sink).await;
    session.input(b"a".to_vec());
    until("an interactive attach", || fake.attaches().len() == 2).await;
    // Groups of keys wait behind the first one; past the budget the rest is cut.
    for _ in 0..300 {
        session.keys(vec!["ctrl+enter".into()]);
    }
    session.input(b"b".to_vec());
    until("the refusal reported", || {
        sink.last
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.input_refused == 1)
    })
    .await;
}

#[tokio::test]
async fn a_controller_that_dies_still_reports_its_end() {
    // The end guard reports even when the task is torn down: dropping a
    // runtime drops its tasks without running them to the end.
    let fake = Fake::start("teardown", Config::default());
    let sink = Arc::new(Sink::default());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let endpoint = fake.endpoint.clone();
    let (sink2, session) = (sink.clone(), {
        let sink = sink.clone();
        rt.spawn(async move {
            let transport = Transport::Local(LocalTransport);
            open_session(&transport, &endpoint, "t1", 40, 10, sink)
                .await
                .unwrap()
        })
    });
    let session = session.await.unwrap();
    assert!(sink2.ended.lock().unwrap().is_none());
    tokio::task::spawn_blocking(move || drop(rt)).await.unwrap();
    assert!(sink.ended.lock().unwrap().is_some());
    drop(session);
}

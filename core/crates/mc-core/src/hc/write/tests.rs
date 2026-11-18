//! A small fake herdr-core: `doc.get`/`doc.set` with versions, terminals with
//! labels, parents and states, and hooks that act like a desktop app at the
//! right moment (adopt, write, park) or lose an answer.

use super::*;
use crate::hc::Endpoint;
use crate::transport::{LocalTransport, Transport};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type Hook = Box<dyn FnMut(&mut Core, &Value) -> Act + Send>;

/// What the fake does with a call after a hook looked at it.
#[derive(PartialEq)]
enum Act {
    /// Handle it and answer.
    Answer,
    /// Handle it but close without an answer (a lost reply).
    Lose,
    /// Close without handling it.
    Drop,
}

#[derive(Default)]
struct Core {
    doc: Value,
    version: u64,
    terminals: BTreeMap<String, Value>,
    next: u64,
    methods: Vec<String>,
    /// Hooks by method, run before it is handled.
    hooks: BTreeMap<String, Hook>,
    /// Calls seen: (method, params).
    calls: Vec<(String, Value)>,
    /// A kill answers with the terminal still live (it ends only on the
    /// next look).
    slow_kill: bool,
    /// Labels of the run terminal when a doc.set came in.
    labels_at_set: Vec<Value>,
}

impl Core {
    fn add(&mut self, address: &str, extra: Value) {
        // A started program has a process; tests take it away to model a
        // start herdr-core has not confirmed yet.
        let mut t = json!({"address": address, "state": "live", "labels": {}, "pid": 4242});
        if let (Some(t), Some(extra)) = (t.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                t.insert(k.clone(), v.clone());
            }
        }
        self.terminals.insert(address.to_owned(), t);
    }

    fn live(&self, address: &str) -> bool {
        self.terminals
            .get(address)
            .is_some_and(|t| t["state"] == "live")
    }

    fn finish(&mut self, address: &str) {
        if let Some(t) = self.terminals.get_mut(address) {
            t["state"] = json!("finished");
            t["reason"] = json!("killed");
        }
        // herdr-core ends children marked die_with_parent too.
        let children: Vec<String> = self
            .terminals
            .values()
            .filter(|t| {
                t["parent"] == address && t["die_with_parent"] == true && t["state"] == "live"
            })
            .map(|t| t["address"].as_str().unwrap().to_owned())
            .collect();
        for c in children {
            self.finish(&c);
        }
    }

    fn handle(&mut self, method: &str, p: &Value) -> Result<Value, (&'static str, String)> {
        match method {
            "hello" => Ok(json!({"version":"0.1.20","generation":1,"methods": self.methods})),
            "doc.get" => Ok(json!({"value": self.doc, "version": self.version})),
            "doc.set" => {
                let tab_terminals: Vec<Value> = self
                    .terminals
                    .values()
                    .map(|t| t["labels"].clone())
                    .collect();
                self.labels_at_set = tab_terminals;
                if p["version"].as_u64() != Some(self.version) {
                    return Err(("conflict", "version moved".into()));
                }
                self.doc = p["value"].clone();
                self.version += 1;
                Ok(json!({"version": self.version}))
            }
            "terminal.run" => {
                self.next += 1;
                let address = format!("t{}", self.next + 100);
                let cwd = p["cwd"].as_str().unwrap_or("/home/u").to_owned();
                self.add(
                    &address,
                    json!({"labels": p["labels"], "cwd": cwd,
                        "argv": p.get("argv").cloned().unwrap_or(json!([]))}),
                );
                Ok(self.terminals[&address].clone())
            }
            "terminal.list" => {
                let all = p["all"] == true;
                let want = p["labels"].as_object().cloned().unwrap_or_default();
                let list: Vec<Value> = self
                    .terminals
                    .values()
                    .filter(|t| all || t["state"] == "live")
                    .filter(|t| want.iter().all(|(k, v)| &t["labels"][k] == v))
                    .cloned()
                    .collect();
                Ok(json!({"terminals": list}))
            }
            "terminal.get" => {
                let a = p["terminal"].as_str().unwrap_or("");
                if self.slow_kill && self.terminals.get(a).is_some_and(|t| t["killing"] == true) {
                    self.finish(a);
                }
                self.terminals
                    .get(a)
                    .cloned()
                    .ok_or(("not_found", format!("{a} was removed")))
            }
            "terminal.kill" => {
                let a = p["terminal"].as_str().unwrap_or("").to_owned();
                if !self.terminals.contains_key(&a) {
                    return Err(("not_found", format!("{a} was removed")));
                }
                if self.slow_kill {
                    self.terminals.get_mut(&a).unwrap()["killing"] = json!(true);
                } else {
                    self.finish(&a);
                }
                Ok(self.terminals[&a].clone())
            }
            "terminal.label" => {
                let a = p["terminal"].as_str().unwrap_or("").to_owned();
                let Some(t) = self.terminals.get_mut(&a) else {
                    return Err(("not_found", format!("{a} was removed")));
                };
                for (k, v) in p["labels"].as_object().unwrap() {
                    if v == "" {
                        t["labels"].as_object_mut().unwrap().remove(k);
                    } else {
                        t["labels"][k] = v.clone();
                    }
                }
                Ok(t.clone())
            }
            "terminal.send" => Ok(json!({})),
            other => Err(("unknown_method", other.to_owned())),
        }
    }
}

struct Fake {
    core: Arc<Mutex<Core>>,
    client: Arc<CoreClient>,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static N: AtomicU64 = AtomicU64::new(0);

async fn fake(setup: impl FnOnce(&mut Core)) -> Fake {
    let mut core = Core {
        methods: [
            "hello",
            "events.subscribe",
            "terminal.get",
            "terminal.list",
            "doc.get",
            "doc.set",
            "terminal.run",
            "terminal.kill",
            "terminal.label",
            "terminal.send",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        doc: Value::Null,
        ..Core::default()
    };
    setup(&mut core);
    let core = Arc::new(Mutex::new(core));
    let dir = std::env::temp_dir().join(format!(
        "mc-test-{}-write-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let serving = core.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let core = serving.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                let Ok(Some(line)) = lines.next_line().await else {
                    return;
                };
                let req: Value = serde_json::from_str(&line).unwrap();
                let method = req["method"].as_str().unwrap().to_owned();
                let params = req["params"].clone();
                let (act, result) = {
                    let mut c = core.lock().unwrap();
                    c.calls.push((method.clone(), params.clone()));
                    let act = match c.hooks.remove(&method) {
                        Some(mut hook) => {
                            let act = hook(&mut c, &params);
                            c.hooks.entry(method.clone()).or_insert(hook);
                            act
                        }
                        None => Act::Answer,
                    };
                    if act == Act::Drop {
                        return;
                    }
                    (act, c.handle(&method, &params))
                };
                if act == Act::Lose {
                    return;
                }
                let reply = match result {
                    Ok(r) => json!({"jsonrpc":"2.0","id":req["id"],"result":r}),
                    Err((kind, msg)) => json!({"jsonrpc":"2.0","id":req["id"],
                        "error":{"code":-32000,"message":msg,"data":{"kind":kind}}}),
                };
                let mut out = serde_json::to_vec(&reply).unwrap();
                out.push(b'\n');
                let _ = write.write_all(&out).await;
            });
        }
    });
    let (client, _) = CoreClient::connect_to(
        Arc::new(Transport::Local(LocalTransport)),
        Endpoint::Socket(socket.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    Fake {
        core,
        client: Arc::new(client),
        _dir: TempDir(dir),
    }
}

/// A state with changes allowed that never writes backup files (tests must
/// not touch the home folder): the backup always succeeds.
fn state() -> LayoutState {
    state_backing_up(Arc::new(|| Ok(())))
}

fn state_backing_up(hook: BackupHook) -> LayoutState {
    LayoutState {
        timing: Timing {
            look_again: [Duration::from_millis(150), Duration::from_millis(500)],
            unsettled_for: Duration::from_secs(2),
        },
        backup_hook: Some(hook),
        ..LayoutState::new(true)
    }
}

fn shell_in(space: Option<&str>) -> NewTab {
    NewTab {
        space: space.map(str::to_owned),
        space_name: "web".into(),
        tab_name: "Shell".into(),
        cwd: "/tmp/w".into(),
        start: Start::Shell,
        size: (60, 20),
    }
}

impl Fake {
    fn doc(&self) -> Value {
        self.core.lock().unwrap().doc.clone()
    }
    fn calls(&self, method: &str) -> usize {
        self.core
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }
    fn terminal(&self, a: &str) -> Value {
        self.core.lock().unwrap().terminals[a].clone()
    }
    fn hook(&self, method: &str, hook: impl FnMut(&mut Core, &Value) -> Act + Send + 'static) {
        self.core
            .lock()
            .unwrap()
            .hooks
            .insert(method.into(), Box::new(hook));
    }
}

fn problem(r: Result<impl std::fmt::Debug>) -> LayoutProblem {
    match r {
        Err(Error::Layout { problem, .. }) => problem,
        other => panic!("expected a layout problem, got {other:?}"),
    }
}

#[tokio::test]
async fn a_new_workspace_is_placed_before_it_is_labelled() {
    let f = fake(|_| {}).await;
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    let doc = f.doc();
    assert_eq!(check_writable_ok(&doc), Ok(()));
    assert_eq!(doc["spaces"][0]["id"], made.space.as_str());
    assert_eq!(doc["spaces"][0]["name"], "web");
    assert_eq!(doc["spaces"][0]["folder"], "/tmp/w");
    assert_eq!(doc["spaces"][0]["tabs"][0]["name"], "Shell");
    assert_eq!(
        layout::placement_of(&doc, &made.address),
        Some((made.space.clone(), made.tab.clone()))
    );
    let t = f.terminal(&made.address);
    assert_eq!(t["labels"][SPACE_LABEL], made.space.as_str());
    assert_eq!(t["labels"][TAB_LABEL], made.tab.as_str());
    // While the document was written, nothing carried a space label yet:
    // a desktop could not adopt it as a nameless space.
    let at_set = f.core.lock().unwrap().labels_at_set.clone();
    assert!(at_set.iter().all(|l| l.get(SPACE_LABEL).is_none()));
    // The new terminal had the phone's size from the start.
    let run = f
        .core
        .lock()
        .unwrap()
        .calls
        .iter()
        .find(|(m, _)| m == "terminal.run")
        .unwrap()
        .1
        .clone();
    assert_eq!(run["size"], json!({"cols":60,"rows":20}));
    assert_eq!(run["keep"], true);
    assert!(run.get("argv").is_none());
}

fn check_writable_ok(v: &Value) -> std::result::Result<(), String> {
    layout::check_writable(v)
}

#[tokio::test]
async fn a_desktop_write_in_between_is_kept() {
    let f = fake(|c| {
        c.doc = json!({"spaces":[{"id":"sOld","name":"old","folder":"/","tabs":[{"id":"tO","name":"o"}]}]});
    })
    .await;
    let mut once = true;
    f.hook("doc.set", move |c, _| {
        if std::mem::take(&mut once) {
            // The desktop writes first: our version is stale.
            c.doc["spaces"][0]["name"] = json!("renamed on the desktop");
            c.version += 1;
        }
        Act::Answer
    });
    let made = create(&f.client, &state(), shell_in(Some("sOld")))
        .await
        .unwrap();
    let doc = f.doc();
    assert_eq!(doc["spaces"][0]["name"], "renamed on the desktop");
    assert_eq!(
        layout::tab_ids(&doc, "sOld").unwrap(),
        vec!["tO".to_string(), made.tab]
    );
    assert_eq!(f.calls("doc.set"), 2);
}

#[tokio::test]
async fn a_layout_that_keeps_changing_gives_up_and_ends_the_terminal() {
    let f = fake(|_| {}).await;
    f.hook("doc.set", |c, _| {
        c.version += 1;
        Act::Answer
    });
    assert_eq!(
        problem(create(&f.client, &state(), shell_in(None)).await),
        LayoutProblem::Conflict
    );
    let c = f.core.lock().unwrap();
    assert!(c.terminals.values().all(|t| t["state"] == "finished"));
}

#[tokio::test]
async fn an_adoption_that_came_first_keeps_its_place_and_gets_names() {
    let f = fake(|c| {
        c.doc = json!({"spaces":[{"id":"sOld","name":"old","folder":"/","tabs":[{"id":"tO"}]}]});
    })
    .await;
    // Right after the start, a desktop puts the terminal into a nameless
    // space under the tab's id (as if it had seen a space label), before
    // another space.
    f.hook("terminal.run", |c, p| {
        let tab = p["labels"][TAB_LABEL].as_str().unwrap().to_owned();
        let address = format!("t{}", c.next + 101);
        let adopted = json!({"id":"spDesk","name":"","folder":"","tabs":[{"id":tab,"name":"",
            "root":{"id":tab,"type":"row","children":[{"id":"pDesk","type":"block",
            "content":{"terminal":address}}]}}]});
        c.doc["spaces"].as_array_mut().unwrap().insert(0, adopted);
        c.version += 1;
        Act::Answer
    });
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    let doc = f.doc();
    // Its pane, its place, one pane only; our names filled in.
    assert_eq!(layout::space_ids(&doc), ["spDesk", "sOld"]);
    assert_eq!(doc["spaces"][0]["tabs"][0]["name"], "Shell");
    assert_eq!(layout::tab_image(&doc, &made.tab).unwrap().nodes.len(), 2);
    assert_eq!(
        layout::placement_of(&doc, &made.address).unwrap().1,
        made.tab
    );
}

#[tokio::test]
async fn a_lost_start_answer_is_found_by_its_tab() {
    let f = fake(|_| {}).await;
    f.hook("terminal.run", |_, _| Act::Lose);
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert_eq!(f.calls("terminal.run"), 1, "never started twice");
    assert!(layout::placement_of(&f.doc(), &made.address).is_some());
}

#[tokio::test]
async fn a_start_never_seen_stays_open_and_a_late_one_is_ended() {
    let f = fake(|_| {}).await;
    let s = state();
    f.hook("terminal.run", |_, _| Act::Drop);
    assert_eq!(
        problem(create(&f.client, &s, shell_in(None)).await),
        LayoutProblem::Unclear
    );
    // Not settled yet: no second start at the same place.
    assert_eq!(
        problem(create(&f.client, &s, shell_in(None)).await),
        LayoutProblem::Busy
    );
    assert_eq!(f.calls("terminal.run"), 1);
    // The first start shows up after all (it arrived late) ...
    let tab = s.unsettled(NEW_SPACE).unwrap().tab;
    f.core
        .lock()
        .unwrap()
        .add("t99", json!({"labels": {TAB_LABEL: tab}}));
    // ... and the later look ends it: the user was told it failed.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!f.core.lock().unwrap().live("t99"));
    // Seen and ended: the place is settled and a new start may go.
    f.core.lock().unwrap().hooks.clear();
    create(&f.client, &s, shell_in(None)).await.unwrap();
    assert!(s.unsettled(NEW_SPACE).is_none());
}

#[tokio::test]
async fn a_start_arriving_after_the_last_look_still_blocks_and_is_ended_on_the_next_try() {
    let f = fake(|_| {}).await;
    let s = state();
    f.hook("terminal.run", |_, _| Act::Drop);
    problem(create(&f.client, &s, shell_in(None)).await);
    // After both looks (nothing seen), but before it counts as never made.
    tokio::time::sleep(Duration::from_millis(700)).await;
    f.core.lock().unwrap().hooks.clear();
    assert_eq!(
        problem(create(&f.client, &s, shell_in(None)).await),
        LayoutProblem::Busy
    );
    let tab = s.unsettled(NEW_SPACE).unwrap().tab;
    f.core
        .lock()
        .unwrap()
        .add("t98", json!({"labels": {TAB_LABEL: tab}}));
    // The next try finds it, ends it, and only then starts.
    create(&f.client, &s, shell_in(None)).await.unwrap();
    assert!(!f.core.lock().unwrap().live("t98"));
}

#[tokio::test]
async fn a_start_never_seen_counts_as_never_made_after_a_while() {
    let f = fake(|_| {}).await;
    let s = state();
    f.hook("terminal.run", |_, _| Act::Drop);
    problem(create(&f.client, &s, shell_in(None)).await);
    f.core.lock().unwrap().hooks.clear();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    create(&f.client, &s, shell_in(None)).await.unwrap();
}

#[tokio::test]
async fn a_failed_start_that_left_a_finished_record_places_nothing() {
    let f = fake(|_| {}).await;
    f.hook("terminal.run", |c, p| {
        c.add(
            "t50",
            json!({"labels": p["labels"], "state": "finished", "reason": "spawn_failed"}),
        );
        Act::Drop
    });
    let got = create(&f.client, &state(), shell_in(None)).await;
    assert!(
        matches!(got, Err(Error::Herdr { ref code, .. }) if code == "spawn_failed"),
        "{got:?}"
    );
    assert!(f.doc().is_null());
    assert_eq!(f.calls("doc.set"), 0);
}

#[tokio::test]
async fn a_lost_write_answer_is_checked_by_reading_back() {
    let f = fake(|_| {}).await;
    f.hook("doc.set", |_, _| Act::Lose);
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert!(layout::placement_of(&f.doc(), &made.address).is_some());
    assert_eq!(f.calls("doc.set"), 1, "it was in: not written again");
    assert!(f.core.lock().unwrap().live(&made.address));
}

#[tokio::test]
async fn a_workspace_deleted_meanwhile_is_not_brought_back() {
    let f = fake(|c| {
        c.doc = json!({"spaces":[{"id":"sA","name":"a","folder":"/","tabs":[{"id":"tA"}]}]});
    })
    .await;
    f.hook("terminal.run", |c, _| {
        c.doc = json!({"spaces":[]});
        c.version += 1;
        Act::Answer
    });
    assert_eq!(
        problem(create(&f.client, &state(), shell_in(Some("sA"))).await),
        LayoutProblem::Gone
    );
    assert_eq!(f.doc(), json!({"spaces":[]}));
    let c = f.core.lock().unwrap();
    assert!(c.terminals.values().all(|t| t["state"] == "finished"));
}

#[tokio::test]
async fn a_parked_terminal_is_not_labelled_back() {
    let f = fake(|_| {}).await;
    // Right after our write the desktop parks it: labels cleared, pane gone.
    let mut once = true;
    f.hook("terminal.get", move |c, p| {
        if std::mem::take(&mut once) {
            let a = p["terminal"].as_str().unwrap().to_owned();
            c.terminals.get_mut(&a).unwrap()["labels"] = json!({});
        }
        Act::Answer
    });
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert!(
        f.terminal(&made.address)["labels"]
            .get(SPACE_LABEL)
            .is_none()
    );
    assert_eq!(f.calls("terminal.label"), 0);
}

#[tokio::test]
async fn nothing_starts_without_the_methods() {
    let f = fake(|c| c.methods.retain(|m| m != "terminal.label")).await;
    let got = create(&f.client, &state(), shell_in(None)).await;
    assert!(matches!(got, Err(Error::Unsupported { .. })), "{got:?}");
    assert_eq!(f.calls("terminal.run"), 0);
}

#[tokio::test]
async fn an_unreadable_layout_is_never_written() {
    let f = fake(|c| {
        c.doc = json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"future"}}]}]})
    })
    .await;
    assert_eq!(
        problem(create(&f.client, &state(), shell_in(Some("s"))).await),
        LayoutProblem::Unreadable
    );
    assert_eq!(f.calls("terminal.run"), 0);
    assert_eq!(
        problem(rename_tab(&f.client, &state(), "t", "x").await),
        LayoutProblem::Unreadable
    );
    assert_eq!(f.calls("doc.set"), 0);
}

#[tokio::test]
async fn an_agent_starts_in_a_login_shell_that_stays() {
    let f = fake(|_| {}).await;
    let mut new = shell_in(None);
    new.start = Start::Program(vec!["claude".into(), "--flag with space".into()]);
    create(&f.client, &state(), new).await.unwrap();
    let c = f.core.lock().unwrap();
    let run = &c.calls.iter().find(|(m, _)| m == "terminal.run").unwrap().1;
    let argv: Vec<String> = serde_json::from_value(run["argv"].clone()).unwrap();
    assert_eq!(&argv[..2], ["sh", "-c"]);
    assert_eq!(&argv[3..], ["sh", "claude", "--flag with space"]);
}

#[tokio::test]
async fn a_command_is_typed_into_the_new_shell() {
    let f = fake(|_| {}).await;
    let mut new = shell_in(None);
    new.start = Start::Line("make test".into());
    let made = create(&f.client, &state(), new).await.unwrap();
    let c = f.core.lock().unwrap();
    let send = &c
        .calls
        .iter()
        .find(|(m, _)| m == "terminal.send")
        .unwrap()
        .1;
    assert_eq!(send["terminal"], made.address.as_str());
    assert_eq!(send["text"], "make test");
}

#[tokio::test]
async fn renaming_and_moving() {
    let f = fake(|c| {
        c.doc =
            json!({"spaces":[{"id":"a","tabs":[{"id":"a1"},{"id":"a2"}]},{"id":"b"},{"id":"c"}]});
    })
    .await;
    let s = state();
    rename_space(&f.client, &s, "b", "Beta").await.unwrap();
    rename_tab(&f.client, &s, "a2", "zwei").await.unwrap();
    move_space(&f.client, &s, "a", &Anchor::Before("c".into()))
        .await
        .unwrap();
    move_tab(&f.client, &s, "a2", &Anchor::Before("a1".into()))
        .await
        .unwrap();
    let doc = f.doc();
    assert_eq!(layout::space_ids(&doc), ["b", "a", "c"]);
    assert_eq!(doc["spaces"][0]["name"], "Beta");
    assert_eq!(layout::tab_ids(&doc, "a").unwrap(), ["a2", "a1"]);
    let before = f.calls("doc.set");
    // The same again changes nothing and writes nothing.
    rename_space(&f.client, &s, "b", "Beta").await.unwrap();
    assert_eq!(f.calls("doc.set"), before);
    assert_eq!(
        problem(rename_space(&f.client, &s, "gone", "x").await),
        LayoutProblem::Gone
    );
}

fn two_tabs() -> Value {
    json!({"spaces":[{"id":"s","name":"s","folder":"/","tabs":[
        {"id":"tA","root":{"id":"tA","type":"row","children":[
            {"id":"p1","type":"block","content":{"terminal":"t1"}},
            {"id":"p9","type":"block","content":{"terminal":"t9"}}]}},
        {"id":"tB","root":{"id":"tB","type":"row","children":[
            {"id":"p2","type":"block","content":{"terminal":"t2"}}]}}]}]})
}

#[tokio::test]
async fn closing_a_tab_ends_its_terminals_and_children_then_removes_it() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({"labels":{"herdr.space":"s","herdr.tab":"tA"}}));
        c.add("t2", json!({"labels":{"herdr.space":"s","herdr.tab":"tB"}}));
        // A sub-agent of t1 without a pane, and one only labelled for tA.
        c.add("t3", json!({"parent":"t1"}));
        c.add("t4", json!({"labels":{"herdr.space":"s","herdr.tab":"tA"}}));
        c.slow_kill = true;
        // t9 is in the layout but long gone (not_found).
    })
    .await;
    close(&f.client, &state(), Target::Tab("tA")).await.unwrap();
    let c = f.core.lock().unwrap();
    for gone in ["t1", "t3", "t4"] {
        assert!(!c.live(gone), "{gone}");
    }
    assert!(c.live("t2"));
    assert_eq!(layout::tab_ids(&c.doc, "s").unwrap(), ["tB"]);
}

#[tokio::test]
async fn closing_the_last_tab_or_a_gone_one() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t2", json!({}));
    })
    .await;
    let s = state();
    close(&f.client, &s, Target::Space("s")).await.unwrap();
    assert_eq!(f.doc(), json!({"spaces":[]}));
    // Already gone: closed.
    close(&f.client, &s, Target::Tab("tA")).await.unwrap();
}

#[tokio::test]
async fn content_added_while_closing_stays() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
    })
    .await;
    let mut reads = 0;
    f.hook("doc.get", move |c, _| {
        reads += 1;
        if reads == 2 {
            // The desktop added a notes pane to tA meanwhile.
            c.doc["spaces"][0]["tabs"][0]["root"]["children"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"pMod","type":"block","content":{"mod":"notes"}}));
            c.version += 1;
        }
        Act::Answer
    });
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::Partial
    );
    assert_eq!(layout::tab_ids(&f.doc(), "s").unwrap(), ["tA", "tB"]);
}

#[tokio::test]
async fn closing_refuses_when_a_child_elsewhere_would_end_too() {
    for (child, why) in [
        // A child in another tab of the same space.
        (json!({"parent":"t1","die_with_parent":true}), "t2"),
    ] {
        let f = fake(|c| {
            c.doc = two_tabs();
            c.add("t1", json!({}));
            c.add("t2", child.clone());
        })
        .await;
        assert_eq!(
            problem(close(&f.client, &state(), Target::Tab("tA")).await),
            LayoutProblem::ChildElsewhere,
            "{why}"
        );
        assert!(f.core.lock().unwrap().live("t1"), "nothing ended");
    }
    // A grandchild shown in tab B under a child without a pane or label.
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
        c.add("t5", json!({"parent":"t1","die_with_parent":true}));
        c.add("t2", json!({"parent":"t5","die_with_parent":true}));
    })
    .await;
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::ChildElsewhere
    );
    // The same child kept alive by `outlive_parent` is no reason to refuse.
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
        c.add("t2", json!({"parent":"t1","die_with_parent":false}));
    })
    .await;
    close(&f.client, &state(), Target::Tab("tA")).await.unwrap();
    assert!(f.core.lock().unwrap().live("t2"));
}

#[tokio::test]
async fn closing_refuses_other_machines() {
    let f = fake(|c| {
        let mut doc = two_tabs();
        doc["spaces"][0]["tabs"][0]["root"]["children"][1]["content"]["terminal"] = json!("box/t7");
        c.doc = doc;
        c.add("t1", json!({}));
    })
    .await;
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::OtherMachine
    );
    assert!(f.core.lock().unwrap().live("t1"));
}

#[tokio::test]
async fn a_loose_terminal_closes_alone() {
    let f = fake(|c| c.add("t7", json!({}))).await;
    close_terminal(&f.client, &state(), "t7").await.unwrap();
    assert!(!f.core.lock().unwrap().live("t7"));
    // Already gone counts as closed.
    close_terminal(&f.client, &state(), "t404").await.unwrap();
}

#[tokio::test]
async fn two_actions_at_once_take_turns() {
    let f = fake(|_| {}).await;
    let s = Arc::new(state());
    let (a, b) = tokio::join!(
        create(&f.client, &s, shell_in(None)),
        create(&f.client, &s, shell_in(None))
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(layout::space_ids(&f.doc()).len(), 2);
    assert_eq!(f.calls("doc.set"), 2, "no conflict between our own writes");
}

#[test]
fn the_launcher_runs_the_command_and_then_a_login_shell() {
    let dir = std::env::temp_dir().join(format!("mc-test-{}-launch", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A stand-in `bash` that runs the script it is given, then reports how
    // it was started when the script execs it as the shell that stays.
    let fake_bash = dir.join("bash");
    std::fs::write(
        &fake_bash,
        "#!/bin/sh\nif [ \"$1 $2 $3\" = \"-l -i -c\" ]; then s=$4; shift 4; exec /bin/sh -c \"$s\" \"$@\"; fi\necho \"shell stays: $*\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake_bash, std::fs::Permissions::from_mode(0o755)).unwrap();
    let argv = login_shell_argv(&["printf".into(), "%s|".into(), "a b".into(), "$HOME".into()]);
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env("SHELL", &fake_bash)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text, "a b|$HOME|shell stays: -l -i\n", "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn one_space() -> Value {
    json!({"spaces":[{"id":"s","name":"s","folder":"/","tabs":[
        {"id":"tA","root":{"id":"tA","type":"row","children":[
            {"id":"p1","type":"block","content":{"terminal":"t1"}}]}}]}]})
}

#[tokio::test]
async fn nothing_changes_without_layout_editing() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
        c.add("t7", json!({}));
    })
    .await;
    let off = LayoutState::new(false);
    let refused = |r: Result<()>| assert!(matches!(r, Err(Error::Unsupported { .. })), "{r:?}");
    refused(
        create(&f.client, &off, shell_in(Some("s")))
            .await
            .map(|_| ()),
    );
    refused(rename_tab(&f.client, &off, "tA", "x").await);
    refused(rename_space(&f.client, &off, "s", "x").await);
    refused(move_tab(&f.client, &off, "tB", &Anchor::Before("tA".into())).await);
    refused(move_space(&f.client, &off, "s", &Anchor::End).await);
    refused(close(&f.client, &off, Target::Tab("tA")).await);
    refused(close_terminal(&f.client, &off, "t7").await);
    for m in ["terminal.run", "doc.set", "terminal.kill", "doc.get"] {
        assert_eq!(f.calls(m), 0, "{m}");
    }
}

#[tokio::test]
async fn the_backup_comes_before_any_change() {
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
    })
    .await;
    let core = f.core.clone();
    let s = state_backing_up(Arc::new(move || {
        let c = core.lock().unwrap();
        for m in ["terminal.run", "doc.set", "terminal.kill"] {
            assert!(c.calls.iter().all(|(n, _)| n != m), "{m} before the backup");
        }
        Ok(())
    }));
    create(&f.client, &s, shell_in(Some("s"))).await.unwrap();
    assert!(s.backed_up.load(Ordering::SeqCst));
    let s = state_backing_up(Arc::new({
        let core = f.core.clone();
        move || {
            let c = core.lock().unwrap();
            assert!(c.calls.iter().all(|(n, _)| n != "terminal.kill"));
            Ok(())
        }
    }));
    // Fresh state: closing backs up before it ends anything.
    f.core.lock().unwrap().calls.clear();
    close(&f.client, &s, Target::Tab("tA")).await.unwrap();
}

#[tokio::test]
async fn a_failed_backup_changes_nothing() {
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
    })
    .await;
    let s = state_backing_up(Arc::new(|| Err(Error::Protocol("disk full".into()))));
    assert_eq!(
        problem(create(&f.client, &s, shell_in(Some("s"))).await),
        LayoutProblem::Backup
    );
    assert_eq!(
        problem(rename_tab(&f.client, &s, "tA", "x").await),
        LayoutProblem::Backup
    );
    assert_eq!(
        problem(close(&f.client, &s, Target::Tab("tA")).await),
        LayoutProblem::Backup
    );
    for m in ["terminal.run", "doc.set", "terminal.kill"] {
        assert_eq!(f.calls(m), 0, "{m}");
    }
    assert!(f.core.lock().unwrap().live("t1"));
}

#[tokio::test]
async fn an_unsettled_start_stays_until_it_surely_ended() {
    let f = fake(|_| {}).await;
    let s = state();
    f.hook("terminal.run", |_, _| Act::Drop);
    problem(create(&f.client, &s, shell_in(None)).await);
    let tab = s.unsettled(NEW_SPACE).unwrap().tab;
    // It arrives, but cannot be ended for now.
    f.core
        .lock()
        .unwrap()
        .add("t99", json!({"labels": {TAB_LABEL: tab}}));
    f.hook("terminal.kill", |_, _| Act::Drop);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        problem(create(&f.client, &s, shell_in(None)).await),
        LayoutProblem::Busy
    );
    assert!(f.core.lock().unwrap().live("t99"));
    assert!(s.unsettled(NEW_SPACE).is_some());
    assert_eq!(f.calls("terminal.run"), 1, "no second start meanwhile");
    // Once it can be ended, it is, and only then the next start goes.
    f.core.lock().unwrap().hooks.clear();
    create(&f.client, &s, shell_in(None)).await.unwrap();
    assert!(!f.core.lock().unwrap().live("t99"));
}

#[tokio::test]
async fn late_looks_use_the_hosts_current_client() {
    let f = fake(|_| {}).await;
    let g = fake(|_| {}).await;
    let slot: ClientSlot = Arc::new(RwLock::new(Some(f.client.clone())));
    let s = state();
    s.attach(slot.clone());
    f.hook("terminal.run", |_, _| Act::Drop);
    problem(create(&f.client, &s, shell_in(None)).await);
    let tab = s.unsettled(NEW_SPACE).unwrap().tab;
    // The host reconnected: the late start shows up on the new client.
    *slot.write().unwrap() = Some(g.client.clone());
    g.core
        .lock()
        .unwrap()
        .add("t97", json!({"labels": {TAB_LABEL: tab}}));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!g.core.lock().unwrap().live("t97"));
    assert!(s.unsettled(NEW_SPACE).is_none(), "ended for sure: settled");
}

#[tokio::test]
async fn a_start_that_never_went_out_blocks_nothing() {
    let f = fake(|_| {}).await;
    let socket = f._dir.0.join("core.sock");
    f.hook("doc.get", move |_, _| {
        // From now on nothing reaches the server.
        let _ = std::fs::remove_file(&socket);
        Act::Answer
    });
    let s = state();
    let got = create(&f.client, &s, shell_in(None)).await;
    assert!(
        !matches!(
            got,
            Err(Error::Layout {
                problem: LayoutProblem::Unclear,
                ..
            })
        ),
        "{got:?}"
    );
    assert!(got.is_err());
    assert!(s.unsettled(NEW_SPACE).is_none());
    assert_eq!(f.calls("terminal.run"), 0);
}

#[tokio::test]
async fn a_start_herdr_core_keeps_after_a_spawn_timeout_is_ended() {
    let f = fake(|_| {}).await;
    f.hook("terminal.run", |c, p| {
        c.add(
            "t60",
            json!({"labels": p["labels"], "reason": "spawn_pending"}),
        );
        Act::Drop
    });
    let s = state();
    let got = create(&f.client, &s, shell_in(None)).await;
    assert!(
        matches!(got, Err(Error::Herdr { ref code, .. }) if code == "spawn_failed"),
        "{got:?}"
    );
    assert!(!f.core.lock().unwrap().live("t60"));
    assert_eq!(f.calls("doc.set"), 0);
    assert!(s.unsettled(NEW_SPACE).is_none());
}

#[tokio::test]
async fn an_unreadable_read_back_keeps_the_terminal() {
    let f = fake(|_| {}).await;
    f.hook("doc.set", |_, _| Act::Lose);
    f.hook("doc.get", |c, _| {
        // After our lost write, another writer leaves a shape we cannot
        // check: whether ours is in stays unknown.
        if c.calls.iter().any(|(m, _)| m == "doc.set") && c.doc.is_object() {
            c.doc["spaces"][0]["tabs"][0]["root"]["window_drag"] = json!("yes");
        }
        Act::Answer
    });
    // Our write is in (the layout holds the terminal): it counts as made,
    // and nothing is ended.
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert!(layout::placement_of(&f.doc(), &made.address).is_some());
    let c = f.core.lock().unwrap();
    assert!(c.terminals.values().all(|t| t["state"] == "live"));
    assert_eq!(c.terminals.len(), 1);
}

#[tokio::test]
async fn an_unreadable_read_back_without_our_pane_keeps_the_terminal() {
    let f = fake(|_| {}).await;
    // The write did not answer and its outcome cannot be read: unclear,
    // and the terminal stays (it may be in).
    f.hook("doc.set", |_, _| Act::Drop);
    f.hook("doc.get", |c, _| {
        if c.calls.iter().any(|(m, _)| m == "doc.set") {
            c.doc = json!({"spaces": [{"id": "sx", "tabs": [{"id": "tx", "root": {"type": "future"}}]}]});
        }
        Act::Answer
    });
    let got = create(&f.client, &state(), shell_in(None)).await;
    assert_eq!(problem(got), LayoutProblem::Unclear);
    let c = f.core.lock().unwrap();
    assert!(c.terminals.values().all(|t| t["state"] == "live"));
    assert_eq!(c.terminals.len(), 1);
}

#[tokio::test]
async fn a_write_that_never_arrived_is_made_again() {
    let f = fake(|_| {}).await;
    let mut once = true;
    f.hook("doc.set", move |_, _| {
        if std::mem::take(&mut once) {
            Act::Drop
        } else {
            Act::Answer
        }
    });
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    let doc = f.doc();
    assert_eq!(layout::space_ids(&doc).len(), 1);
    assert!(layout::placement_of(&doc, &made.address).is_some());
    assert!(f.core.lock().unwrap().live(&made.address));
}

#[tokio::test]
async fn a_terminal_that_will_not_end_leaves_the_tab() {
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
    })
    .await;
    f.hook("terminal.kill", |_, _| Act::Drop);
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::Partial
    );
    assert_eq!(f.doc(), one_space());
    assert_eq!(f.calls("doc.set"), 0);
}

#[tokio::test]
async fn a_terminal_joining_while_closing_keeps_the_tab() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({"labels":{"herdr.space":"s","herdr.tab":"tA"}}));
    })
    .await;
    let mut reads = 0;
    f.hook("doc.get", move |c, _| {
        reads += 1;
        if reads == 2 {
            // Started elsewhere for tA, not in a pane yet.
            c.add("t8", json!({"labels":{"herdr.space":"s","herdr.tab":"tA"}}));
        }
        Act::Answer
    });
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::Partial
    );
    assert_eq!(layout::tab_ids(&f.doc(), "s").unwrap(), ["tA", "tB"]);
    let c = f.core.lock().unwrap();
    assert!(!c.live("t1") && c.live("t8"));
}

#[tokio::test]
async fn an_empty_tab_added_while_closing_keeps_the_workspace() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
    })
    .await;
    let mut reads = 0;
    f.hook("doc.get", move |c, _| {
        reads += 1;
        if reads == 2 {
            c.doc["spaces"][0]["tabs"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"tNew"}));
            c.version += 1;
        }
        Act::Answer
    });
    assert_eq!(
        problem(close(&f.client, &state(), Target::Space("s")).await),
        LayoutProblem::Partial
    );
    assert_eq!(layout::space_ids(&f.doc()), ["s"]);
}

#[tokio::test]
async fn a_loose_terminal_is_checked_before_it_ends() {
    // A child that herdr-core would end with it, shown in a workspace.
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t7", json!({}));
        c.add("t2", json!({"parent":"t7","die_with_parent":true}));
    })
    .await;
    assert_eq!(
        problem(close_terminal(&f.client, &state(), "t7").await),
        LayoutProblem::ChildElsewhere
    );
    assert!(f.core.lock().unwrap().live("t7"));
    // Placed in a tab meanwhile: no longer loose.
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
    })
    .await;
    assert_eq!(
        problem(close_terminal(&f.client, &state(), "t1").await),
        LayoutProblem::Placed
    );
    assert!(f.core.lock().unwrap().live("t1"));
    // Claimed by a workspace's label.
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t6", json!({"labels":{"herdr.space":"s"}}));
    })
    .await;
    assert_eq!(
        problem(close_terminal(&f.client, &state(), "t6").await),
        LayoutProblem::Placed
    );
    // Labelled for a workspace without tabs: listed as loose, but the Herdr
    // app may be adopting it, so it stays, with its own reason.
    let f = fake(|c| {
        c.doc = json!({"spaces":[{"id":"e","name":"e","folder":"/","tabs":[]}]});
        c.add("t5", json!({"labels":{"herdr.space":"e"}}));
    })
    .await;
    assert_eq!(
        problem(close_terminal(&f.client, &state(), "t5").await),
        LayoutProblem::Claimed
    );
    assert!(f.core.lock().unwrap().live("t5"));
    // On another machine.
    assert_eq!(
        problem(close_terminal(&f.client, &state(), "box/t1").await),
        LayoutProblem::OtherMachine
    );
    // A loose child without a pane goes with it.
    let f = fake(|c| {
        c.add("t7", json!({}));
        c.add("t8", json!({"parent":"t7"}));
    })
    .await;
    close_terminal(&f.client, &state(), "t7").await.unwrap();
    let c = f.core.lock().unwrap();
    assert!(!c.live("t7") && !c.live("t8"));
}

#[tokio::test]
async fn a_missing_document_keeps_its_version() {
    let f = fake(|c| c.version = 5).await;
    create(&f.client, &state(), shell_in(None)).await.unwrap();
    let c = f.core.lock().unwrap();
    let set = &c.calls.iter().find(|(m, _)| m == "doc.set").unwrap().1;
    assert_eq!(set["version"], 5);
}

#[tokio::test]
async fn a_layout_over_the_size_limit_is_not_written() {
    let big = "x".repeat(layout::MAX_BYTES - 64);
    let f = fake(|c| c.doc = json!({"spaces":[], "future": big})).await;
    let before = f.doc();
    assert_eq!(
        problem(create(&f.client, &state(), shell_in(None)).await),
        LayoutProblem::TooLarge
    );
    assert_eq!(f.calls("doc.set"), 0);
    assert_eq!(f.doc(), before);
    let c = f.core.lock().unwrap();
    assert!(c.terminals.values().all(|t| t["state"] == "finished"));
}

#[tokio::test]
async fn a_failed_label_is_tried_again() {
    let f = fake(|_| {}).await;
    let mut once = true;
    f.hook("terminal.label", move |_, _| {
        if std::mem::take(&mut once) {
            Act::Drop
        } else {
            Act::Answer
        }
    });
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert_eq!(f.calls("terminal.label"), 2);
    assert_eq!(
        f.terminal(&made.address)["labels"][SPACE_LABEL],
        made.space.as_str()
    );
}

#[test]
fn unsettled_starts_survive_a_restart_of_the_command_line() {
    let s = state();
    s.set_unsettled(NEW_SPACE, Some("tbX")).unwrap();
    let kept = serde_json::to_string(&s.unsettled_starts()).unwrap();
    let again = LayoutState::new(true);
    again.restore_unsettled(serde_json::from_str(&kept).unwrap());
    assert_eq!(again.unsettled(NEW_SPACE).unwrap().tab, "tbX");
}

#[tokio::test]
async fn a_placed_terminal_is_kept_when_only_its_name_does_not_get_in() {
    let f = fake(|_| {}).await;
    let mut first = true;
    f.hook("doc.set", move |c, _| {
        if std::mem::take(&mut first) {
            // Our write lands, its answer is lost.
            return Act::Lose;
        }
        // The desktop clears the new tab's name and keeps writing.
        if let Some(tabs) = c.doc["spaces"][0]["tabs"].as_array_mut() {
            tabs[0]["name"] = json!("");
        }
        c.version += 1;
        Act::Answer
    });
    f.hook("doc.get", |c, _| {
        if c.calls.iter().any(|(m, _)| m == "doc.set")
            && let Some(tabs) = c.doc["spaces"][0]["tabs"].as_array_mut()
        {
            tabs[0]["name"] = json!("");
        }
        Act::Answer
    });
    let made = create(&f.client, &state(), shell_in(None)).await.unwrap();
    assert!(f.core.lock().unwrap().live(&made.address));
    assert!(layout::placement_of(&f.doc(), &made.address).is_some());
}

#[tokio::test]
async fn a_start_not_yet_confirmed_is_not_placed() {
    let f = fake(|_| {}).await;
    // The answer is lost; the record is there but its start is not
    // confirmed (no process, no reason).
    f.hook("terminal.run", |_, _| Act::Lose);
    f.hook("terminal.list", |c, _| {
        for t in c.terminals.values_mut() {
            t["pid"] = Value::Null;
        }
        Act::Answer
    });
    let s = state();
    assert_eq!(
        problem(create(&f.client, &s, shell_in(None)).await),
        LayoutProblem::Unclear
    );
    assert!(layout::space_ids(&f.doc()).is_empty(), "nothing placed");
    assert!(s.unsettled(NEW_SPACE).is_some(), "the place stays open");
}

#[tokio::test]
async fn closing_the_last_tab_ends_terminals_shown_in_it_by_their_space() {
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
        // In no pane, labelled for the space only: the app shows it in the
        // space's first tab.
        c.add("t3", json!({"labels": {SPACE_LABEL: "s"}}));
    })
    .await;
    close(&f.client, &state(), Target::Tab("tA")).await.unwrap();
    let c = f.core.lock().unwrap();
    assert!(!c.live("t1"));
    assert!(!c.live("t3"), "shown in the closed tab: ended");
    assert!(layout::space_ids(&c.doc).is_empty());
}

#[tokio::test]
async fn the_only_tab_waits_for_a_terminal_labelled_for_a_tab_not_written_yet() {
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
        // The desktop starts a new tab's terminal before it writes the tab.
        c.add(
            "t2",
            json!({"labels": {SPACE_LABEL: "s", TAB_LABEL: "tNew"}}),
        );
    })
    .await;
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::Unplaced
    );
    let c = f.core.lock().unwrap();
    assert!(c.live("t2"), "not ended");
    assert!(c.live("t1"), "refused before anything ended");
    assert_eq!(layout::space_ids(&c.doc), ["s"]);
}

#[tokio::test]
async fn a_move_whose_anchor_is_gone_changes_nothing() {
    let f = fake(|c| {
        c.doc =
            json!({"spaces":[{"id":"a","name":"a"},{"id":"c","name":"c"},{"id":"d","name":"d"}]});
    })
    .await;
    assert_eq!(
        problem(move_space(&f.client, &state(), "c", &Anchor::Before("b".into())).await),
        LayoutProblem::Conflict
    );
    assert_eq!(layout::space_ids(&f.doc()), ["a", "c", "d"]);
    assert_eq!(f.calls("doc.set"), 0);
}

#[tokio::test]
async fn a_replaced_host_hands_on_its_open_starts() {
    let f = fake(|_| {}).await;
    let g = fake(|_| {}).await;
    let old: ClientSlot = Arc::new(RwLock::new(Some(f.client.clone())));
    let s = Arc::new(state());
    s.attach(old.clone());
    f.hook("terminal.run", |_, _| Act::Drop);
    problem(create(&f.client, &s, shell_in(None)).await);
    let tab = s.unsettled(NEW_SPACE).unwrap().tab;
    // Settings saved: a new host takes this state with its own client slot;
    // the old one is stopped and empties its slot.
    let new: ClientSlot = Arc::new(RwLock::new(Some(g.client.clone())));
    s.attach(new);
    *old.write().unwrap() = None;
    assert_eq!(
        problem(create(&g.client, &s, shell_in(None)).await),
        LayoutProblem::Busy,
        "the open start keeps blocking on the new host"
    );
    g.core
        .lock()
        .unwrap()
        .add("t97", json!({"labels": {TAB_LABEL: tab}}));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !g.core.lock().unwrap().live("t97"),
        "looked for on the new host"
    );
}

#[tokio::test]
async fn a_start_is_on_record_before_it_goes_out() {
    let f = fake(|_| {}).await;
    let kept: Arc<Mutex<Vec<Vec<UnsettledStart>>>> = Arc::default();
    let k = kept.clone();
    let s = state().with_persist(Arc::new(move |starts: &[UnsettledStart]| {
        k.lock().unwrap().push(starts.to_vec());
        Ok(())
    }));
    let seen_at_run: Arc<Mutex<usize>> = Arc::default();
    let (k, seen) = (kept.clone(), seen_at_run.clone());
    f.hook("terminal.run", move |_, _| {
        *seen.lock().unwrap() = k.lock().unwrap().last().map_or(0, Vec::len);
        Act::Answer
    });
    create(&f.client, &s, shell_in(None)).await.unwrap();
    assert_eq!(*seen_at_run.lock().unwrap(), 1, "kept before terminal.run");
    assert!(
        kept.lock().unwrap().last().unwrap().is_empty(),
        "settled after"
    );
}

#[tokio::test]
async fn a_start_that_cannot_be_kept_does_not_go_out() {
    let f = fake(|_| {}).await;
    let s = state().with_persist(Arc::new(|_: &[UnsettledStart]| {
        Err(Error::Protocol("disk full".into()))
    }));
    assert!(create(&f.client, &s, shell_in(None)).await.is_err());
    assert_eq!(f.calls("terminal.run"), 0);
    assert!(
        s.unsettled(NEW_SPACE).is_none(),
        "nothing went out: nothing blocks"
    );
}

#[tokio::test]
async fn a_stale_open_start_never_ends_a_placed_terminal() {
    // The start answered and was placed, but its record was left behind
    // (clearing it failed, or the process ended right after the answer).
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({"labels": {SPACE_LABEL: "s", TAB_LABEL: "tA"}}));
    })
    .await;
    let s = state();
    s.restore_unsettled(vec![UnsettledStart {
        place: "s".into(),
        tab: "tA".into(),
        since_ms: 0,
    }]);
    create(&f.client, &s, shell_in(Some("s"))).await.unwrap();
    assert!(f.core.lock().unwrap().live("t1"), "placed: never ended");
    assert!(s.unsettled("s").is_none(), "the stale record is settled");
}

#[tokio::test]
async fn a_stale_open_start_never_ends_a_terminal_of_a_workspace() {
    // The record's tab id is now carried by a terminal the desktop placed
    // into a workspace (it has `herdr.space`) but has not written yet.
    let f = fake(|c| {
        c.doc = one_space();
        c.add("t1", json!({}));
        c.add("t2", json!({"labels": {SPACE_LABEL: "s", TAB_LABEL: "tX"}}));
    })
    .await;
    let s = state();
    s.restore_unsettled(vec![UnsettledStart {
        place: "s".into(),
        tab: "tX".into(),
        since_ms: 0,
    }]);
    create(&f.client, &s, shell_in(Some("s"))).await.unwrap();
    assert!(
        f.core.lock().unwrap().live("t2"),
        "has a workspace: never ended"
    );
}

#[tokio::test]
async fn a_failure_to_clear_a_settled_start_is_reported() {
    let f = fake(|_| {}).await;
    // Keeping a start works, clearing it does not.
    let s = state().with_persist(Arc::new(|starts: &[UnsettledStart]| {
        if starts.is_empty() {
            Err(Error::Protocol("read-only".into()))
        } else {
            Ok(())
        }
    }));
    create(&f.client, &s, shell_in(None)).await.unwrap();
    assert!(s.take_keep_error().is_some());
    assert!(s.take_keep_error().is_none(), "reported once");
}

/// Workspace `s` (tabs tA, tB) and workspace `s2` (tab tC).
fn two_spaces() -> Value {
    let mut doc = two_tabs();
    doc["spaces"].as_array_mut().unwrap().push(json!(
        {"id":"s2","name":"s2","folder":"/","tabs":[
            {"id":"tC","root":{"id":"tC","type":"row","children":[
                {"id":"p3","type":"block","content":{"terminal":"t3"}}]}}]}));
    doc
}

#[tokio::test]
async fn a_tab_label_shown_in_another_workspace_does_not_end_it() {
    let f = fake(|c| {
        c.doc = two_spaces();
        c.add("t1", json!({}));
        // Shown in s2 (its space label), but its tab label names tA.
        c.add(
            "t5",
            json!({"labels": {SPACE_LABEL: "s2", TAB_LABEL: "tA"}}),
        );
    })
    .await;
    assert_eq!(
        problem(close(&f.client, &state(), Target::Tab("tA")).await),
        LayoutProblem::Partial
    );
    let c = f.core.lock().unwrap();
    assert!(c.live("t5"));
    assert!(c.live("t1"), "refused before anything ended");
    assert_eq!(layout::tab_ids(&c.doc, "s").unwrap(), ["tA", "tB"]);
}

#[tokio::test]
async fn a_tab_the_desktop_is_still_writing_is_not_ended_with_the_first_tab() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
        c.add("t2", json!({}));
        // The desktop started it for a new tab not in the layout yet.
        c.add(
            "t7",
            json!({"labels": {SPACE_LABEL: "s", TAB_LABEL: "tNew"}}),
        );
        // No tab label: shown in the first tab, ends with it.
        c.add("t8", json!({"labels": {SPACE_LABEL: "s"}}));
    })
    .await;
    let s = state();
    // Closing another tab leaves it alone.
    close(&f.client, &s, Target::Tab("tB")).await.unwrap();
    assert!(f.core.lock().unwrap().live("t7"));
    // Closing the first tab while others remain: not ended, not closed.
    f.core.lock().unwrap().doc = two_tabs();
    f.core.lock().unwrap().add("t2", json!({}));
    assert_eq!(
        problem(close(&f.client, &s, Target::Tab("tA")).await),
        LayoutProblem::Unplaced
    );
    let c = f.core.lock().unwrap();
    assert!(c.live("t7"));
    assert!(c.live("t8"), "refused before anything ended");
}

#[tokio::test]
async fn a_terminal_without_a_tab_label_ends_with_the_first_tab() {
    let f = fake(|c| {
        c.doc = two_tabs();
        c.add("t1", json!({}));
        c.add("t2", json!({}));
        c.add("t8", json!({"labels": {SPACE_LABEL: "s"}}));
    })
    .await;
    close(&f.client, &state(), Target::Tab("tA")).await.unwrap();
    let c = f.core.lock().unwrap();
    assert!(!c.live("t8"));
    assert!(c.live("t2"));
}

#[test]
fn backup_names_tell_instances_and_documents_apart() {
    let a = instance_tag(&Endpoint::Socket("/run/a/default.sock".into()));
    let b = instance_tag(&Endpoint::Socket("/run/b/default.sock".into()));
    assert_ne!(a, b);
    assert_eq!(
        a,
        instance_tag(&Endpoint::Socket("/run/a/default.sock".into()))
    );
    let one = backup_name(&a, 3, "{\"x\":1}");
    assert_ne!(
        one,
        backup_name(&b, 3, "{\"x\":1}"),
        "same version, other core"
    );
    assert_ne!(
        one,
        backup_name(&a, 3, "{\"x\":2}"),
        "same version, other content"
    );
    assert_eq!(one, backup_name(&a, 3, "{\"x\":1}"));
    assert!(one.starts_with(&format!("{APP_DOCUMENT}-{a}-v3-")));
    assert!(one.ends_with(".json"));
}

/// A scratch folder under the system's temp dir, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mc-prune-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn touch(&self, name: &str) {
        std::fs::write(self.0.join(name), "{}").unwrap();
    }
    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_sh(script: &str) -> String {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn backup_file(instance: &str, at: u64) -> String {
    format!("{at:013}-{APP_DOCUMENT}-{instance}-v{at}-00ff.json")
}

#[test]
fn pruning_keeps_the_newest_and_this_sessions_copy() {
    let d = Scratch::new("keep");
    for at in 1..=25 {
        d.touch(&backup_file("aa11", at));
    }
    d.touch(&backup_file("bb22", 1));
    let keep = backup_file("aa11", 1);
    let out = run_sh(&prune_script(
        &format!("'{}'", d.0.display()),
        "aa11",
        &keep,
    ));
    assert_eq!(prune_failures(&out), None, "{out}");
    let names = d.names();
    let mine: Vec<_> = names.iter().filter(|n| n.contains("-aa11-")).collect();
    assert_eq!(mine.len(), BACKUPS_KEPT, "{names:?}");
    assert!(names.contains(&keep), "this session's copy stays");
    assert!(names.contains(&backup_file("aa11", 25)), "the newest stays");
    assert!(!names.contains(&backup_file("aa11", 6)), "an old one goes");
    assert!(
        names.contains(&backup_file("bb22", 1)),
        "another core's stays"
    );
}

#[test]
fn a_pruning_failure_shows() {
    use std::os::unix::fs::PermissionsExt;
    let d = Scratch::new("fail");
    for at in 1..=(BACKUPS_KEPT as u64 + 3) {
        d.touch(&backup_file("aa11", at));
    }
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o555)).unwrap();
    let gone = run_sh(&prune_script("'/nonexistent/mc-prune'", "aa11", "none"));
    assert!(prune_failures(&gone).is_some_and(|f| f.contains("cannot be entered")));
    assert!(prune_failures("").is_some(), "no answer is a failure too");
    let script = prune_script(&format!("'{}'", d.0.display()), "aa11", "none");
    let out = run_sh(&script);
    if d.names().len() < BACKUPS_KEPT + 3 {
        // Running with rights that ignore the folder's mode: nothing to see.
        return;
    }
    assert!(
        prune_failures(&out).is_some_and(|f| f.contains("could not")),
        "{out}"
    );
}

#[test]
fn an_unlistable_backup_folder_is_a_pruning_failure() {
    use std::os::unix::fs::PermissionsExt;
    let empty = Scratch::new("empty");
    let out = run_sh(&prune_script(
        &format!("'{}'", empty.0.display()),
        "aa11",
        "none",
    ));
    assert_eq!(prune_failures(&out), None, "an empty folder is fine: {out}");
    let d = Scratch::new("nolist");
    d.touch(&backup_file("aa11", 1));
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o311)).unwrap();
    if std::fs::read_dir(&d.0).is_ok() {
        // Running with rights that ignore the folder's mode: nothing to see.
        return;
    }
    let out = run_sh(&prune_script(
        &format!("'{}'", d.0.display()),
        "aa11",
        "none",
    ));
    assert!(
        prune_failures(&out).is_some_and(|f| f.contains("cannot be listed")),
        "{out}"
    );
}

//! `mc` – developer CLI for the MissionCtrl core.
//!
//! Reads a host list (JSON) and talks to the herdr servers through the same
//! core the apps use. Not shipped to users.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use mc_core::herdr::HerdrClient;
use mc_core::transport::{
    KnownHostsFile, LocalTransport, SshAuth, SshTarget, SshTransport, Transport,
};
use mc_core::{AuthRef, ConnectionState, Connector, Fleet, FleetView, HostProfile};
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "mc", about = "MissionCtrl core – developer CLI")]
struct Cli {
    /// Host list (default: ~/.config/missionctrl/hosts.json)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List configured hosts
    Hosts,
    /// Connect to one host and print herdr version and socket
    Ping { host: String },
    /// Print one host's session snapshot as JSON
    Snapshot { host: String },
    /// Live view of all hosts and their agents (Ctrl+C to quit)
    Watch {
        /// Only these hosts (default: all)
        hosts: Vec<String>,
    },
    /// Print the last lines of a pane
    Read {
        host: String,
        pane: String,
        #[arg(long, default_value_t = 40)]
        lines: u32,
        /// Only print line count and revision, not the content
        #[arg(long)]
        stats: bool,
        /// Keep ANSI colour codes
        #[arg(long)]
        ansi: bool,
        /// Only the visible screen (never makes herdr scroll a resting agent)
        #[arg(long)]
        visible: bool,
    },
    /// Git branch and repository of directories on the host (read-only)
    Dirs { host: String, dirs: Vec<String> },
    /// Upload a local file to ~/.cache/missionctrl/uploads on the host
    Upload { host: String, file: PathBuf },
    /// Create a tab (no focus change), optionally starting an agent
    NewTab {
        host: String,
        workspace: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        /// Agent kind to start, e.g. claude or codex
        #[arg(long)]
        agent: Option<String>,
    },
    /// Create a workspace (no focus change), optionally starting an agent
    NewWorkspace {
        host: String,
        #[arg(long)]
        label: Option<String>,
        /// Directory on the host: `~/…`, absolute, or relative to home; default home
        #[arg(long)]
        cwd: Option<String>,
        /// Agent kind to start, e.g. claude or codex
        #[arg(long)]
        agent: Option<String>,
    },
    /// Close a tab
    CloseTab { host: String, tab: String },
    /// Move a tab to a 0-based position within its workspace
    MoveTab {
        host: String,
        tab: String,
        index: u32,
    },
    /// Move a workspace to a 0-based position
    MoveWorkspace {
        host: String,
        workspace: String,
        index: u32,
    },
    /// Type text into a pane
    Send {
        host: String,
        pane: String,
        text: String,
        /// Press Enter afterwards
        #[arg(long)]
        enter: bool,
    },
    /// Send key combos (`enter`, `ctrl+c`, `home` …) to a pane
    Keys {
        host: String,
        pane: String,
        keys: Vec<String>,
    },
    /// Control a pane as a live terminal (TUI mode): send each input after a
    /// pause, then print the screen. `\r`, `\e`, `\t` in inputs are CR, ESC, Tab.
    Term {
        host: String,
        pane: String,
        #[arg(long, default_value_t = 60)]
        cols: u16,
        #[arg(long, default_value_t = 20)]
        rows: u16,
        /// Pause before each input and before printing, in milliseconds
        #[arg(long, default_value_t = 800)]
        pause: u64,
        inputs: Vec<String>,
    },
}

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    known_hosts: Option<PathBuf>,
    hosts: Vec<HostEntry>,
}

#[derive(Deserialize, Clone)]
struct HostEntry {
    #[serde(flatten)]
    profile: HostProfile,
    /// `local` connects to the Unix socket on this machine instead of SSH.
    #[serde(default)]
    local: bool,
    #[serde(default)]
    auth: Option<AuthRef>,
}

fn expand(path: &Path) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix("~/")) {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => path.to_owned(),
    }
}

fn load_config(path: Option<PathBuf>) -> anyhow::Result<Config> {
    let path = expand(&path.unwrap_or_else(|| PathBuf::from("~/.config/missionctrl/hosts.json")));
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Connector that resolves each profile to SSH (with agent or key file) or the
/// local socket, based on the CLI config.
fn connector(config: &Config) -> Connector {
    let entries: Arc<BTreeMap<String, HostEntry>> = Arc::new(
        config
            .hosts
            .iter()
            .map(|h| (h.profile.id.clone(), h.clone()))
            .collect(),
    );
    let known_hosts = expand(
        config
            .known_hosts
            .as_deref()
            .unwrap_or_else(|| Path::new("~/.ssh/known_hosts")),
    );
    Arc::new(move |profile: HostProfile| {
        let entries = entries.clone();
        let known_hosts = known_hosts.clone();
        Box::pin(async move {
            let entry = entries
                .get(&profile.id)
                .ok_or_else(|| mc_core::Error::Connect(format!("no config for {}", profile.id)))?;
            if entry.local {
                return Ok(Transport::Local(LocalTransport));
            }
            let auth = match &entry.auth {
                Some(AuthRef::Agent { socket, key_filter }) => SshAuth::Agent {
                    socket: expand(socket),
                    key_filter: key_filter.clone(),
                },
                Some(AuthRef::KeyFile { path }) => {
                    let key = mc_core::transport::load_private_key(&expand(path), None)?;
                    SshAuth::Key(Arc::new(key))
                }
                None => SshAuth::Agent {
                    socket: std::env::var_os("SSH_AUTH_SOCK")
                        .map(PathBuf::from)
                        .ok_or_else(|| {
                            mc_core::Error::Agent(
                                "no auth configured and SSH_AUTH_SOCK unset".into(),
                            )
                        })?,
                    key_filter: None,
                },
            };
            let target = SshTarget {
                host: profile.host.clone(),
                port: profile.port,
                user: profile.user.clone(),
            };
            let transport =
                SshTransport::connect(target, auth, Arc::new(KnownHostsFile(known_hosts))).await?;
            Ok(Transport::Ssh(transport))
        })
    })
}

fn find<'a>(config: &'a Config, id: &str) -> anyhow::Result<&'a HostEntry> {
    config
        .hosts
        .iter()
        .find(|h| h.profile.id == id || h.profile.name == id)
        .with_context(|| format!("unknown host {id:?}"))
}

async fn connect_one(config: &Config, id: &str) -> anyhow::Result<HerdrClient> {
    let entry = find(config, id)?;
    let transport = connector(config)(entry.profile.clone()).await?;
    let (client, _) = HerdrClient::connect(
        Arc::new(transport),
        entry.profile.socket_path.as_deref(),
        entry.profile.session.as_deref(),
    )
    .await?;
    Ok(client)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mc_core=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let config = load_config(cli.config)?;

    match cli.command {
        Command::Hosts => {
            for h in &config.hosts {
                let target = if h.local {
                    "local socket".to_owned()
                } else {
                    format!("{}@{}:{}", h.profile.user, h.profile.host, h.profile.port)
                };
                println!("{:<14} {}", h.profile.id, target);
            }
        }
        Command::Ping { host } => {
            let client = connect_one(&config, &host).await?;
            let pong = client.ping().await?;
            println!(
                "herdr {} (protocol {}) at {}",
                pong.version,
                pong.protocol,
                client.socket_path()
            );
        }
        Command::Snapshot { host } => {
            let client = connect_one(&config, &host).await?;
            let snapshot = client.snapshot().await?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        Command::Read {
            host,
            pane,
            lines,
            stats,
            ansi,
            visible,
        } => {
            let client = connect_one(&config, &host).await?;
            let read = if visible {
                client.read_visible(&pane, ansi).await?
            } else {
                client.read_pane_as(&pane, lines, ansi).await?
            };
            if stats {
                println!(
                    "{}: {} lines, revision {}, truncated {}, escapes {}",
                    read.pane_id,
                    read.text.lines().count(),
                    read.revision,
                    read.truncated,
                    read.text.matches('\x1b').count()
                );
            } else {
                println!("{}", read.text);
            }
        }
        Command::Dirs { host, dirs } => {
            let client = connect_one(&config, &host).await?;
            let report = client.transport().dir_info(&dirs).await?;
            println!("home {}", report.home);
            for d in report.dirs {
                println!(
                    "{}  branch={}  repo={}",
                    d.dir,
                    d.branch.unwrap_or_default(),
                    d.repo.unwrap_or_default()
                );
            }
        }
        Command::Upload { host, file } => {
            let data =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let client = connect_one(&config, &host).await?;
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("upload");
            let path = client.transport().upload(name, &data).await?;
            println!("{path}");
        }
        Command::NewTab {
            host,
            workspace,
            label,
            cwd,
            agent,
        } => {
            let client = connect_one(&config, &host).await?;
            let cwd = client
                .transport()
                .resolve_dir(cwd.as_deref().unwrap_or(""))
                .await?;
            let (tab, pane) = client
                .create_tab(&workspace, label.as_deref(), Some(&cwd))
                .await?;
            let name = match agent {
                Some(kind) => client.start_agent_free_name(&pane, &kind).await?,
                None => String::new(),
            };
            println!("{tab} {pane} {cwd} {name}");
        }
        Command::NewWorkspace {
            host,
            label,
            cwd,
            agent,
        } => {
            let client = connect_one(&config, &host).await?;
            let cwd = client
                .transport()
                .resolve_dir(cwd.as_deref().unwrap_or(""))
                .await?;
            let (ws, _, pane) = client
                .create_workspace(label.as_deref(), Some(&cwd))
                .await?;
            let name = match agent {
                Some(kind) => client.start_agent_free_name(&pane, &kind).await?,
                None => String::new(),
            };
            println!("{ws} {pane} {cwd} {name}");
        }
        Command::CloseTab { host, tab } => {
            let client = connect_one(&config, &host).await?;
            client.close_tab(&tab).await?;
        }
        Command::MoveTab { host, tab, index } => {
            let client = connect_one(&config, &host).await?;
            client.move_tab(&tab, index).await?;
        }
        Command::MoveWorkspace {
            host,
            workspace,
            index,
        } => {
            let client = connect_one(&config, &host).await?;
            client.move_workspace(&workspace, index).await?;
        }
        Command::Send {
            host,
            pane,
            text,
            enter,
        } => {
            let client = connect_one(&config, &host).await?;
            if enter {
                client.send_line(&pane, &text).await?;
            } else {
                client.send_text(&pane, &text).await?;
            }
        }
        Command::Keys { host, pane, keys } => {
            let client = connect_one(&config, &host).await?;
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            client.send_keys(&pane, &keys).await?;
        }
        Command::Term {
            host,
            pane,
            cols,
            rows,
            pause,
            inputs,
        } => {
            let client = connect_one(&config, &host).await?;
            let sink = Arc::new(LastScreen::default());
            let session = mc_core::terminal::TerminalSession::open(
                client.transport(),
                client.socket_path(),
                &pane,
                cols,
                rows,
                sink.clone(),
            )
            .await?;
            let pause = std::time::Duration::from_millis(pause);
            for input in inputs {
                tokio::time::sleep(pause).await;
                session.input(unescape(&input).into_bytes());
            }
            tokio::time::sleep(pause).await;
            let screen = sink.screen.lock().unwrap().clone();
            println!(
                "{}x{} cursor {},{} visible {} frames {}",
                screen.cols,
                screen.rows,
                screen.cursor_row,
                screen.cursor_col,
                screen.cursor_visible,
                screen.seq
            );
            println!("{}", screen.text());
            if let Some(why) = sink.ended.lock().unwrap().clone() {
                println!("ended: {why:?}");
            }
            session.release();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        Command::Watch { hosts } => {
            let selected: Vec<&HostEntry> = config
                .hosts
                .iter()
                .filter(|entry| {
                    hosts.is_empty()
                        || hosts
                            .iter()
                            .any(|h| *h == entry.profile.id || *h == entry.profile.name)
                })
                .collect();
            if selected.is_empty() {
                bail!("none of {hosts:?} is configured");
            }
            let fleet = Fleet::new(connector(&config));
            for entry in selected {
                fleet.add_host(entry.profile.clone());
            }
            let mut rx = fleet.subscribe();
            loop {
                tokio::select! {
                    changed = rx.changed() => {
                        if changed.is_err() { break; }
                        render(&rx.borrow_and_update());
                    }
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
            fleet.stop_all();
        }
    }
    Ok(())
}

fn render(view: &FleetView) {
    // Clear screen and move home, then draw the whole view.
    print!("\x1b[2J\x1b[H");
    println!("HOSTS");
    for host in &view.hosts {
        let state = match &host.connection {
            ConnectionState::Connecting => "connecting…".to_owned(),
            ConnectionState::Online => "online".to_owned(),
            ConnectionState::Reconnecting {
                attempt,
                error,
                retry_in_ms,
            } => {
                format!(
                    "reconnecting (#{attempt}, in {:.1}s): {error}",
                    *retry_in_ms as f64 / 1000.0
                )
            }
            ConnectionState::ActionRequired { error } => format!("ACTION REQUIRED: {error:?}"),
            ConnectionState::Offline => "offline".to_owned(),
        };
        let (ws, panes) = host
            .snapshot
            .as_ref()
            .map(|s| (s.workspaces.len(), s.panes.len()))
            .unwrap_or_default();
        println!(
            "  {:<12} {:<9} {:>2} workspaces {:>3} panes  {}{}",
            host.profile.name,
            host.server_version.as_deref().unwrap_or("-"),
            ws,
            panes,
            state,
            if host.stale && host.snapshot.is_some() {
                "  (stale)"
            } else {
                ""
            }
        );
    }
    println!(
        "\nAGENTS ({} blocked, {} done, {} working)",
        view.count(mc_core::herdr::model::AgentStatus::Blocked),
        view.count(mc_core::herdr::model::AgentStatus::Done),
        view.count(mc_core::herdr::model::AgentStatus::Working)
    );
    if view.agents.is_empty() {
        println!("  (none)");
    }
    for a in &view.agents {
        let place = format!(
            "{} / {}",
            a.workspace_label.as_deref().unwrap_or("?"),
            a.tab_label.as_deref().unwrap_or("?")
        );
        println!(
            "  {:<9} {:<12} {:<14} {:<28} {}{}",
            a.agent.status_label(),
            a.host_name,
            a.agent.display_name(),
            place,
            a.agent.pane_id,
            if a.stale { "  (stale)" } else { "" }
        );
    }
}

#[derive(Default)]
struct LastScreen {
    screen: std::sync::Mutex<mc_core::terminal::Screen>,
    ended: std::sync::Mutex<Option<mc_core::terminal::Ended>>,
}

impl mc_core::terminal::TerminalSink for LastScreen {
    fn screen(&self, screen: mc_core::terminal::Screen) {
        *self.screen.lock().unwrap() = screen;
    }
    fn ended(&self, why: mc_core::terminal::Ended) {
        *self.ended.lock().unwrap() = Some(why);
    }
}

fn unescape(s: &str) -> String {
    s.replace("\\r", "\r")
        .replace("\\e", "\x1b")
        .replace("\\t", "\t")
}

//! `mc` – developer CLI for the MissionCtrl core.
//!
//! Reads a host list (JSON) and talks to the herdr servers through the same
//! core the apps use. Not shipped to users.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use mc_core::hc::layout::{self, Anchor};
use mc_core::hc::model::APP_DOCUMENT;
use mc_core::hc::write::{self as layout_write, LayoutState, Persist, Target, UnsettledStart};
use mc_core::hc::{CoreClient, Endpoint};
use mc_core::herdr::HerdrClient;
use mc_core::transport::{
    KnownHostsFile, LocalTransport, SshAuth, SshTarget, SshTransport, Transport,
};
use mc_core::{
    AuthRef, Backend, ConnectionState, Connector, Fleet, FleetView, HostProfile, ServerClient,
};
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "mc", about = "MissionCtrl core – developer CLI")]
struct Cli {
    /// Host list (default: ~/.config/missionctrl/hosts.json)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Allow changing the Herdr app's layout on herdr-core for this call
    /// (otherwise only hosts with `"edit_layout": true` in the host list)
    #[arg(long, global = true)]
    edit_layout: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List configured hosts
    Hosts,
    /// Connect to one host and print each herdr server that answers (TUI,
    /// herdr-core) with version and socket
    Ping { host: String },
    /// Print one host's session snapshot as JSON (herdr-core: mapped onto the
    /// same shape)
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
        /// herdr-core: a line to type into the new shell
        #[arg(long)]
        command: Option<String>,
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
        /// herdr-core: a line to type into the new shell
        #[arg(long)]
        command: Option<String>,
    },
    /// Close a tab
    CloseTab { host: String, tab: String },
    /// Close a workspace
    CloseWorkspace { host: String, workspace: String },
    /// herdr-core: end a terminal that stands in no workspace
    CloseTerminal { host: String, pane: String },
    /// Rename a tab
    RenameTab {
        host: String,
        tab: String,
        label: String,
    },
    /// Rename a workspace
    RenameWorkspace {
        host: String,
        workspace: String,
        label: String,
    },
    /// herdr-core: the Herdr app's layout document (`herdr-app`)
    Doc {
        #[command(subcommand)]
        action: DocAction,
    },
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
    /// Mark a pane's last turn as seen (herdr-core: label `herdr.seen`)
    Seen { host: String, pane: String },
    /// Control a pane as a live terminal (TUI mode): send each input after a
    /// pause, then print the screen. `\r`, `\e`, `\t`, `\n` in inputs are CR, ESC, Tab, LF.
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
        /// Open, type and release this many times on one connection
        #[arg(long, default_value_t = 1)]
        repeat: u32,
        /// Keep the connection this long afterwards (milliseconds), e.g. to
        /// count the server's open streams
        #[arg(long, default_value_t = 0)]
        hold: u64,
        inputs: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DocAction {
    /// Save the document and its version to a local file
    Backup { host: String, file: PathBuf },
    /// Write a saved document back (over whatever is there now)
    Restore { host: String, file: PathBuf },
    /// Print the document
    Show { host: String },
}

/// What a herdr-core new tab runs.
fn core_start(agent: Option<String>, command: Option<String>) -> layout_write::Start {
    match (agent, command) {
        (Some(kind), _) => layout_write::Start::Program(vec![kind]),
        (None, Some(line)) => layout_write::Start::Line(line),
        (None, None) => layout_write::Start::Shell,
    }
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

/// Layout changes of one `mc` call: allowed by the host's `edit_layout` or
/// `--edit-layout`. Starts that did not answer are kept in a file between
/// calls (`~/.cache/missionctrl/`), so the next call does not start a second
/// terminal while the first may still arrive. The file is locked for the
/// whole call (read, change, keep): two `mc` calls on one host take turns
/// and cannot drop each other's open starts. Every change to the open
/// starts is written at once (a start is on record before it goes out),
/// replacing the file whole.
struct CliLayout {
    state: LayoutState,
    /// Held until the call ends.
    _lock: std::fs::File,
}

impl CliLayout {
    fn new(config: &Config, host: &str, flag: bool) -> anyhow::Result<Self> {
        let entry = find(config, host)?;
        let safe: String = entry
            .profile
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file = expand(Path::new(&format!(
            "~/.cache/missionctrl/cli-layout-{safe}.json"
        )));
        let dir = file
            .parent()
            .context("ledger file has no folder")?
            .to_owned();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(file.with_extension("lock"))
            .context("opening the layout lock file")?;
        lock.lock().context("locking the layout lock file")?;
        let kept = match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("reading {}", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
        };
        let persist: Persist = Arc::new(move |starts: &[UnsettledStart]| {
            keep_starts(&file, starts).map_err(|e| {
                mc_core::Error::Protocol(format!("keeping open starts in {}: {e}", file.display()))
            })
        });
        let state = LayoutState::new(entry.profile.edit_layout || flag).with_persist(persist);
        state.restore_unsettled(kept);
        Ok(Self { state, _lock: lock })
    }

    /// After a start that worked: fail when its record could not be
    /// cleared. The record left behind blocks nothing for long and never
    /// ends the placed terminal, but the file needs looking at.
    fn kept(&self) -> anyhow::Result<()> {
        match self.state.take_keep_error() {
            Some(e) => anyhow::bail!("created, but clearing the open start failed: {e}"),
            None => Ok(()),
        }
    }
}

/// Write the open starts to `file` whole (a temporary file renamed over
/// it), or remove it when there are none.
fn keep_starts(file: &Path, starts: &[UnsettledStart]) -> std::io::Result<()> {
    if starts.is_empty() {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_vec(starts)?;
    let tmp = file.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&text)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, file)
}

/// The anchor of a move to boundary `index` of `list` as it is now.
fn anchor_at(list: &[String], index: u32) -> Anchor {
    Anchor::from_index(list, index as usize)
}

/// The server a host's profile selects, by the app's own rule
/// ([`mc_core::host::select_backend`]): `tui` and `core` exactly that one,
/// `auto` the TUI unless it is surely absent. Only a `core` profile may start
/// herdr-core.
async fn connect_server(config: &Config, id: &str) -> anyhow::Result<ServerClient> {
    let entry = find(config, id)?;
    let transport = Arc::new(connector(config)(entry.profile.clone()).await?);
    let socket = entry.profile.socket_path.as_deref();
    let session = entry.profile.session.as_deref();
    let start = entry.profile.backend == Backend::Core;
    let client = mc_core::host::select_backend(entry.profile.backend, None, |backend| {
        let transport = transport.clone();
        async move {
            Ok(match backend {
                Backend::Core => ServerClient::Core(Arc::new(
                    core_client(transport, socket, session, start).await?,
                )),
                _ => {
                    let (client, _) = HerdrClient::connect(transport, socket, session).await?;
                    ServerClient::Tui(Arc::new(client))
                }
            })
        }
    })
    .await?;
    Ok(client)
}

/// The herdr TUI client for workspace and tab commands; a host on herdr-core
/// is refused, never quietly sent to a TUI server on the same machine.
async fn connect_one(config: &Config, id: &str) -> anyhow::Result<Arc<HerdrClient>> {
    Ok(connect_server(config, id).await?.tui()?.clone())
}

async fn connect_core(config: &Config, id: &str) -> anyhow::Result<CoreClient> {
    let entry = find(config, id)?;
    let transport = connector(config)(entry.profile.clone()).await?;
    Ok(core_client(
        Arc::new(transport),
        entry.profile.socket_path.as_deref(),
        entry.profile.session.as_deref(),
        entry.profile.backend == Backend::Core,
    )
    .await?)
}

/// A herdr-core client as the app connects it (sockets, then `herdr-core
/// connect` when `start` allows it). `MC_HC_CONNECT` forces a command for the
/// `herdr-core connect` route, e.g. to test it against a core with its own
/// directories.
async fn core_client(
    transport: Arc<mc_core::transport::Transport>,
    socket: Option<&str>,
    session: Option<&str>,
    start: bool,
) -> mc_core::Result<CoreClient> {
    let (client, _) = match std::env::var("MC_HC_CONNECT") {
        Ok(command) => CoreClient::connect_to(transport, Endpoint::Exec(command)).await?,
        Err(_) => CoreClient::connect(transport, socket, session, start).await?,
    };
    Ok(client)
}

/// `mc doc`: back up, restore or show the Herdr app's layout document.
async fn doc_command(config: &Config, action: DocAction) -> anyhow::Result<()> {
    match action {
        DocAction::Show { host } => {
            let core = connect_core(config, &host).await?;
            let got = core.doc_get(APP_DOCUMENT).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                "version": got.version, "value": got.value }))?
            );
        }
        DocAction::Backup { host, file } => {
            let core = connect_core(config, &host).await?;
            let got = core.doc_get(APP_DOCUMENT).await?;
            let saved = serde_json::json!({ "version": got.version, "value": got.value });
            std::fs::write(&file, serde_json::to_vec_pretty(&saved)?)?;
            println!("saved version {} to {}", got.version, file.display());
        }
        DocAction::Restore { host, file } => {
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
            // A backup file ({version, value}) or a bare document.
            let value = match saved.get("value") {
                Some(v) if saved.get("version").is_some() => v.clone(),
                _ => saved,
            };
            if let Err(why) = mc_core::hc::layout::check_writable(&value) {
                bail!("not a document the Herdr app can read: {why}");
            }
            let core = connect_core(config, &host).await?;
            let now = core.doc_get(APP_DOCUMENT).await?;
            let version = core.doc_set(APP_DOCUMENT, &value, now.version).await?;
            println!("restored (version {} → {version})", now.version);
        }
    }
    Ok(())
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
            let entry = find(&config, &host)?;
            let mut found = 0;
            if entry.profile.backend != Backend::Core
                && let Ok(client) = connect_one(&config, &host).await
            {
                let pong = client.ping().await?;
                println!(
                    "herdr {} (protocol {}) at {}",
                    pong.version,
                    pong.protocol,
                    client.socket_path()
                );
                found += 1;
            }
            if entry.profile.backend != Backend::Tui
                && let Ok(client) = connect_core(&config, &host).await
            {
                let hello = client.hello().await?;
                println!(
                    "herdr-core {} (generation {}, core {:?}) at {}",
                    hello.version,
                    hello.generation,
                    hello.core.name,
                    client.endpoint()
                );
                found += 1;
            }
            if found == 0 {
                bail!("no herdr server answers on {host}");
            }
        }
        Command::Snapshot { host } => {
            let snapshot = match connect_server(&config, &host).await? {
                ServerClient::Tui(client) => client.snapshot().await?,
                ServerClient::Core(client) => {
                    let hello = client.hello().await?;
                    let has_doc = hello.methods.iter().any(|m| m == "doc.get");
                    mc_core::hc::snapshot::build(
                        &hello,
                        &client.terminals().await?,
                        &client.app_document(has_doc).await?.doc,
                        &mut mc_core::hc::snapshot::LocalSeen::default(),
                    )
                }
            };
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
            let client = connect_server(&config, &host).await?;
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
            let client = connect_server(&config, &host).await?;
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
            let client = connect_server(&config, &host).await?;
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
            command,
        } => {
            if let ServerClient::Core(core) = connect_server(&config, &host).await? {
                let cwd = core
                    .transport()
                    .resolve_dir(cwd.as_deref().unwrap_or(""))
                    .await?;
                let layout = CliLayout::new(&config, &host, cli.edit_layout)?;
                let made = layout_write::create(
                    &core,
                    &layout.state,
                    layout_write::NewTab {
                        space: Some(workspace),
                        space_name: String::new(),
                        tab_name: label.unwrap_or_default(),
                        cwd: cwd.clone(),
                        start: core_start(agent, command),
                        size: layout_write::DEFAULT_SIZE,
                    },
                )
                .await;
                let made = made?;
                println!("{} {} {cwd}", made.tab, made.address);
                return layout.kept();
            }
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
            command,
        } => {
            if let ServerClient::Core(core) = connect_server(&config, &host).await? {
                let cwd = core
                    .transport()
                    .resolve_dir(cwd.as_deref().unwrap_or(""))
                    .await?;
                let layout = CliLayout::new(&config, &host, cli.edit_layout)?;
                let made = layout_write::create(
                    &core,
                    &layout.state,
                    layout_write::NewTab {
                        space: None,
                        space_name: label.unwrap_or_default(),
                        tab_name: String::new(),
                        cwd: cwd.clone(),
                        start: core_start(agent, command),
                        size: layout_write::DEFAULT_SIZE,
                    },
                )
                .await;
                let made = made?;
                println!("{} {} {} {cwd}", made.space, made.tab, made.address);
                return layout.kept();
            }
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
        Command::CloseTab { host, tab } => match connect_server(&config, &host).await? {
            ServerClient::Core(core) => {
                layout_write::close(
                    &core,
                    &CliLayout::new(&config, &host, cli.edit_layout)?.state,
                    Target::Tab(&tab),
                )
                .await?
            }
            ServerClient::Tui(client) => client.close_tab(&tab).await?,
        },
        Command::CloseWorkspace { host, workspace } => {
            match connect_server(&config, &host).await? {
                ServerClient::Core(core) => {
                    layout_write::close(
                        &core,
                        &CliLayout::new(&config, &host, cli.edit_layout)?.state,
                        Target::Space(&workspace),
                    )
                    .await?
                }
                ServerClient::Tui(client) => client.close_workspace(&workspace).await?,
            }
        }
        Command::CloseTerminal { host, pane } => {
            let core = connect_core(&config, &host).await?;
            let layout = CliLayout::new(&config, &host, cli.edit_layout)?;
            layout_write::close_terminal(&core, &layout.state, &pane).await?;
        }
        Command::RenameTab { host, tab, label } => match connect_server(&config, &host).await? {
            ServerClient::Core(core) => {
                layout_write::rename_tab(
                    &core,
                    &CliLayout::new(&config, &host, cli.edit_layout)?.state,
                    &tab,
                    &label,
                )
                .await?
            }
            ServerClient::Tui(client) => client.rename_tab(&tab, &label).await?,
        },
        Command::RenameWorkspace {
            host,
            workspace,
            label,
        } => match connect_server(&config, &host).await? {
            ServerClient::Core(core) => {
                layout_write::rename_space(
                    &core,
                    &CliLayout::new(&config, &host, cli.edit_layout)?.state,
                    &workspace,
                    &label,
                )
                .await?
            }
            ServerClient::Tui(client) => client.rename_workspace(&workspace, &label).await?,
        },
        Command::MoveTab { host, tab, index } => match connect_server(&config, &host).await? {
            ServerClient::Core(core) => {
                let layout = CliLayout::new(&config, &host, cli.edit_layout)?;
                let doc = core.doc_get(APP_DOCUMENT).await?.value;
                let list = layout::space_of_tab(&doc, &tab)
                    .and_then(|s| layout::tab_ids(&doc, &s))
                    .unwrap_or_default();
                layout_write::move_tab(&core, &layout.state, &tab, &anchor_at(&list, index)).await?
            }
            ServerClient::Tui(client) => client.move_tab(&tab, index).await?,
        },
        Command::MoveWorkspace {
            host,
            workspace,
            index,
        } => match connect_server(&config, &host).await? {
            ServerClient::Core(core) => {
                let layout = CliLayout::new(&config, &host, cli.edit_layout)?;
                let doc = core.doc_get(APP_DOCUMENT).await?.value;
                let list = layout::space_ids(&doc);
                layout_write::move_space(&core, &layout.state, &workspace, &anchor_at(&list, index))
                    .await?
            }
            ServerClient::Tui(client) => client.move_workspace(&workspace, index).await?,
        },
        Command::Doc { action } => doc_command(&config, action).await?,
        Command::Send {
            host,
            pane,
            text,
            enter,
        } => {
            let client = connect_server(&config, &host).await?;
            if enter {
                client.send_line(&pane, &text).await?;
            } else {
                client.send_text(&pane, &text).await?;
            }
        }
        Command::Seen { host, pane } => {
            let client = connect_server(&config, &host).await?;
            client.mark_seen(&pane).await?;
        }
        Command::Keys { host, pane, keys } => {
            let client = connect_server(&config, &host).await?;
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            client.send_keys(&pane, &keys).await?;
        }
        Command::Term {
            host,
            pane,
            cols,
            rows,
            pause,
            repeat,
            hold,
            inputs,
        } => {
            let client = connect_server(&config, &host).await?;
            let pause = std::time::Duration::from_millis(pause);
            for round in 1..=repeat.max(1) {
                let sink = Arc::new(LastScreen::default());
                let session = client
                    .open_terminal(&pane, cols, rows, sink.clone())
                    .await?;
                for input in &inputs {
                    tokio::time::sleep(pause).await;
                    session.input(unescape(input).into_bytes());
                }
                tokio::time::sleep(pause).await;
                if round == repeat.max(1) {
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
                }
                session.release();
                // Wait for the end the session reports, at most a few seconds.
                for _ in 0..50 {
                    if sink.ended.lock().unwrap().is_some() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                if let Some(why) = sink.ended.lock().unwrap().clone() {
                    println!("round {round} ended: {why:?}");
                }
            }
            if hold > 0 {
                println!("holding the connection for {hold} ms");
                tokio::time::sleep(std::time::Duration::from_millis(hold)).await;
            }
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
                ..
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
        let backend = match host.backend {
            Some(Backend::Core) => "core",
            Some(_) => "tui",
            None => "-",
        };
        let also = if host.available_backends.len() > 1 {
            "  (tui and core here)"
        } else {
            ""
        };
        println!(
            "  {:<12} {:<4} {:<9} {:>2} workspaces {:>3} panes  {}{}{also}",
            host.profile.name,
            backend,
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
        .replace("\\n", "\n")
}

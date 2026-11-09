//! Byte-stream transports to the herdr sockets.
//!
//! A [`Transport`] opens fresh bidirectional streams to Unix socket paths on
//! the machine running herdr. Over SSH this is a
//! `direct-streamlocal@openssh.com` channel (like `ssh -W /path`); locally it
//! is a plain Unix socket connection (used for tests and desktop development).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::io::{AsyncRead, AsyncWrite};
pub use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

pub type BoxStream = Box<dyn AsyncStream>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Limit for short remote commands (discovery, Git facts).
const EXEC_TIMEOUT: Duration = Duration::from_secs(15);
/// Limit for one upload (a screenshot over a slow mobile link).
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Printed by [`Transport::exec_sh`] before the script's own output, so
/// anything the login prints first (motd, `.bashrc` noise) is skipped.
const OUTPUT_MARK: &str = "@@missionctrl-output@@";

/// Environment of the remote login, needed to locate the herdr socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEnv {
    pub home: String,
    pub xdg_config_home: Option<String>,
}

pub enum Transport {
    #[cfg(unix)]
    Local(LocalTransport),
    Ssh(SshTransport),
}

impl Transport {
    /// Open a new stream to the Unix socket at `path` on the herdr machine.
    pub async fn open_socket(&self, path: &str) -> Result<BoxStream> {
        match self {
            #[cfg(unix)]
            Transport::Local(t) => t.open_socket(path).await,
            Transport::Ssh(t) => t.open_socket(path).await,
        }
    }

    pub async fn remote_env(&self) -> Result<RemoteEnv> {
        match self {
            #[cfg(unix)]
            Transport::Local(t) => t.remote_env(),
            Transport::Ssh(t) => t.remote_env().await,
        }
    }

    /// Store `data` as a new file in the upload directory on the herdr machine
    /// (`~/.cache/missionctrl/uploads/`) and return its absolute path.
    /// `file_name` is sanitized and made unique.
    pub async fn upload(&self, file_name: &str, data: &[u8]) -> Result<String> {
        let env = self.remote_env().await?;
        let dir = format!(
            "{}/.cache/missionctrl/uploads",
            env.home.trim_end_matches('/')
        );
        let path = format!("{dir}/{}", unique_upload_name(file_name));
        match self {
            #[cfg(unix)]
            Transport::Local(_) => {
                use tokio::io::AsyncWriteExt;
                tokio::fs::create_dir_all(&dir).await?;
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .await?;
                file.write_all(data).await?;
                file.flush().await?;
            }
            Transport::Ssh(t) => t.write_file(&dir, &path, data).await?,
        }
        Ok(path)
    }

    /// Home directory plus Git branch and repository name of each directory in
    /// `dirs` on the herdr machine (for the path line under an agent's name).
    /// herdr does not report these, so they come from `git -C <dir>`. Missing
    /// Git or a directory outside a repository gives empty fields, not an error.
    pub async fn dir_info(&self, dirs: &[String]) -> Result<DirReport> {
        let out = self.exec_sh(&dir_info_script(dirs)).await?;
        Ok(parse_dir_info(dirs, &out))
    }

    /// Absolute path of the directory `raw` on the herdr machine: `~` and `~/…`
    /// expanded, relative paths taken from the home directory, empty = home.
    /// herdr takes `cwd` literally and silently starts in the home directory when
    /// it does not exist, so a missing directory is an error here
    /// ([`Error::NoSuchDirectory`]).
    pub async fn resolve_dir(&self, raw: &str) -> Result<String> {
        let out = self.exec_sh(&resolve_dir_script(raw)).await?;
        parse_resolve_dir(raw, &out)
    }

    /// A plan an agent wrote (`~/.claude…/plans/<name>.md`, path as shown on its screen), at
    /// most [PLAN_MAX] bytes. Other paths are refused: the path comes from pane output.
    pub async fn read_plan(&self, path: &str) -> Result<String> {
        let env = self.remote_env().await?;
        let full = plan_path(&env.home, path)
            .ok_or_else(|| Error::Protocol(format!("not a plan file: {path}")))?;
        self.exec_sh(&format!("head -c {PLAN_MAX} {}", shell_quote(&full)))
            .await
    }

    /// Run a POSIX `sh` script on the herdr machine and return its stdout,
    /// whatever the login shell is (fish, …) and whatever it prints itself.
    pub async fn exec_sh(&self, script: &str) -> Result<String> {
        match self {
            #[cfg(unix)]
            Transport::Local(t) => t.exec_sh(script).await,
            Transport::Ssh(t) => t.exec_sh(script).await,
        }
    }

    /// Start a long-running POSIX `sh` script on the herdr machine and talk to
    /// it: writes go to its stdin, reads come from its stdout. stderr is not
    /// captured, so scripts that report errors redirect it (`2>&1`). Dropping
    /// the stream closes stdin. `script` must not contain backslashes: over
    /// SSH it is single-quoted for the login shell, which may be fish.
    pub async fn exec_stream(&self, script: &str) -> Result<BoxStream> {
        match self {
            #[cfg(unix)]
            Transport::Local(t) => t.exec_stream(script),
            Transport::Ssh(t) => t.exec_stream(script).await,
        }
    }

    /// True once the underlying connection is gone and must be re-established.
    pub fn is_closed(&self) -> bool {
        match self {
            #[cfg(unix)]
            Transport::Local(_) => false,
            Transport::Ssh(t) => t.handle.is_closed(),
        }
    }

    pub async fn close(&self) {
        if let Transport::Ssh(t) = self {
            let _ = t
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Local
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub struct LocalTransport;

#[cfg(unix)]
impl LocalTransport {
    async fn open_socket(&self, path: &str) -> Result<BoxStream> {
        let stream =
            tokio::net::UnixStream::connect(path)
                .await
                .map_err(|e| Error::SocketUnavailable {
                    path: path.to_owned(),
                    reason: e.to_string(),
                })?;
        Ok(Box::new(stream))
    }

    fn remote_env(&self) -> Result<RemoteEnv> {
        let home = std::env::var("HOME").map_err(|_| Error::Protocol("HOME is not set".into()))?;
        let xdg_config_home = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty());
        Ok(RemoteEnv {
            home,
            xdg_config_home,
        })
    }

    async fn exec_sh(&self, script: &str) -> Result<String> {
        let script = with_mark(script);
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(script)
                .output()
        })
        .await
        .map_err(|e| Error::Protocol(e.to_string()))??;
        after_mark(&String::from_utf8_lossy(&output.stdout))
    }

    fn exec_stream(&self, script: &str) -> Result<BoxStream> {
        use std::process::Stdio;
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(Error::Protocol("no pipes to the local process".into()));
        };
        // Reap the process once it ends (closing stdin ends it).
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        Ok(Box::new(tokio::io::join(stdout, stdin)))
    }
}

// ---------------------------------------------------------------------------
// SSH
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
}

#[derive(Clone)]
pub enum SshAuth {
    /// Sign through an ssh-agent socket.
    Agent {
        socket: PathBuf,
        key_filter: Option<String>,
    },
    /// Sign with an in-memory private key (the app's own key store).
    Key(Arc<PrivateKey>),
    /// Password, tried as `password` and then as `keyboard-interactive`
    /// (many PAM setups only offer the latter). Wiped from memory on drop.
    Password(Zeroizing<String>),
}

/// Load an OpenSSH private key file (optionally encrypted).
pub fn load_private_key(path: &std::path::Path, passphrase: Option<&str>) -> Result<PrivateKey> {
    russh::keys::load_secret_key(path, passphrase)
        .map_err(|e| Error::Connect(format!("loading key {}: {e}", path.display())))
}

/// Result of checking a server's host key against stored trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    Unknown,
    Changed,
}

/// Decides whether a server host key is trusted. The app implements this on
/// top of its own store; [`KnownHostsFile`] reads an OpenSSH `known_hosts`.
pub trait HostKeyVerifier: Send + Sync + 'static {
    fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus;
}

/// Verifier backed by an OpenSSH `known_hosts` file (read-only).
pub struct KnownHostsFile(pub PathBuf);

impl HostKeyVerifier for KnownHostsFile {
    fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        match russh::keys::check_known_hosts_path(host, port, key, &self.0) {
            Ok(true) => HostKeyStatus::Trusted,
            Ok(false) => HostKeyStatus::Unknown,
            Err(russh::keys::Error::KeyChanged { .. }) => HostKeyStatus::Changed,
            Err(_) => HostKeyStatus::Unknown,
        }
    }
}

struct ClientHandler {
    target: SshTarget,
    verifier: Arc<dyn HostKeyVerifier>,
    rejection: Arc<Mutex<Option<Error>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            self.reject(Error::Connect(
                "SSH host certificates are not supported".into(),
            ));
            return Ok(false);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        match self
            .verifier
            .verify(&self.target.host, self.target.port, key)
        {
            HostKeyStatus::Trusted => Ok(true),
            HostKeyStatus::Unknown => {
                self.reject(Error::HostKeyUnknown {
                    host: self.target.host.clone(),
                    port: self.target.port,
                    fingerprint,
                    openssh: key.to_openssh().unwrap_or_default(),
                });
                Ok(false)
            }
            HostKeyStatus::Changed => {
                self.reject(Error::HostKeyChanged {
                    host: self.target.host.clone(),
                    port: self.target.port,
                    fingerprint,
                });
                Ok(false)
            }
        }
    }
}

impl ClientHandler {
    fn reject(&self, error: Error) {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(error);
        }
    }
}

pub struct SshTransport {
    handle: client::Handle<ClientHandler>,
    env: tokio::sync::OnceCell<RemoteEnv>,
}

impl SshTransport {
    pub async fn connect(
        target: SshTarget,
        auth: SshAuth,
        verifier: Arc<dyn HostKeyVerifier>,
    ) -> Result<Self> {
        let config = Arc::new(client::Config {
            // Detect dead links (mobile networks) without an inactivity cutoff.
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 3,
            inactivity_timeout: None,
            nodelay: true,
            ..Default::default()
        });
        let rejection = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            target: target.clone(),
            verifier,
            rejection: rejection.clone(),
        };

        let addr = (target.host.clone(), target.port);
        let connected =
            tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, addr, handler))
                .await
                .map_err(|_| Error::Timeout)?;
        let mut handle = match connected {
            Ok(handle) => handle,
            Err(e) => {
                let rejected = rejection.lock().ok().and_then(|mut slot| slot.take());
                return Err(rejected.unwrap_or_else(|| Error::Connect(e.to_string())));
            }
        };

        let authenticated = tokio::time::timeout(
            CONNECT_TIMEOUT,
            authenticate(&mut handle, &target.user, auth),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        if !authenticated {
            return Err(Error::AuthFailed { user: target.user });
        }
        tracing::debug!(host = %target.host, user = %target.user, "ssh authenticated");
        Ok(Self {
            handle,
            env: tokio::sync::OnceCell::new(),
        })
    }

    async fn open_socket(&self, path: &str) -> Result<BoxStream> {
        let channel = self
            .handle
            .channel_open_direct_streamlocal(path)
            .await
            .map_err(|e| Error::SocketUnavailable {
                path: path.to_owned(),
                reason: e.to_string(),
            })?;
        Ok(Box::new(channel.into_stream()))
    }

    /// Asked once per connection; it does not change.
    async fn remote_env(&self) -> Result<RemoteEnv> {
        self.env
            .get_or_try_init(|| async {
                let out = self
                    .exec_sh(r#"printf '%s\n%s\n' "$HOME" "${XDG_CONFIG_HOME:-}""#)
                    .await?;
                parse_remote_env(&out)
            })
            .await
            .cloned()
    }

    /// Write a new file through `cat`, streaming `data` as stdin. Fails
    /// rather than overwrite an existing file (`set -C`).
    async fn write_file(&self, dir: &str, path: &str, data: &[u8]) -> Result<()> {
        let command = format!(
            "sh -c {} sh {} {}",
            shell_quote(r#"umask 077 && mkdir -p "$1" && set -C && cat > "$2""#),
            shell_quote(dir),
            shell_quote(path)
        );
        let out = self.run(&command, data, UPLOAD_TIMEOUT).await?;
        match out.status {
            Some(0) => Ok(()),
            other => Err(Error::Protocol(format!(
                "upload failed (exit {other:?}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
        }
    }

    /// The script goes in on stdin of a plain `sh`, so it never passes
    /// through the quoting rules of the login shell.
    async fn exec_sh(&self, script: &str) -> Result<String> {
        let out = self
            .run("sh", with_mark(script).as_bytes(), EXEC_TIMEOUT)
            .await?;
        let stdout = String::from_utf8(out.stdout)
            .map_err(|_| Error::Protocol("remote output is not UTF-8".into()))?;
        after_mark(&stdout)
    }

    async fn exec_stream(&self, script: &str) -> Result<BoxStream> {
        let channel = self.handle.channel_open_session().await?;
        channel
            .exec(true, format!("sh -c {}", shell_quote(script)))
            .await?;
        Ok(Box::new(channel.into_stream()))
    }

    /// Run `command`, feed it `stdin`, and collect its output until the
    /// channel closes.
    async fn run(&self, command: &str, stdin: &[u8], limit: Duration) -> Result<Output> {
        tokio::time::timeout(limit, async {
            let mut channel = self.handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            channel.data(stdin).await?;
            channel.eof().await?;
            let mut out = Output::default();
            while let Some(msg) = channel.wait().await {
                match msg {
                    russh::ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                    russh::ChannelMsg::ExtendedData { data, .. } => {
                        out.stderr.extend_from_slice(&data)
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => out.status = Some(exit_status),
                    russh::ChannelMsg::Close => break,
                    _ => {}
                }
            }
            Ok(out)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

#[derive(Default)]
struct Output {
    status: Option<u32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Prefix `script` so it first prints [`OUTPUT_MARK`] on a line of its own.
fn with_mark(script: &str) -> String {
    format!("printf '%s\\n' '{OUTPUT_MARK}'; {script}")
}

/// What a [`with_mark`] script printed after the mark.
fn after_mark(out: &str) -> Result<String> {
    out.split_once(&format!("{OUTPUT_MARK}\n"))
        .map(|(_, rest)| rest.to_owned())
        .ok_or_else(|| Error::Protocol(format!("remote command printed no output: {out:?}")))
}

fn parse_remote_env(out: &str) -> Result<RemoteEnv> {
    let mut lines = out.lines();
    let home = lines
        .next()
        .filter(|h| h.starts_with('/'))
        .ok_or_else(|| Error::Protocol(format!("unexpected remote HOME: {out:?}")))?
        .to_owned();
    let xdg_config_home = lines
        .next()
        .filter(|v| v.starts_with('/'))
        .map(str::to_owned);
    Ok(RemoteEnv {
        home,
        xdg_config_home,
    })
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    auth: SshAuth,
) -> Result<bool> {
    let rsa_hash = handle.best_supported_rsa_hash().await?.flatten();
    match auth {
        SshAuth::Key(key) => {
            let result = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(key, rsa_hash))
                .await?;
            Ok(result.success())
        }
        SshAuth::Password(password) => {
            if handle
                .authenticate_password(user, password.as_str())
                .await?
                .success()
            {
                return Ok(true);
            }
            keyboard_interactive(handle, user, &password).await
        }
        SshAuth::Agent { socket, key_filter } => {
            let mut agent = AgentClient::connect_uds(&socket)
                .await
                .map_err(|e| Error::Agent(format!("{}: {e}", socket.display())))?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|e| Error::Agent(e.to_string()))?;
            for identity in identities {
                let AgentIdentity::PublicKey { key, comment } = identity else {
                    continue;
                };
                if let Some(filter) = &key_filter {
                    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
                    if !comment.contains(filter.as_str()) && !fingerprint.contains(filter.as_str())
                    {
                        continue;
                    }
                }
                let result = handle
                    .authenticate_publickey_with(user, key, rsa_hash, &mut agent)
                    .await
                    .map_err(|e| Error::Agent(e.to_string()))?;
                if result.success() {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

/// Answer every keyboard-interactive prompt with the password. Gives up after
/// a few rounds so a server asking for something else (e.g. an OTP) fails
/// cleanly instead of looping.
async fn keyboard_interactive(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    password: &str,
) -> Result<bool> {
    use russh::client::KeyboardInteractiveAuthResponse as Reply;
    const ROUNDS: usize = 4;
    let mut reply = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await?;
    let mut rounds = 0;
    loop {
        match reply {
            Reply::Success => return Ok(true),
            Reply::Failure { .. } => return Ok(false),
            Reply::InfoRequest { .. } if rounds == ROUNDS => return Ok(false),
            Reply::InfoRequest { prompts, .. } => {
                rounds += 1;
                let answers = prompts.iter().map(|_| password.to_owned()).collect();
                reply = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

/// Largest plan [`Transport::read_plan`] returns.
pub const PLAN_MAX: usize = 256 * 1024;

/// A Markdown file in a `plans` directory, `~/…` or absolute, with `~` expanded; None for
/// anything else (other files, `..`, shell characters, relative paths).
fn plan_path(home: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._-/~".contains(c);
    if !raw.chars().all(allowed) || raw.contains("..") || !raw.ends_with(".md") {
        return None;
    }
    if !raw.rsplit_once('/')?.0.ends_with("/plans") {
        return None;
    }
    let full = match raw.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.trim_end_matches('/')),
        None if raw.starts_with('/') => raw.to_owned(),
        None => return None,
    };
    (!full.contains('~')).then_some(full)
}

/// POSIX shell script for [`Transport::resolve_dir`]: `ok<TAB>path` or `missing`.
fn resolve_dir_script(raw: &str) -> String {
    let raw = raw.trim();
    let raw = if raw.is_empty() { "~" } else { raw };
    format!(
        concat!(
            "d={}; ",
            "case \"$d\" in \"~\") d=\"$HOME\";; \"~/\"*) d=\"$HOME/${{d#??}}\";; esac; ",
            "cd -- \"$HOME\" 2>/dev/null; ",
            "if CDPATH= cd -- \"$d\" 2>/dev/null; then printf 'ok\\t%s\\n' \"$PWD\"; ",
            "else printf 'missing\\n'; fi"
        ),
        shell_quote(raw)
    )
}

fn parse_resolve_dir(raw: &str, out: &str) -> Result<String> {
    let line = out.lines().next().unwrap_or_default();
    match line.split_once('\t') {
        Some(("ok", path)) if path.starts_with('/') => Ok(path.to_owned()),
        _ if line.trim() == "missing" => Err(Error::NoSuchDirectory {
            path: raw.trim().to_owned(),
        }),
        _ => Err(Error::Protocol(format!(
            "resolve_dir: unexpected output {out:?}"
        ))),
    }
}

/// Single-quote a string for a POSIX shell.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Git facts about one working directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirInfo {
    pub dir: String,
    /// Current branch, or the short commit when detached.
    pub branch: Option<String>,
    /// Repository name: from the `origin` URL, else the top-level directory.
    pub repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirReport {
    pub home: String,
    pub dirs: Vec<DirInfo>,
}

/// POSIX shell script: `$HOME`, then one `index<TAB>branch<TAB>toplevel<TAB>origin`
/// line per dir.
fn dir_info_script(dirs: &[String]) -> String {
    let list: Vec<String> = dirs.iter().map(|d| shell_quote(d)).collect();
    format!(
        concat!(
            "export GIT_OPTIONAL_LOCKS=0; printf '%s\\n' \"$HOME\"; i=0; ",
            "for d in {}; do ",
            "b=$(git -C \"$d\" symbolic-ref --short -q HEAD 2>/dev/null || git -C \"$d\" rev-parse --short HEAD 2>/dev/null); ",
            "t=$(git -C \"$d\" rev-parse --show-toplevel 2>/dev/null); ",
            "u=$(git -C \"$d\" config --get remote.origin.url 2>/dev/null); ",
            "printf '%s\\t%s\\t%s\\t%s\\n' \"$i\" \"$b\" \"$t\" \"$u\"; i=$((i + 1)); ",
            "done"
        ),
        if list.is_empty() {
            "''".to_owned()
        } else {
            list.join(" ")
        }
    )
}

/// Lines are matched to `dirs` by their index; missing or garbled lines
/// leave a directory without facts.
fn parse_dir_info(dirs: &[String], out: &str) -> DirReport {
    let mut lines = out.lines();
    let home = lines.next().unwrap_or_default().trim().to_owned();
    let mut infos: Vec<DirInfo> = dirs
        .iter()
        .map(|dir| DirInfo {
            dir: dir.clone(),
            ..DirInfo::default()
        })
        .collect();
    for line in lines {
        let mut cols = line.split('\t').map(str::trim);
        let Some(info) = cols
            .next()
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| infos.get_mut(i))
        else {
            continue;
        };
        let some = |s: Option<&str>| s.filter(|v| !v.is_empty()).map(str::to_owned);
        let branch = some(cols.next());
        let top = some(cols.next());
        let origin = some(cols.next());
        info.repo = origin
            .as_deref()
            .and_then(repo_name)
            .or_else(|| top.as_deref().and_then(repo_name));
        info.branch = branch;
    }
    DirReport { home, dirs: infos }
}

/// `git@github.com:me/app.git`, `https://…/app/` or `/src/app` → `app`.
fn repo_name(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches('/');
    let last = s.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_owned())
}

/// `<unix-millis>-<random>-<sanitized name>`, keeping only `[A-Za-z0-9._-]`.
fn unique_upload_name(file_name: &str) -> String {
    let base: String = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = base.trim_start_matches('.');
    let base = if base.is_empty() { "upload" } else { base };
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    // Two uploads in the same millisecond must not collide.
    let mut random = [0u8; 4];
    let _ = getrandom::fill(&mut random);
    format!("{millis}-{:08x}-{base}", u32::from_le_bytes(random))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dir_info() {
        let dirs = vec!["/a".to_owned(), "/b".to_owned(), "/c".to_owned()];
        // Login noise before the mark; dir 1 missing, a stray line in between.
        let out = concat!(
            "Welcome to host!\n1\tnoise\t/x\t\n",
            "@@missionctrl-output@@\n",
            "/home/me\n0\tmain\t/src/app\tgit@github.com:me/app-x.git\n",
            "stray output\n2\tabc123\t/src/tool\t\n"
        );
        let r = parse_dir_info(&dirs, &after_mark(out).unwrap());
        assert_eq!(r.home, "/home/me");
        assert_eq!(r.dirs[0].branch.as_deref(), Some("main"));
        assert_eq!(r.dirs[0].repo.as_deref(), Some("app-x"));
        assert_eq!(
            r.dirs[1],
            DirInfo {
                dir: "/b".into(),
                branch: None,
                repo: None
            }
        );
        assert_eq!(r.dirs[2].repo.as_deref(), Some("tool"));
        assert_eq!(repo_name("https://host/x/y/"), Some("y".into()));
    }

    #[test]
    fn parses_remote_env_after_noise() {
        let out = "motd without newline@@missionctrl-output@@\n/home/me\n\n";
        assert_eq!(
            parse_remote_env(&after_mark(out).unwrap()).unwrap(),
            RemoteEnv {
                home: "/home/me".into(),
                xdg_config_home: None
            }
        );
        assert!(after_mark("no mark here\n/home/me\n").is_err());
    }

    #[tokio::test]
    async fn exec_sh_skips_to_output() {
        let out = Transport::Local(LocalTransport)
            .exec_sh("echo it\\'s; printf 'a\\tb\\n'")
            .await
            .unwrap();
        assert_eq!(out, "it's\na\tb\n");
    }

    #[tokio::test]
    async fn dir_info_runs_locally() {
        let here = env!("CARGO_MANIFEST_DIR").to_owned();
        let r = Transport::Local(LocalTransport)
            .dir_info(&[here, "/nonexistent-dir".to_owned()])
            .await
            .unwrap();
        assert!(r.home.starts_with('/'));
        assert_eq!(r.dirs.len(), 2);
        assert!(r.dirs[0].branch.is_some());
        assert_eq!(r.dirs[1].branch, None);
    }

    #[tokio::test]
    async fn resolve_dir_runs_locally() {
        let t = Transport::Local(LocalTransport);
        let home = std::env::var("HOME").unwrap();
        let home = home.trim_end_matches('/');
        assert_eq!(t.resolve_dir("").await.unwrap(), home);
        assert_eq!(t.resolve_dir(" ~ ").await.unwrap(), home);
        assert_eq!(t.resolve_dir("/tmp").await.unwrap(), "/tmp");
        assert_eq!(t.resolve_dir("~/").await.unwrap(), home);
        let here = env!("CARGO_MANIFEST_DIR");
        assert_eq!(t.resolve_dir(here).await.unwrap(), here);
        if let Some(rest) = here.strip_prefix(&format!("{home}/")) {
            assert_eq!(t.resolve_dir(&format!("~/{rest}")).await.unwrap(), here);
            assert_eq!(t.resolve_dir(rest).await.unwrap(), here);
        }
        match t.resolve_dir("~/nonexistent-dir 'x'").await {
            Err(Error::NoSuchDirectory { path }) => assert_eq!(path, "~/nonexistent-dir 'x'"),
            other => panic!("expected NoSuchDirectory, got {other:?}"),
        }
    }

    #[test]
    fn resolve_dir_output() {
        assert_eq!(
            parse_resolve_dir("~", "ok\t/home/me\n").unwrap(),
            "/home/me"
        );
        assert!(matches!(
            parse_resolve_dir("x", "missing\n"),
            Err(Error::NoSuchDirectory { .. })
        ));
        assert!(matches!(
            parse_resolve_dir("x", ""),
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn plan_paths_are_only_plans() {
        let home = "/home/me";
        assert_eq!(
            plan_path(home, "~/.claude/plans/happy-crafting-lecun.md").as_deref(),
            Some("/home/me/.claude/plans/happy-crafting-lecun.md")
        );
        assert_eq!(
            plan_path(home, "/srv/x/.claude/plans/a_b-1.md").as_deref(),
            Some("/srv/x/.claude/plans/a_b-1.md")
        );
        for bad in [
            "~/.ssh/id_ed25519",
            "~/.claude/plans/../../.ssh/k.md",
            "~/.claude/plans/a.md;rm",
            "~/.claude/plans/a b.md",
            "~/.claude/plans/a.txt",
            ".claude/plans/a.md",
            "~/plans.md",
            "~/.claude/plans/~x.md",
        ] {
            assert_eq!(plan_path(home, bad), None, "{bad}");
        }
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn sanitizes_upload_names() {
        let name = unique_upload_name("../../etc/pass wd.png");
        assert!(name.ends_with("-pass_wd.png"), "{name}");
        assert!(unique_upload_name(".hidden").ends_with("-hidden"));
        assert!(unique_upload_name("").ends_with("-upload"));
        assert!(!unique_upload_name("a/../b").contains('/'));
    }
}

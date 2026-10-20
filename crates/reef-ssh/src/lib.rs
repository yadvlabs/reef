//! Git over SSH. Authentication is by public key (looked up by SHA-256 fingerprint);
//! the only commands accepted are `git-upload-pack` and `git-receive-pack`, which are
//! spawned as `git upload-pack`/`git receive-pack` with the channel piped to them.

use reef_api::AppState;
use reef_core::Error;
use reef_db::accounts::Account;
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::smart::{RefUpdate, Service, parse_ref_updates};
use russh::keys::{HashAlg, PrivateKey};
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct SshServer {
    state: AppState,
}

impl SshServer {
    pub fn new(state: AppState) -> SshServer {
        SshServer { state }
    }

    /// Load or create the host key at `<data_dir>/ssh/host_ed25519`.
    pub fn host_key(data_dir: &Path) -> Result<PrivateKey, Error> {
        let dir = data_dir.join("ssh");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("host_ed25519");
        if !path.exists() {
            let out = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", "reef host key", "-f"])
                .arg(&path)
                .stdin(Stdio::null())
                .output()?;
            if !out.status.success() {
                return Err(Error::msg(format!("ssh-keygen failed: {}", String::from_utf8_lossy(&out.stderr))));
            }
        }
        let pem = std::fs::read(&path)?;
        PrivateKey::from_openssh(&pem).map_err(|e| Error::msg(format!("host key {}: {e}", path.display())))
    }

    /// Serve `listener` (already bound, so readiness can be reported once) until `shutdown`
    /// resolves; open sessions are then told to disconnect.
    pub async fn run(
        self,
        listener: tokio::net::TcpListener,
        data_dir: &Path,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), Error> {
        let key = Self::host_key(data_dir)?;
        let listen = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
        tracing::info!(fingerprint = %key.public_key().fingerprint(HashAlg::Sha256), listen = %listen, "ssh listening");
        let mut methods = russh::MethodSet::empty();
        methods.push(russh::MethodKind::PublicKey);
        let config = russh::server::Config {
            keys: vec![key],
            methods,
            auth_rejection_time: std::time::Duration::from_secs(1),
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };
        let mut server = self;
        let running = server.run_on_socket(Arc::new(config), &listener);
        let handle = running.handle();
        tokio::spawn(async move {
            shutdown.await;
            handle.shutdown("server shutting down".into());
        });
        running.await.map_err(Error::internal)
    }
}

impl Server for SshServer {
    type Handler = Conn;
    fn new_client(&mut self, peer: Option<std::net::SocketAddr>) -> Conn {
        Conn { state: self.state.clone(), peer, user: None, channels: HashMap::new() }
    }
    fn handle_session_error(&mut self, e: <Self::Handler as Handler>::Error) {
        tracing::debug!(error = %e, "ssh session ended with error");
    }
}

pub struct Conn {
    state: AppState,
    peer: Option<std::net::SocketAddr>,
    user: Option<Account>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

#[derive(Debug)]
pub struct ConnError(String);

impl std::fmt::Display for ConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ConnError {}
impl From<russh::Error> for ConnError {
    fn from(e: russh::Error) -> Self {
        ConnError(e.to_string())
    }
}

/// Parse `git-upload-pack 'owner/repo.git'` (and the `git upload-pack` spelling).
fn parse_command(cmd: &str) -> Option<(Service, String, String)> {
    let cmd = cmd.trim();
    let (service, rest) = if let Some(r) = cmd.strip_prefix("git-upload-pack") {
        (Service::UploadPack, r)
    } else if let Some(r) = cmd.strip_prefix("git-receive-pack") {
        (Service::ReceivePack, r)
    } else if let Some(r) = cmd.strip_prefix("git upload-pack") {
        (Service::UploadPack, r)
    } else if let Some(r) = cmd.strip_prefix("git receive-pack") {
        (Service::ReceivePack, r)
    } else {
        return None;
    };
    let path = rest.trim().trim_matches('\'').trim_matches('"').trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    // `.wiki` stays on the repository name; the service resolves it.
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((service, owner.to_string(), repo.to_string()))
}

/// Parse `git-lfs-authenticate owner/repo.git download|upload`.
fn parse_lfs_authenticate(cmd: &str) -> Option<(String, String, String)> {
    let rest = cmd.trim().strip_prefix("git-lfs-authenticate")?;
    let mut parts = rest.split_whitespace();
    let path = parts.next()?.trim_matches('\'').trim_matches('"').trim_start_matches('/');
    let op = parts.next()?.trim_matches('\'').trim_matches('"');
    if op != "download" && op != "upload" {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string(), op.to_string()))
}

/// Answer `git-lfs-authenticate`: a short-lived bearer token for the repository's LFS endpoint.
async fn lfs_authenticate(state: &AppState, channel: Channel<Msg>, user: &Account, owner: &str, repo: &str, op: &str) -> Result<(), Error> {
    let rw: RepoWithOwner = match state.db.repo_by_path(owner, repo).await {
        Ok(rw) => rw,
        Err(Error::NotFound) => {
            deny(&channel, "repository not found").await;
            return Ok(());
        }
        Err(e) => {
            deny(&channel, "internal error").await;
            return Err(e);
        }
    };
    let perm = state.db.permission(Some(user), &rw).await?;
    let needed = if op == "upload" { Permission::Write } else { Permission::Read };
    if perm < needed {
        deny(&channel, if perm.can_read() { "you do not have push access to this repository" } else { "repository not found" }).await;
        return Ok(());
    }
    if op == "upload" && rw.repo.archived != 0 {
        deny(&channel, "this repository is archived and read-only").await;
        return Ok(());
    }
    match reef_api::lfs::ssh_authenticate(state, user, &rw).await {
        Ok(v) => {
            let mut body = v.to_string();
            body.push('\n');
            let _ = channel.data_bytes(bytes::Bytes::from(body)).await;
            let _ = channel.exit_status(0).await;
            let _ = channel.eof().await;
            let _ = channel.close().await;
            Ok(())
        }
        Err(e) => {
            deny(&channel, "could not mint an LFS token").await;
            Err(e)
        }
    }
}

impl Handler for Conn {
    type Error = ConnError;

    async fn auth_publickey(&mut self, user: &str, key: &russh::keys::PublicKey) -> Result<Auth, Self::Error> {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        match self.state.db.ssh_key_by_fingerprint(&fp).await {
            Ok(k) => match self.state.db.account_by_id(reef_core::Id(k.user_id)).await {
                Ok(a) if a.suspended == 0 => {
                    tracing::info!(login = %a.login, ssh_user = user, peer = ?self.peer, "ssh auth ok");
                    self.user = Some(a);
                    Ok(Auth::Accept)
                }
                _ => Ok(Auth::reject()),
            },
            Err(_) => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(&mut self, id: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).into_owned();
        let Some(channel) = self.channels.remove(&id) else { return Ok(()) };
        let Some(user) = self.user.clone() else {
            session.channel_failure(id)?;
            return Ok(());
        };
        if let Some((owner, repo, op)) = parse_lfs_authenticate(&cmd) {
            session.channel_success(id)?;
            let state = self.state.clone();
            tokio::spawn(async move {
                if let Err(e) = lfs_authenticate(&state, channel, &user, &owner, &repo, &op).await {
                    tracing::info!(error = %e, login = %user.login, "git-lfs-authenticate failed");
                }
            });
            return Ok(());
        }
        let Some((service, owner, repo)) = parse_command(&cmd) else {
            let _ = channel.data_bytes(bytes::Bytes::from_static(b"")).await;
            let _ = channel
                .extended_data(1, "reef: only git-upload-pack, git-receive-pack and git-lfs-authenticate are allowed over SSH\n".as_bytes())
                .await;
            let _ = channel.exit_status(1).await;
            let _ = channel.eof().await;
            let _ = channel.close().await;
            session.channel_success(id)?;
            return Ok(());
        };
        session.channel_success(id)?;
        let state = self.state.clone();
        tokio::spawn(async move {
            let msg = run_service(&state, channel, &user, service, &owner, &repo).await.err();
            if let Some(e) = msg {
                tracing::info!(error = %e, login = %user.login, "ssh git service ended");
            }
        });
        Ok(())
    }
}

async fn deny(channel: &Channel<Msg>, msg: &str) {
    let _ = channel.extended_data(1, format!("reef: {msg}\n").as_bytes()).await;
    let _ = channel.exit_status(1).await;
    let _ = channel.eof().await;
    let _ = channel.close().await;
}

async fn run_service(
    state: &AppState,
    mut channel: Channel<Msg>,
    user: &Account,
    service: Service,
    owner: &str,
    repo: &str,
) -> Result<(), Error> {
    let (repo, wiki) = reef_api::githttp::split_wiki(repo);
    let rw: RepoWithOwner = match state.db.repo_by_path(owner, repo).await {
        Ok(rw) => rw,
        Err(Error::NotFound) => {
            deny(&channel, "repository not found").await;
            return Ok(());
        }
        Err(e) => {
            deny(&channel, "internal error").await;
            return Err(e);
        }
    };
    let perm = state.db.permission(Some(user), &rw).await?;
    let needed = match service {
        Service::UploadPack => Permission::Read,
        Service::ReceivePack => Permission::Write,
    };
    if perm < needed {
        deny(&channel, if perm.can_read() { "you do not have push access to this repository" } else { "repository not found" }).await;
        return Ok(());
    }
    if service == Service::ReceivePack && rw.repo.archived != 0 {
        deny(&channel, "this repository is archived and read-only").await;
        return Ok(());
    }
    let path = if wiki {
        match state.store.ensure_wiki(rw.repo.id(), "main").await {
            Ok(p) => p,
            Err(e) => {
                deny(&channel, "wiki unavailable").await;
                return Err(e);
            }
        }
    } else {
        state.store.path(rw.repo.id())
    };
    let sub = match service {
        Service::UploadPack => "upload-pack",
        Service::ReceivePack => "receive-pack",
    };
    let mut cmd = tokio::process::Command::new(state.store.git_binary());
    if let Some(d) = state.store.hooks_dir() {
        cmd.arg("-c").arg(format!("core.hooksPath={}", d.display()));
    }
    cmd.arg(sub).arg(&path);
    cmd.env_clear();
    for (k, v) in reef_git::smart::Env::base_env() {
        cmd.env(k, v);
    }
    cmd.env("GIT_PROTOCOL", "version=2");
    for (k, v) in reef_api::githttp::hook_env(state, &rw, Some(user)) {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let mut out_w = channel.make_writer();
    let mut err_w = channel.make_writer_ext(Some(1));
    let capture = service == Service::ReceivePack;
    // Pushes are capped at `[limits] max_push_bytes` like over HTTP: the bytes going into
    // receive-pack are counted, and crossing the cap kills git and refuses the push.
    let cap = if capture { state.config.limits.max_push_bytes } else { u64::MAX };
    let (prefix, exceeded) = {
        let mut reader = channel.make_reader();
        let child = &mut child;

        // client -> git stdin, capturing the ref-update commands of a push.
        let pump_in = async {
            let r = pump_stdin(&mut reader, stdin, cap, capture).await;
            if r.1 {
                let _ = child.start_kill();
            }
            r
        };
        let pump_out = async {
            let _ = tokio::io::copy(&mut stdout, &mut out_w).await;
            let _ = out_w.shutdown().await;
        };
        let pump_err = async {
            let _ = tokio::io::copy(&mut stderr, &mut err_w).await;
        };
        let (result, _, _) = tokio::join!(pump_in, pump_out, pump_err);
        result
    };
    let status = child.wait().await?;
    if exceeded {
        deny(&channel, &reef_git::smart::push_too_large(cap)).await;
        return Err(Error::msg(format!("push by {} to {} refused: {}", user.login, rw.full_name(), reef_git::smart::push_too_large(cap))));
    }
    let code = status.code().unwrap_or(1) as u32;
    let _ = channel.exit_status(code).await;
    let _ = channel.eof().await;
    let _ = channel.close().await;

    if capture && status.success() && !wiki {
        let updates: Vec<RefUpdate> = parse_ref_updates(&prefix);
        reef_api::githttp::after_push(state, &rw, Some(user), &updates).await?;
    }
    Ok(())
}

/// Copy the client's bytes into git's stdin until EOF, keeping the first MiB when `capture`
/// (the ref-update commands of a push). Stops as soon as more than `cap` bytes arrived,
/// without handing on the excess, and reports it: `(prefix, exceeded)`. Closes stdin at the
/// end so git sees EOF.
async fn pump_stdin<R, W>(reader: &mut R, mut stdin: W, cap: u64, capture: bool) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut prefix: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 32 * 1024];
    let mut total: u64 = 0;
    let mut exceeded = false;
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        total += n as u64;
        if total > cap {
            exceeded = true;
            break;
        }
        if capture && prefix.len() < 1024 * 1024 {
            prefix.extend_from_slice(&buf[..n]);
        }
        if stdin.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = stdin.shutdown().await;
    drop(stdin);
    (prefix, exceeded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_commands() {
        let (s, o, r) = parse_command("git-upload-pack 'yadvlabs/reef.git'").unwrap();
        assert_eq!(s, Service::UploadPack);
        assert_eq!((o.as_str(), r.as_str()), ("yadvlabs", "reef"));
        let (s, _, r) = parse_command("git receive-pack /yadvlabs/reef").unwrap();
        assert_eq!(s, Service::ReceivePack);
        assert_eq!(r, "reef");
        assert!(parse_command("ls -la").is_none());
        assert!(parse_command("git-upload-pack 'a/b/c.git'").is_none());
    }

    #[tokio::test]
    async fn pushes_over_the_cap_are_cut_off() {
        let data = vec![7u8; 100_000];
        let mut sink = Vec::new();
        let (prefix, exceeded) = pump_stdin(&mut &data[..], &mut sink, 100_000, true).await;
        assert!(!exceeded);
        assert_eq!((sink.len(), prefix.len()), (100_000, 100_000), "exactly the cap passes");
        let mut sink = Vec::new();
        let (_, exceeded) = pump_stdin(&mut &data[..], &mut sink, 50_000, true).await;
        assert!(exceeded, "one byte over is refused");
        assert!(sink.len() <= 50_000, "nothing past the cap reaches git ({} bytes)", sink.len());
        let (_, exceeded) = pump_stdin(&mut &data[..], &mut Vec::new(), u64::MAX, false).await;
        assert!(!exceeded, "fetches are not capped");
    }
}

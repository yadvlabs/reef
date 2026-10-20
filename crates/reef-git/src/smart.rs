//! The smart HTTP and SSH transports, by spawning `git upload-pack` / `git receive-pack`.
//!
//! The handler streams the client's request body into the child's stdin and the child's
//! stdout back to the client. The child is killed if the client goes away.

use bytes::Bytes;
use futures::Stream;
use reef_core::{Error, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::io::ReaderStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    UploadPack,
    ReceivePack,
}

impl Service {
    pub fn parse(s: &str) -> Option<Service> {
        match s {
            "git-upload-pack" => Some(Service::UploadPack),
            "git-receive-pack" => Some(Service::ReceivePack),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Service::UploadPack => "git-upload-pack",
            Service::ReceivePack => "git-receive-pack",
        }
    }
    fn subcommand(self) -> &'static str {
        match self {
            Service::UploadPack => "upload-pack",
            Service::ReceivePack => "receive-pack",
        }
    }
}

/// Environment for the spawned git process. `protocol` is the client's `Git-Protocol` header
/// (e.g. `version=2`), validated before use.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub protocol: Option<String>,
    /// Extra `KEY=VALUE` pairs, e.g. identity for hooks.
    pub extra: Vec<(String, String)>,
    /// `core.hooksPath` for the spawned process.
    pub hooks_dir: Option<std::path::PathBuf>,
}

impl Env {
    /// Environment every spawned git process gets in addition to `extra`.
    pub fn base_env() -> Vec<(String, String)> {
        let mut v = vec![("PATH".to_string(), std::env::var("PATH").unwrap_or_default()), ("LANG".to_string(), "C.UTF-8".to_string())];
        if let Ok(exe) = std::env::current_exe() {
            v.push(("REEF_BIN".to_string(), exe.to_string_lossy().into_owned()));
        }
        v
    }
}

fn base_command(git: &Path, repo: &Path, service: Service, env: &Env) -> Command {
    let mut cmd = Command::new(git);
    // Hard limits and safety knobs that apply to every server-side invocation.
    match &env.hooks_dir {
        Some(d) => cmd.arg("-c").arg(format!("core.hooksPath={}", d.display())),
        None => cmd.arg("-c").arg("core.hooksPath=/nonexistent"),
    };
    cmd.arg("-c").arg("receive.advertisePushOptions=true");
    cmd.arg("-c").arg("uploadpack.allowFilter=true");
    cmd.arg("-c").arg("uploadpack.allowAnySHA1InWant=true");
    cmd.arg(service.subcommand());
    cmd.arg("--stateless-rpc");
    cmd.env_clear();
    for (k, v) in Env::base_env() {
        cmd.env(k, v);
    }
    cmd.env("GIT_DIR", repo);
    if let Some(p) = &env.protocol
        && is_valid_protocol(p)
    {
        cmd.env("GIT_PROTOCOL", p);
    }
    for (k, v) in &env.extra {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd
}

/// `GIT_PROTOCOL` values are `key[=value]` pairs separated by colons.
fn is_valid_protocol(p: &str) -> bool {
    !p.is_empty()
        && p.len() < 100
        && p.split(':').all(|kv| !kv.is_empty() && kv.chars().all(|c| c.is_ascii_alphanumeric() || c == '=' || c == '.' || c == '-'))
}

/// `GET /info/refs?service=...`: the reference advertisement. Returns the raw payload,
/// which for protocol v1 the HTTP handler prefixes with the `# service=` pkt-line.
pub async fn advertise_refs(git: &Path, repo: &Path, service: Service, env: &Env) -> Result<Vec<u8>> {
    let mut cmd = base_command(git, repo, service, env);
    cmd.arg("--advertise-refs");
    cmd.arg(repo);
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(Error::msg(format!("{} --advertise-refs failed: {}", service.name(), String::from_utf8_lossy(&out.stderr))));
    }
    Ok(out.stdout)
}

/// The outcome of a service call, resolved once the child exits.
pub struct Exit {
    pub success: bool,
    pub stderr: String,
}

/// `POST /git-upload-pack` and `POST /git-receive-pack`. Returns a stream of the child's
/// stdout and a future that resolves with the exit status once the client has been served.
pub fn serve<B>(
    git: &Path,
    repo: &Path,
    service: Service,
    env: &Env,
    body: B,
) -> Result<(impl Stream<Item = std::io::Result<Bytes>> + Send + 'static, impl std::future::Future<Output = Exit> + Send + 'static)>
where
    B: AsyncRead + Send + Unpin + 'static,
{
    let mut cmd = base_command(git, repo, service, env);
    cmd.arg(repo);
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().ok_or_else(|| Error::msg("no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| Error::msg("no stdout"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| Error::msg("no stderr"))?;

    // Pump the request body into git. Errors here (client hung up) simply end the copy;
    // git sees EOF and exits.
    let pump = tokio::spawn(async move {
        let mut body = body;
        let _ = tokio::io::copy(&mut body, &mut stdin).await;
        let _ = stdin.shutdown().await;
    });

    let done = async move {
        let mut err = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stderr, &mut err).await;
        let status = child.wait().await;
        pump.abort();
        Exit { success: status.map(|s| s.success()).unwrap_or(false), stderr: String::from_utf8_lossy(&err).into_owned() }
    };

    Ok((ReaderStream::with_capacity(stdout, 64 * 1024), done))
}

/// Encode a pkt-line.
pub fn pkt_line(s: &str) -> Vec<u8> {
    let mut v = format!("{:04x}", s.len() + 4).into_bytes();
    v.extend_from_slice(s.as_bytes());
    v
}

/// The pkt-line flush packet.
pub const FLUSH: &[u8] = b"0000";

/// Parse the `old new refname` lines a receive-pack client sends, for post-push processing.
/// The body of a receive-pack request starts with these pkt-lines before the pack data.
pub fn parse_ref_updates(body_prefix: &[u8]) -> Vec<RefUpdate> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= body_prefix.len() {
        let len = match std::str::from_utf8(&body_prefix[i..i + 4]).ok().and_then(|h| usize::from_str_radix(h, 16).ok()) {
            Some(l) => l,
            None => break,
        };
        if len == 0 {
            break;
        }
        if len < 4 || i + len > body_prefix.len() {
            break;
        }
        let line = &body_prefix[i + 4..i + len];
        let line = std::str::from_utf8(line).unwrap_or("");
        let line = line.split('\0').next().unwrap_or("").trim_end_matches('\n');
        let mut parts = line.split(' ');
        if let (Some(old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next())
            && old.len() == 40
            && new.len() == 40
        {
            out.push(RefUpdate { old: old.to_string(), new: new.to_string(), name: name.to_string() });
        }
        i += len;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub old: String,
    pub new: String,
    pub name: String,
}

impl RefUpdate {
    pub const ZERO: &'static str = "0000000000000000000000000000000000000000";
    pub fn is_create(&self) -> bool {
        self.old == Self::ZERO
    }
    pub fn is_delete(&self) -> bool {
        self.new == Self::ZERO
    }
    pub fn branch(&self) -> Option<&str> {
        self.name.strip_prefix("refs/heads/")
    }
    pub fn tag(&self) -> Option<&str> {
        self.name.strip_prefix("refs/tags/")
    }
}

/// The message a push over `[limits] max_push_bytes` is refused with (HTTP body, SSH stderr).
pub fn push_too_large(cap: u64) -> String {
    format!("push exceeds the maximum size of {cap} bytes")
}

/// Counts the bytes of a push as they reach `git receive-pack` and fails the read once more
/// than `cap` have passed; the bytes beyond the cap are never handed on. Wrap the stream git
/// actually reads (after any `Content-Encoding` is undone), so a small compressed body cannot
/// expand past the cap. [`CappedRead::exceeded`] tells the caller afterwards whether the push
/// was cut, so it records nothing for it.
pub struct CappedRead<R> {
    inner: R,
    cap: u64,
    seen: u64,
    exceeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<R> CappedRead<R> {
    pub fn new(inner: R, cap: u64) -> CappedRead<R> {
        CappedRead { inner, cap, seen: 0, exceeded: Default::default() }
    }

    /// Set once the cap was crossed; stays readable after the reader is moved into git's pump.
    pub fn exceeded(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.exceeded.clone()
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CappedRead<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let too_large = |cap| std::io::Error::other(push_too_large(cap));
        if this.seen > this.cap {
            return std::task::Poll::Ready(Err(too_large(this.cap)));
        }
        let before = buf.filled().len();
        std::task::ready!(std::pin::Pin::new(&mut this.inner).poll_read(cx, buf))?;
        this.seen += (buf.filled().len() - before) as u64;
        if this.seen > this.cap {
            buf.set_filled(before);
            this.exceeded.store(true, std::sync::atomic::Ordering::SeqCst);
            return std::task::Poll::Ready(Err(too_large(this.cap)));
        }
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ref_update_lines() {
        let old = "0000000000000000000000000000000000000000";
        let new = "1111111111111111111111111111111111111111";
        let line = format!("{old} {new} refs/heads/main\0report-status side-band-64k\n");
        let mut body = pkt_line(&line);
        body.extend_from_slice(FLUSH);
        body.extend_from_slice(b"PACK....");
        let ups = parse_ref_updates(&body);
        assert_eq!(ups.len(), 1);
        assert!(ups[0].is_create());
        assert_eq!(ups[0].branch(), Some("main"));
    }

    #[tokio::test]
    async fn capped_reads_stop_at_the_limit() {
        use tokio::io::AsyncReadExt;
        let mut ok = CappedRead::new(&b"0123456789"[..], 10);
        let mut out = Vec::new();
        ok.read_to_end(&mut out).await.unwrap();
        assert_eq!(out.len(), 10, "exactly the cap passes");
        assert!(!ok.exceeded().load(std::sync::atomic::Ordering::SeqCst));
        let mut over = CappedRead::new(&b"0123456789A"[..], 10);
        let flag = over.exceeded();
        let mut out = Vec::new();
        let err = over.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(err.to_string(), push_too_large(10));
        assert!(out.len() <= 10, "nothing past the cap is handed on");
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }
}

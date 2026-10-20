//! Git smart HTTP: `GET .../info/refs?service=`, `POST .../git-upload-pack`,
//! `POST .../git-receive-pack`. Authentication is HTTP Basic with a password or token, or
//! a bearer token. Anonymous reads are allowed on public repositories.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiError;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::TryStreamExt;
use reef_core::Error;
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::smart::{self, Env, Service};
use tokio_util::io::StreamReader;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/info/refs", get(info_refs))
        .route("/{owner}/{repo}/git-upload-pack", post(upload_pack))
        .route("/{owner}/{repo}/git-receive-pack", post(receive_pack))
        .route("/{owner}/{repo}/HEAD", get(head))
}

fn unauthorized() -> Response {
    let mut r = (StatusCode::UNAUTHORIZED, "Authentication required\n").into_response();
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"Reef\""));
    r
}

/// `owner/repo.wiki(.git)` addresses the wiki repository. Returns (repo name, is_wiki).
pub fn split_wiki(repo: &str) -> (&str, bool) {
    let base = repo.strip_suffix(".git").unwrap_or(repo);
    match base.strip_suffix(".wiki") {
        Some(r) => (r, true),
        None => (base, false),
    }
}

/// Load the repository and check `service` is allowed for the caller.
async fn authorize(s: &AppState, auth: &Auth, owner: &str, repo: &str, service: Service) -> Result<RepoWithOwner, Response> {
    let (repo, _) = split_wiki(repo);
    let rw = match s.db.repo_by_path(owner, repo).await {
        Ok(rw) => rw,
        Err(Error::NotFound) => {
            // Do not reveal whether a private repository exists: ask for credentials first.
            return Err(if auth.user.is_none() {
                unauthorized()
            } else {
                (StatusCode::NOT_FOUND, "Repository not found\n").into_response()
            });
        }
        Err(e) => return Err(ApiError(e).into_response()),
    };
    let perm = match s.db.permission(auth.user.as_ref(), &rw).await {
        Ok(p) => p,
        Err(e) => return Err(ApiError(e).into_response()),
    };
    let needed = match service {
        Service::UploadPack => Permission::Read,
        Service::ReceivePack => Permission::Write,
    };
    if perm < needed {
        return Err(if auth.user.is_none() { unauthorized() } else { (StatusCode::FORBIDDEN, "Permission denied\n").into_response() });
    }
    if service == Service::ReceivePack && rw.repo.archived != 0 {
        return Err((StatusCode::FORBIDDEN, "Repository is archived\n").into_response());
    }
    if service == Service::ReceivePack {
        crate::mirror::deny_push_if_mirror(s, &rw).await?;
    }
    if service == Service::ReceivePack && !auth.has_scope("repo") {
        return Err((StatusCode::FORBIDDEN, "Token lacks the repo scope\n").into_response());
    }
    Ok(rw)
}

fn env_for(s: &AppState, headers: &HeaderMap, auth: &Auth, rw: &RepoWithOwner) -> Env {
    Env {
        protocol: headers.get("git-protocol").and_then(|v| v.to_str().ok()).map(String::from),
        extra: hook_env(s, rw, auth.user.as_ref()),
        hooks_dir: s.store.hooks_dir().map(|p| p.to_path_buf()),
    }
}

/// Environment handed to spawned git processes (and so to hooks): who is pushing where.
pub fn hook_env(s: &AppState, rw: &RepoWithOwner, user: Option<&reef_db::accounts::Account>) -> Vec<(String, String)> {
    let mut v = vec![
        ("REEF_REPO_ID".to_string(), rw.repo.id.to_string()),
        ("REEF_REPO_FULL_NAME".to_string(), rw.full_name()),
        ("REEF_DATABASE_URL".to_string(), s.config.database_url()),
        ("REEF_DATA_DIR".to_string(), s.config.data_dir.to_string_lossy().into_owned()),
        ("REEF_GIT_BINARY".to_string(), s.store.git_binary().to_string_lossy().into_owned()),
    ];
    if let Some(u) = user {
        v.push(("REEF_PUSHER_ID".into(), u.id.to_string()));
        v.push(("REEF_PUSHER_LOGIN".into(), u.login.clone()));
        v.push(("REEF_PUSHER_ADMIN".into(), if u.is_admin() { "1".into() } else { "0".into() }));
    }
    v
}

#[derive(serde::Deserialize)]
pub struct InfoRefsQuery {
    service: Option<String>,
}

pub async fn info_refs(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<InfoRefsQuery>,
) -> Response {
    let Some(service) = q.service.as_deref().and_then(Service::parse) else {
        return (StatusCode::FORBIDDEN, "Dumb HTTP transport is not supported. Use git 1.6.6 or later.\n").into_response();
    };
    let rw = match authorize(&s, &auth, &owner, &repo, service).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let env = env_for(&s, &headers, &auth, &rw);
    let path = match repo_path(&s, &rw, &repo).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let adv = match smart::advertise_refs(s.store.git_binary(), &path, service, &env).await {
        Ok(a) => a,
        Err(e) => return ApiError(e).into_response(),
    };
    // Protocol v2 advertisements carry no service header; v0/v1 do.
    let is_v2 = env.protocol.as_deref().map(|p| p.contains("version=2")).unwrap_or(false);
    let mut body = Vec::with_capacity(adv.len() + 64);
    if !is_v2 {
        body.extend_from_slice(&smart::pkt_line(&format!("# service={}\n", service.name())));
        body.extend_from_slice(smart::FLUSH);
    }
    body.extend_from_slice(&adv);
    (
        [
            (header::CONTENT_TYPE, format!("application/x-{}-advertisement", service.name())),
            (header::CACHE_CONTROL, "no-cache, max-age=0, must-revalidate".to_string()),
        ],
        body,
    )
        .into_response()
}

pub async fn upload_pack(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>, req: Request) -> Response {
    service(s, auth, owner, repo, Service::UploadPack, req).await
}

pub async fn receive_pack(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>, req: Request) -> Response {
    service(s, auth, owner, repo, Service::ReceivePack, req).await
}

async fn service(s: AppState, auth: Auth, owner: String, repo: String, service: Service, req: Request) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, service).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let headers = req.headers().clone();
    let env = env_for(&s, &headers, &auth, &rw);
    let path = match repo_path(&s, &rw, &repo).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let is_wiki = split_wiki(&repo).1;
    let gzip = headers.get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).map(|v| v.contains("gzip")).unwrap_or(false);
    // Pushes are capped at `[limits] max_push_bytes`: a declared length above it is refused up
    // front, and the stream git reads is counted after any gzip is undone, so neither a chunked
    // body nor a small compressed one can carry more; the push is cut off where it crosses.
    let max_push = s.config.limits.max_push_bytes;
    if service == Service::ReceivePack
        && let Some(len) = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok())
        && len > max_push
    {
        return (StatusCode::PAYLOAD_TOO_LARGE, format!("{}\n", smart::push_too_large(max_push))).into_response();
    }

    // For receive-pack we peek the ref updates so we can update metadata afterwards.
    let mut cut = None;
    let (body_reader, ref_updates): (Box<dyn tokio::io::AsyncRead + Send + Unpin>, Vec<smart::RefUpdate>) = {
        let stream = req.into_body().into_data_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(stream);
        let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = if gzip {
            Box::new(async_compression::tokio::bufread::GzipDecoder::new(tokio::io::BufReader::new(reader)))
        } else {
            Box::new(reader)
        };
        let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = if service == Service::ReceivePack {
            let capped = smart::CappedRead::new(reader, max_push);
            cut = Some(capped.exceeded());
            Box::new(capped)
        } else {
            reader
        };
        if service == Service::ReceivePack {
            // Read the pkt-line command section (up to the flush packet) into memory; pass the
            // rest of the body through untouched.
            let mut reader = reader;
            let mut prefix = Vec::new();
            match read_commands(&mut reader, &mut prefix).await {
                Ok(()) => {}
                Err(_) if cut.as_ref().is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst)) => {
                    return (StatusCode::PAYLOAD_TOO_LARGE, format!("{}\n", smart::push_too_large(max_push))).into_response();
                }
                Err(e) => return (StatusCode::BAD_REQUEST, format!("bad request: {e}\n")).into_response(),
            }
            let updates = smart::parse_ref_updates(&prefix);
            let chained = tokio::io::AsyncReadExt::chain(std::io::Cursor::new(prefix), reader);
            (Box::new(chained), updates)
        } else {
            (reader, Vec::new())
        }
    };

    let (stream, done) = match smart::serve(s.store.git_binary(), &path, service, &env, body_reader) {
        Ok(x) => x,
        Err(e) => return ApiError(e).into_response(),
    };

    // After the client has been served, record the push.
    if service == Service::ReceivePack && !is_wiki {
        let s2 = s.clone();
        let actor = auth.user.clone();
        tokio::spawn(async move {
            let exit = done.await;
            let cut = cut.is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst));
            if cut {
                tracing::info!(repo = rw.repo.id, max_push, "push refused: {}", smart::push_too_large(max_push));
            } else if exit.success {
                if let Err(e) = crate::githttp::after_push(&s2, &rw, actor.as_ref(), &ref_updates).await {
                    tracing::warn!(error = %e, repo = rw.repo.id, "post-push processing failed");
                }
            } else {
                tracing::info!(repo = rw.repo.id, stderr = %exit.stderr, "receive-pack ended with an error");
            }
        });
    } else {
        tokio::spawn(async move {
            let _ = done.await;
        });
    }

    let body = Body::from_stream(stream);
    (
        [
            (header::CONTENT_TYPE, format!("application/x-{}-result", service.name())),
            (header::CACHE_CONTROL, "no-cache, max-age=0, must-revalidate".to_string()),
        ],
        body,
    )
        .into_response()
}

/// Read pkt-lines until the flush packet into `buf` (which then contains the command list
/// plus the flush). Bounded so a hostile client cannot make us buffer a whole pack.
async fn read_commands<R: tokio::io::AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;
    loop {
        let mut len = [0u8; 4];
        r.read_exact(&mut len).await?;
        buf.extend_from_slice(&len);
        let n = usize::from_str_radix(std::str::from_utf8(&len).map_err(std::io::Error::other)?, 16).map_err(std::io::Error::other)?;
        if n == 0 {
            return Ok(());
        }
        if !(4..=65520).contains(&n) {
            return Err(std::io::Error::other("bad pkt-line length"));
        }
        let mut line = vec![0u8; n - 4];
        r.read_exact(&mut line).await?;
        buf.extend_from_slice(&line);
        if buf.len() > 4 * 1024 * 1024 {
            return Err(std::io::Error::other("too many ref updates"));
        }
    }
}

/// Bookkeeping after a successful push: default branch, emptiness, size, and an event.
pub async fn after_push(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: Option<&reef_db::accounts::Account>,
    updates: &[smart::RefUpdate],
) -> reef_core::Result<()> {
    let path = s.store.path(rw.repo.id());
    let p2 = path.clone();
    let (is_empty, branches, head) = tokio::task::spawn_blocking(move || {
        let r = reef_git::RepoReader::open(&p2)?;
        Ok::<_, Error>((r.is_empty(), r.branches()?, r.head_branch()))
    })
    .await
    .map_err(Error::internal)??;
    // If HEAD points at a branch that does not exist but others do, move it to the first
    // pushed branch (preferring main/master), like GitHub does for the first push.
    if !branches.is_empty() {
        let head_exists = head.as_ref().map(|h| branches.iter().any(|(b, _)| b == h)).unwrap_or(false);
        if !head_exists {
            let pick = ["main", "master", "trunk", "develop"]
                .iter()
                .find(|c| branches.iter().any(|(b, _)| b == *c))
                .map(|c| c.to_string())
                .or_else(|| updates.iter().filter_map(|u| u.branch()).next().map(String::from))
                .unwrap_or_else(|| branches[0].0.clone());
            s.store.set_head(&path, &pick).await?;
            s.db.update_repo(rw.repo.id(), reef_db::repos::RepoPatch { default_branch: Some(pick), ..Default::default() }).await?;
        }
    }
    let size = s.store.size_kb(rw.repo.id()).await.ok();
    s.db.touch_repo_push(rw.repo.id(), size, is_empty).await?;
    if updates.iter().any(|u| u.branch() == Some(rw.repo.default_branch.as_str())) || rw.repo.is_empty != 0 {
        let (s2, rw2) = (s.clone(), rw.clone());
        tokio::spawn(async move {
            if let Ok(rw2) = s2.db.repo_with_owner_by_id(rw2.repo.id()).await {
                crate::rest::repos::refresh_language(&s2, &rw2).await;
            }
        });
    }
    for u in updates {
        let kind = if u.is_create() {
            "create"
        } else if u.is_delete() {
            "delete"
        } else {
            "push"
        };
        s.db.record_event(rw.repo.id(), actor.map(|a| a.id()), kind, &serde_json::json!({"ref": u.name, "before": u.old, "after": u.new}))
            .await?;
        if let Some(b) = u.branch() {
            crate::pulls::on_branch_updated(s, rw, b).await?;
        }
        if let Err(e) = crate::actions::on_push(s, rw, actor, u).await {
            tracing::warn!(error = %e, "actions: planning runs for push failed");
        }
    }
    crate::mirror::after_push(s, rw).await;
    Ok(())
}

/// Path of the code or wiki repository named in the URL.
async fn repo_path(s: &AppState, rw: &RepoWithOwner, repo_in_url: &str) -> Result<std::path::PathBuf, Response> {
    if split_wiki(repo_in_url).1 {
        if rw.repo.has_wiki == 0 {
            return Err((StatusCode::NOT_FOUND, "Wiki is disabled\n").into_response());
        }
        s.store.ensure_wiki(rw.repo.id(), "main").await.map_err(|e| ApiError(e).into_response())
    } else {
        Ok(s.store.path(rw.repo.id()))
    }
}

pub async fn head(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, Service::UploadPack).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    format!("ref: refs/heads/{}\n", rw.repo.default_branch).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use base64::Engine as _;
    use http_body_util::BodyExt;
    use std::net::SocketAddr;
    use tokio::io::AsyncReadExt;
    use tower::ServiceExt;

    fn git(dir: &std::path::Path, args: &[&str]) -> Vec<u8> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    }

    /// A receive-pack request body creating `refs/heads/main` at a commit carrying 1 MiB of
    /// zeros in an uncompressed pack, gzipped: about 1 MiB for git, a few KiB on the wire.
    async fn gzipped_push(tag: &str) -> Vec<u8> {
        let src = std::env::temp_dir().join(format!("reef-pushsrc-{tag}-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(&src).unwrap();
        git(&src, &["init", "-q", "-b", "main"]);
        std::fs::write(src.join("zeros"), vec![0u8; 1024 * 1024]).unwrap();
        git(&src, &["add", "zeros"]);
        git(&src, &["commit", "-q", "-m", "zeros"]);
        let sha = String::from_utf8(git(&src, &["rev-parse", "HEAD"])).unwrap().trim().to_string();
        let mut child = std::process::Command::new("git")
            .arg("-C")
            .arg(&src)
            .args(["-c", "pack.compression=0", "pack-objects", "--stdout", "--revs", "--no-reuse-object", "-q"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(format!("{sha}\n").as_bytes()).unwrap();
        let pack = child.wait_with_output().unwrap().stdout;
        assert!(pack.len() > 1024 * 1024, "the pack is stored uncompressed");
        let zero = reef_git::smart::RefUpdate::ZERO;
        let mut body = reef_git::smart::pkt_line(&format!("{zero} {sha} refs/heads/main\0report-status\n"));
        body.extend_from_slice(reef_git::smart::FLUSH);
        body.extend_from_slice(&pack);
        let mut gz = Vec::new();
        async_compression::tokio::bufread::GzipEncoder::new(&body[..]).read_to_end(&mut gz).await.unwrap();
        assert!(gz.len() < 64 * 1024, "gzip shrinks the zeros ({} bytes)", gz.len());
        std::fs::remove_dir_all(&src).ok();
        gz
    }

    async fn push(max_push_bytes: u64, tag: &str) -> (StatusCode, bool) {
        let s = crate::testing::state(tag, |c| c.limits.max_push_bytes = max_push_bytes).await;
        let (u, token) = crate::testing::user_with_token(&s, "pusher").await;
        let repo = s.db.create_repo(u.id(), reef_db::repos::NewRepo { name: "big".into(), ..Default::default() }).await.unwrap();
        s.store.init(repo.id(), &repo.default_branch).await.unwrap();
        let body = gzipped_push(tag).await;
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("pusher:{token}"));
        let mut req = Request::post("/pusher/big.git/git-receive-pack")
            .header("authorization", format!("Basic {basic}"))
            .header("content-type", "application/x-git-receive-pack-request")
            .header("content-encoding", "gzip")
            .header("content-length", body.len().to_string())
            .body(Body::from(body))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 90], 40000))));
        let res = crate::router(s.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let _ = res.into_body().collect().await;
        let created = std::process::Command::new("git")
            .arg("-C")
            .arg(s.store.path(repo.id()))
            .args(["rev-parse", "--verify", "-q", "refs/heads/main"])
            .output()
            .unwrap()
            .status
            .success();
        (status, created)
    }

    /// `[limits] max_push_bytes` bounds what git receives, not what crossed the wire: a small
    /// gzipped body that inflates past the cap is cut off and nothing is written.
    #[tokio::test]
    async fn the_push_cap_counts_decompressed_bytes() {
        let (status, created) = push(8 * 1024 * 1024, "pushok").await;
        assert_eq!(status, StatusCode::OK);
        assert!(created, "under the cap the push lands");
        let (_, created) = push(256 * 1024, "pushcut").await;
        assert!(!created, "a gzipped push that inflates past the cap is refused");
    }
}

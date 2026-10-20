#![recursion_limit = "512"]
// Handlers return `Response`-sized errors and the runner protocol has wide path tuples; the
// size-based lints would only add boxing for no benefit.
#![allow(clippy::large_enum_variant, clippy::result_large_err, clippy::type_complexity)]
//! The HTTP surface of Reef: GitHub-compatible REST (`/api/v3`) and GraphQL
//! (`/api/graphql`), the OAuth device flow, git smart HTTP, Reef's own session API,
//! the embedded web UI, and the health, readiness and metrics endpoints.

pub mod actions;
pub mod admin;
pub mod auth;
pub mod config;
pub mod error;
pub mod githttp;
pub mod graphql;
pub mod health;
pub mod languages;
pub mod lfs;
pub mod license;
pub mod logout;
pub mod metrics;
pub mod mirror;
pub mod notifications;
pub mod oauth;
pub mod oidc;
pub mod outbound;
pub mod profile;
pub mod projects;
pub mod proxy_protocol;
pub mod pulls;
pub mod ratelimit;
pub mod render;
pub mod rest;
pub mod security;
pub mod shapes;
pub mod signing;
pub mod stats;
pub mod timeout;
pub mod web;
pub mod webhooks;
pub mod wiki;

use axum::Router;
use axum::extract::Request;
use axum::extract::connect_info::{ConnectInfo, Connected};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use futures::FutureExt;
use reef_db::Db;
use reef_git::Store;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::net::{TcpListener, UnixListener};
use tower::Service;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer};

pub use config::Config;

/// `v<version> (<commit>)`, stamped by `build.rs`; what `reef version` and the API meta
/// endpoint report.
pub const VERSION: &str = env!("REEF_VERSION");
/// The version alone (`v0.1.0`), for `reef_build_info{version}`.
pub const BUILD_VERSION: &str = env!("REEF_BUILD_VERSION");
/// The commit alone (`abc123def456`, `-dirty` when built from a modified tree).
pub const BUILD_COMMIT: &str = env!("REEF_BUILD_COMMIT");

/// Make `ring` the process-wide rustls provider (reqwest, russh and the TLS listeners all use
/// rustls; installing one provider explicitly avoids the "no default provider" panic).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub use timeout::REQUEST_TIMEOUT;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub store: Store,
    pub config: Arc<Config>,
    pub schema: graphql::Schema,
    pub oidc: Arc<oidc::Provider>,
    pub actions: Arc<actions::ActionsState>,
    pub stats: Arc<stats::StatsCache>,
    /// API rate limiting (`[limits] requests_per_hour`, per instance).
    pub limiter: Arc<ratelimit::RateLimiter>,
    /// Set when shutdown begins: `/readyz` answers 503 so load balancers drain this instance.
    pub shutting_down: Arc<AtomicBool>,
    /// Set when the SSH server task ended outside a shutdown: `/readyz` reports `ssh` failed.
    pub ssh_failed: Arc<AtomicBool>,
}

/// `AppState` as per-request GraphQL data (the schema is built before the state exists).
#[derive(Clone)]
pub struct AppStateHandle(pub AppState);

impl AppState {
    pub fn new(db: Db, store: Store, config: Config) -> reef_core::Result<AppState> {
        let actions = Arc::new(actions::ActionsState::load(&config.data_dir)?);
        let limiter = Arc::new(ratelimit::RateLimiter::new(&config));
        let config = Arc::new(config);
        let schema = graphql::build_schema(db.clone(), store.clone(), config.clone());
        Ok(AppState {
            db,
            store,
            config,
            schema,
            oidc: Arc::new(oidc::Provider::default()),
            actions,
            stats: Arc::new(stats::StatsCache::default()),
            limiter,
            shutting_down: Arc::new(AtomicBool::new(false)),
            ssh_failed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Flip readiness off; `/readyz` reports 503 from now on.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
    }

    /// The SSH server is gone: `/readyz` reports it until the process is restarted.
    pub fn mark_ssh_failed(&self) {
        self.ssh_failed.store(true, Ordering::Relaxed);
    }
}

/// The complete application router. Cross-cutting policy is attached where it belongs:
/// the rate limiter to the API routers (so every API route is limited and nothing else is),
/// the request timeout around everything with the streaming routes opting out where they are
/// defined (`timeout::exempt`, `timeout::exempt_router`); git-writing work that must not be
/// cut half way runs detached in the helpers REST and GraphQL share (`timeout::detached`).
pub fn router(state: AppState) -> Router {
    // `/metrics` moves to its own listener when `[metrics].listen` is set.
    let metrics_here = state.config.metrics_listen_addr().is_none();
    let limited = |r: Router<AppState>| r.route_layer(middleware::from_fn_with_state(state.clone(), ratelimit::rate_limit));
    Router::new()
        .merge(timeout::exempt_router(health::router(metrics_here)))
        .nest("/api/v3", limited(rest::router()))
        .merge(limited(Router::new().route("/api/graphql", axum::routing::post(graphql::handler).get(graphql::playground))))
        .nest(
            "/api/reef",
            limited(
                oauth::reef_router()
                    .merge(wiki::router())
                    .merge(projects::router())
                    .merge(profile::router())
                    .merge(admin::router())
                    .merge(notifications::router())
                    .merge(stats::router())
                    .merge(security::router())
                    .merge(mirror::router()),
            ),
        )
        .merge(oauth::github_router())
        .route("/login/oidc", axum::routing::get(oidc::start))
        .route("/login/oidc/callback", axum::routing::get(oidc::callback))
        .route(logout::PATH, axum::routing::post(logout::backchannel_logout))
        .route("/{owner}/{repo}/raw/{rev}/{*path}", axum::routing::get(rest::contents::raw))
        .route("/{owner}/{repo}/commit/{sha}", axum::routing::get(rest::pulls::commit_patch_url))
        .route("/{owner}/{repo}/pull/{number}", axum::routing::get(rest::pulls::pull_patch_url))
        .merge(timeout::exempt_router(lfs::router()))
        .merge(timeout::exempt_router(actions::runner::router()))
        .merge(timeout::exempt_router(githttp::router()))
        .merge(web::router())
        .layer(middleware::from_fn(timeout::request_timeout))
        .layer(middleware::from_fn(common_headers))
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(middleware::from_fn(metrics::http_metrics))
        .layer(middleware::from_fn_with_state(state.clone(), request_log))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(middleware::from_fn_with_state(state.clone(), inbound_request_id))
        .with_state(state)
}

/// What `[metrics].listen` serves: `/metrics`, with the primary listener's panic recovery,
/// counted, logged and given request ids like its routes (every listener).
pub fn metrics_listener_router(state: AppState) -> Router {
    health::metrics_router()
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(middleware::from_fn(metrics::http_metrics))
        .layer(middleware::from_fn_with_state(state.clone(), request_log))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(middleware::from_fn_with_state(state.clone(), inbound_request_id))
        .with_state(state)
}

/// The socket peer of a request (the PROXY-reported one when a header was accepted),
/// canonical (`ratelimit::canonical_ip`): `Some(ip)` over TCP, `None` on the Unix socket.
fn peer_of(req: &Request) -> Option<std::net::IpAddr> {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| ratelimit::canonical_ip(c.0.ip()))
}

/// A client-supplied `X-Request-Id` is kept only from a trusted peer (a trusted proxy or
/// loopback; the local Unix socket only when `[http] trusted_proxies` is set) and only when it
/// matches `^[A-Za-z0-9._-]{1,64}$`; otherwise it is dropped and `SetRequestIdLayer`
/// generates a UUID.
async fn inbound_request_id(axum::extract::State(s): axum::extract::State<AppState>, mut req: Request, next: Next) -> Response {
    if let Some(v) = req.headers().get("x-request-id") {
        let trusted = s.limiter.trusts_peer(peer_of(&req));
        if !trusted || !well_formed_request_id(v.as_bytes()) {
            req.headers_mut().remove("x-request-id");
        }
    }
    next.run(req).await
}

fn well_formed_request_id(v: &[u8]) -> bool {
    (1..=64).contains(&v.len()) && v.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Bound sockets, all opened before readiness is announced.
pub struct Listeners {
    pub main: TcpListener,
    pub extra: Vec<TcpListener>,
    pub unix: Option<UnixListener>,
    /// `[metrics].listen`: health and metrics only.
    pub metrics: Option<TcpListener>,
}

/// The `[tls]` material, loaded and parsed; `None` for plain HTTP. `serve` binds it to the
/// TCP listeners; the binary calls this before announcing readiness so a bad certificate or
/// key fails the start rather than the first connection.
pub async fn tls_config(config: &Config) -> std::io::Result<Option<axum_server::tls_rustls::RustlsConfig>> {
    if !config.tls.enabled() {
        return Ok(None);
    }
    axum_server::tls_rustls::RustlsConfig::from_pem_file(&config.tls.cert_file, &config.tls.key_file)
        .await
        .map(Some)
        .map_err(|e| std::io::Error::other(format!("tls: {e}")))
}

/// Serve the application on every listener until `shutdown` resolves, then drain. TCP
/// listeners speak TLS when `tls` is given (`tls_config`); the Unix socket is always plain HTTP.
pub async fn serve(
    listeners: Listeners,
    tls: Option<axum_server::tls_rustls::RustlsConfig>,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = ApiHostRewrite { inner: router(state.clone()), config: state.config.clone() };
    let shutdown = shutdown.shared();
    let proxy = proxy_protocol::ProxyPolicy::from_config(&state.config).map(Arc::new);
    let mut tasks: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>>> = Vec::new();
    tasks.push(serve_tcp(listeners.main, app.clone(), tls.clone(), proxy.clone(), shutdown.clone()));
    for l in listeners.extra {
        tasks.push(serve_tcp(l, app.clone(), tls.clone(), proxy.clone(), shutdown.clone()));
    }
    if let Some(u) = listeners.unix {
        let svc = axum::ServiceExt::<Request>::into_make_service_with_connect_info::<UnixConnectInfo>(app.clone());
        tasks.push(Box::pin(axum::serve(u, svc).with_graceful_shutdown(shutdown.clone()).into_future()));
    }
    if let Some(m) = listeners.metrics {
        let metrics = metrics_listener_router(state.clone());
        tasks.push(serve_tcp(m, metrics, tls.clone(), proxy.clone(), shutdown.clone()));
    }
    futures::future::try_join_all(tasks).await.map(|_| ())
}

/// Serve one TCP listener: plain, or TLS when `tls` is given; with `[http] proxy_protocol`
/// (`proxy`) every connection is first checked for a PROXY header (see `proxy_protocol`).
/// TLS handshakes and PROXY detection run per connection (`accept_loop`), so a slow client
/// never holds up the others.
fn serve_tcp<S>(
    listener: TcpListener,
    svc: S,
    tls: Option<axum_server::tls_rustls::RustlsConfig>,
    proxy: Option<Arc<proxy_protocol::ProxyPolicy>>,
    shutdown: futures::future::Shared<impl std::future::Future<Output = ()> + Send + 'static>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>>
where
    S: Service<Request, Response = Response, Error = std::convert::Infallible> + Clone + Send + Sync + 'static,
    <S as Service<Request>>::Future: Send,
{
    use axum::serve::ListenerExt;
    let make = axum::ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(svc);
    match (tls, proxy) {
        (None, None) => Box::pin(axum::serve(listener, make).with_graceful_shutdown(shutdown).into_future()),
        (None, Some(proxy)) => Box::pin(async move {
            let accepted = accept_loop(listener, Some(proxy), |s| async move { Ok(s) })?;
            axum::serve(accepted.tap_io(|_| {}), make).with_graceful_shutdown(shutdown).await
        }),
        (Some(tls), proxy) => Box::pin(async move {
            let acceptor = tokio_rustls::TlsAcceptor::from(tls.get_inner());
            let accepted = accept_loop(listener, proxy, move |s| {
                let acceptor = acceptor.clone();
                async move { acceptor.accept(s).await }
            })?;
            axum::serve(accepted.tap_io(|_| {}), make).with_graceful_shutdown(shutdown).await
        }),
    }
}

/// Connections accepted by `accept_loop`, ready to serve, with the peer address to report
/// (the PROXY header's source when one was accepted).
pub struct Accepted<Io> {
    rx: tokio::sync::mpsc::Receiver<(Io, SocketAddr)>,
    local: SocketAddr,
}

impl<Io> axum::serve::Listener for Accepted<Io>
where
    Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    type Io = Io;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Io, SocketAddr) {
        match self.rx.recv().await {
            Some(c) => c,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.local)
    }
}

/// How long a new connection may take to be set up (PROXY header and TLS handshake) before
/// it is dropped.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections being set up at once per listener. At the cap the listener stops accepting
/// (new connections wait in the kernel's backlog) until one finishes or times out, so a flood
/// of clients that open connections and never complete a handshake cannot pile up tasks.
pub const MAX_PENDING_HANDSHAKES: usize = 1024;

/// Accept on `listener` and set each connection up on its own task: the PROXY check under
/// `proxy`, then `setup` (the TLS handshake, or nothing), within `HANDSHAKE_TIMEOUT` and with
/// at most `MAX_PENDING_HANDSHAKES` in progress. Ends when the returned listener is dropped
/// (the server stopped).
pub fn accept_loop<Io, F, Fut>(
    listener: TcpListener,
    proxy: Option<Arc<proxy_protocol::ProxyPolicy>>,
    setup: F,
) -> std::io::Result<Accepted<Io>>
where
    Io: Send + 'static,
    F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<Io>> + Send + 'static,
{
    accept_loop_with(listener, proxy, setup, HANDSHAKE_TIMEOUT, MAX_PENDING_HANDSHAKES)
}

/// `accept_loop` with an explicit handshake timeout and cap on pending handshakes.
pub fn accept_loop_with<Io, F, Fut>(
    listener: TcpListener,
    proxy: Option<Arc<proxy_protocol::ProxyPolicy>>,
    setup: F,
    handshake_timeout: Duration,
    max_pending: usize,
) -> std::io::Result<Accepted<Io>>
where
    Io: Send + 'static,
    F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<Io>> + Send + 'static,
{
    let local = listener.local_addr()?;
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let setup = Arc::new(setup);
    let pending = Arc::new(tokio::sync::Semaphore::new(max_pending.max(1)));
    tokio::spawn(async move {
        loop {
            // A slot first: at the cap, stop accepting until a handshake finishes.
            let permit = tokio::select! {
                _ = tx.closed() => return,
                p = pending.clone().acquire_owned() => match p {
                    Ok(p) => p,
                    Err(_) => return,
                },
            };
            let (mut stream, peer) = tokio::select! {
                _ = tx.closed() => return,
                r = listener.accept() => match r {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(error = %e, "accepting a connection failed");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                },
            };
            let (tx, setup, proxy) = (tx.clone(), setup.clone(), proxy.clone());
            tokio::spawn(async move {
                let handshake = async {
                    let peer = match &proxy {
                        Some(policy) => match proxy_protocol::accept(&mut stream, peer, policy).await {
                            Ok(Some(addr)) => addr,
                            Ok(None) => return None,
                            Err(e) => {
                                tracing::info!(peer = %peer, error = %e, "dropping a connection with a bad PROXY header");
                                return None;
                            }
                        },
                        None => peer,
                    };
                    match setup(stream).await {
                        Ok(io) => Some((io, peer)),
                        Err(e) => {
                            tracing::debug!(peer = %peer, error = %e, "connection setup failed");
                            None
                        }
                    }
                };
                let ready = match tokio::time::timeout(handshake_timeout, handshake).await {
                    Ok(r) => r,
                    Err(_) => {
                        tracing::debug!(peer = %peer, "dropping a connection whose handshake timed out");
                        None
                    }
                };
                // The slot is free once the handshake is over, before waiting on the server.
                drop(permit);
                if let Some(c) = ready {
                    let _ = tx.send(c).await;
                }
            });
        }
    });
    Ok(Accepted { rx, local })
}

/// Connection info for the Unix socket listener (there is no peer address to report).
#[derive(Clone, Debug)]
pub struct UnixConnectInfo;

impl Connected<IncomingStream<'_, UnixListener>> for UnixConnectInfo {
    fn connect_info(_: IncomingStream<'_, UnixListener>) -> Self {
        UnixConnectInfo
    }
}

/// Requests arriving on the `api.` host (as `gh` sends for github.com-style hosts) are
/// rewritten onto the `/api/v3` and `/api/graphql` routes. This runs before routing, so it
/// wraps the router rather than being a layer on it.
#[derive(Clone)]
pub struct ApiHostRewrite<S> {
    inner: S,
    config: Arc<Config>,
}

impl<S, B> Service<Request<B>> for ApiHostRewrite<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        self.inner.call(api_host_rewrite(&self.config, req))
    }
}

fn api_host_rewrite<B>(config: &Config, mut req: Request<B>) -> Request<B> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or("").to_ascii_lowercase())
        .unwrap_or_default();
    if config.is_api_host(&host) {
        let path = req.uri().path();
        let new_path = if path == "/graphql" {
            "/api/graphql".to_string()
        } else if path.starts_with("/api/v3")
            || path.starts_with("/api/graphql")
            || path.starts_with("/api/reef")
            || health::is_health_path(path)
        {
            path.to_string()
        } else {
            format!("/api/v3{}", path.trim_end_matches('/'))
        };
        let mut parts = req.uri().clone().into_parts();
        let pq = match req.uri().query() {
            Some(q) => format!("{new_path}?{q}"),
            None => new_path,
        };
        if let Ok(pq) = pq.parse() {
            parts.path_and_query = Some(pq);
            if let Ok(uri) = axum::http::Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
        }
    }
    req
}

/// The request id set by `SetRequestIdLayer`, as text.
fn request_id_of(req: &Request) -> String {
    req.extensions().get::<RequestId>().and_then(|r| r.header_value().to_str().ok()).unwrap_or("").to_string()
}

/// One line per request at `info` (`debug` for the health endpoints), `msg` = `request`:
/// method, path without the query string, status, duration in milliseconds (float, 3
/// decimals), response bytes when known, request id and the client address (the peer,
/// or the forwarded address behind a trusted proxy; an IP without port, `unix` on the Unix
/// socket with nothing forwarded).
async fn request_log(axum::extract::State(s): axum::extract::State<AppState>, req: Request, next: Next) -> Response {
    let start = std::time::Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let request_id = request_id_of(&req);
    let remote = match s.limiter.client_ip(peer_of(&req), req.headers()) {
        Some(ip) => ip.to_string(),
        None if req.extensions().get::<ConnectInfo<UnixConnectInfo>>().is_some() => "unix".to_string(),
        None => String::new(),
    };
    let res = next.run(req).await;
    let status = res.status().as_u16();
    let duration_ms = (start.elapsed().as_secs_f64() * 1_000_000.0).round() / 1000.0;
    let bytes = http_body::Body::size_hint(res.body()).exact().unwrap_or(0);
    if health::is_health_path(&path) {
        tracing::debug!(method = %method, path = %path, status, duration_ms, bytes, request_id = %request_id, remote = %remote, "request");
    } else {
        tracing::info!(method = %method, path = %path, status, duration_ms, bytes, request_id = %request_id, remote = %remote, "request");
    }
    res
}

/// A handler panicked: log it and answer 500. The request id is on the response through
/// `PropagateRequestIdLayer`, which sits outside this layer.
fn panic_response(err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let detail = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "unknown panic".to_string()
    };
    tracing::error!(panic = %detail, "request handler panicked");
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, [(header::CONTENT_TYPE, "application/json")], r#"{"message":"internal error"}"#)
        .into_response()
}

async fn common_headers(req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let request_id = request_id_of(&req);
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&request_id) {
        h.insert("x-github-request-id", v);
    }
    h.insert("x-github-media-type", HeaderValue::from_static("github.v3; format=json"));
    h.insert("x-frame-options", HeaderValue::from_static("deny"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("origin-when-cross-origin, strict-origin-when-cross-origin"));
    if is_api {
        // The X-RateLimit-* values themselves come from `ratelimit::rate_limit`.
        h.insert("access-control-expose-headers", HeaderValue::from_static("ETag, Link, Location, Retry-After, X-GitHub-OTP, X-RateLimit-Limit, X-RateLimit-Remaining, X-RateLimit-Used, X-RateLimit-Resource, X-RateLimit-Reset, X-OAuth-Scopes, X-Accepted-OAuth-Scopes, X-GitHub-Media-Type, X-GitHub-SSO, X-GitHub-Request-Id, X-Request-Id"));
    }
    res
}

#[cfg(test)]
pub(crate) mod testing {
    //! Shared fixtures for tests that drive the real router or services against SQLite.
    use super::*;

    /// A standalone-mode `AppState` on a fresh SQLite database under a temporary directory.
    /// `tune` adjusts the configuration before the state is built.
    pub(crate) async fn state(tag: &str, tune: impl FnOnce(&mut Config)) -> AppState {
        let dir = std::env::temp_dir().join(format!("reef-{tag}-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config { data_dir: dir.clone(), ..Default::default() };
        cfg.auth.mode = "standalone".into();
        tune(&mut cfg);
        let db = reef_db::Db::connect(&cfg.database_url()).await.unwrap();
        let store = reef_git::Store::new(dir.join("repositories"), "git").with_tmp_dir(dir.join("tmp"));
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        AppState::new(db, store, cfg).unwrap()
    }

    /// A user with a personal access token: `(user, plain token)`.
    pub(crate) async fn user_with_token(s: &AppState, login: &str) -> (reef_db::accounts::Account, String) {
        let u =
            s.db.create_user(reef_db::accounts::NewUser {
                login,
                email: &format!("{login}@example.com"),
                password: Some("correct horse"),
                name: None,
                is_admin: false,
            })
            .await
            .unwrap();
        let (plain, _) = s.db.create_token(u.id(), "pat", "test", &["repo"], None, None).await.unwrap();
        (u, plain)
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectInfo, SocketAddr};
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn get(path: &str, ip: &str, token: Option<&str>) -> Request<Body> {
        let mut b = Request::get(path);
        if let Some(t) = token {
            b = b.header("authorization", format!("token {t}"));
        }
        let mut req = b.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 40000)));
        req
    }

    #[tokio::test]
    async fn api_calls_are_rate_limited_with_github_headers() {
        let s = super::testing::state("ratelimit", |_| {}).await;
        let app = super::router(s.clone());
        // 60 unauthenticated requests per hour per address, then 429.
        for i in 0..60u32 {
            let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.5", None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "request {i}");
            assert_eq!(res.headers()["x-ratelimit-limit"], "60");
            assert_eq!(res.headers()["x-ratelimit-remaining"], (59 - i).to_string());
            assert_eq!(res.headers()["x-ratelimit-used"], (i + 1).to_string());
            assert_eq!(res.headers()["x-ratelimit-resource"], "core");
            assert!(res.headers().contains_key("x-ratelimit-reset"));
        }
        let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.5", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(res.headers()["x-ratelimit-remaining"], "0");
        assert_eq!(res.headers()["x-ratelimit-used"], "60");
        assert_eq!(res.headers()["retry-after"], "60");
        assert_eq!(res.headers()["content-type"], "application/json");
        assert!(res.headers().contains_key("x-request-id"), "the id layers still wrap a 429");
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(body.contains("API rate limit exceeded for 203.0.113.5"), "{body}");
        // `/rate_limit` reports without spending; another address and non-API routes are unaffected.
        let res = app.clone().oneshot(get("/api/v3/rate_limit", "203.0.113.5", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["rate"]["limit"], 60);
        assert_eq!(body["rate"]["remaining"], 0);
        assert_eq!(body["resources"]["core"]["used"], 60);
        let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.6", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["x-ratelimit-remaining"], "59");
        for path in ["/healthz", "/readyz", "/metrics"] {
            let res = app.clone().oneshot(get(path, "203.0.113.5", None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert!(!res.headers().contains_key("x-ratelimit-limit"), "{path} is exempt");
        }
        // Authenticated callers draw from their own, larger budget. Credentials are verified
        // on the source's budget first (refunded when they resolve), so an exhausted address
        // cannot make the server verify new ones; once verified they work from any address.
        let (_, token) = super::testing::user_with_token(&s, "limited").await;
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.5", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "not verified from an exhausted address");
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.9", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["x-ratelimit-limit"], "5000");
        assert_eq!(res.headers()["x-ratelimit-remaining"], "4999");
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.5", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "verified: charged to the user wherever it comes from");
        assert_eq!(res.headers()["x-ratelimit-remaining"], "4998");
        // Bad credentials are answered 401 and charged to the (exhausted) address.
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.5", Some("fgp_nope"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.7", Some("fgp_nope"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(res.headers()["x-ratelimit-remaining"], "59");
        // The request metrics are keyed by the matched route pattern, never the raw path.
        let _ = app.clone().oneshot(get("/api/v3/repos/nobody/nothing", "203.0.113.8", None)).await.unwrap();
        let text = prometheus::TextEncoder::new().encode_to_string(&prometheus::default_registry().gather()).unwrap();
        assert!(text.contains("reef_http_requests_total{method=\"GET\",route=\"/api/v3/zen\",status=\"429\"}"), "{text}");
        assert!(text.contains("route=\"/api/v3/repos/{owner}/{repo}\",status=\"404\""), "{text}");
        assert!(!text.contains("nobody/nothing"), "raw paths never become labels");
        assert!(text.contains("reef_http_request_duration_seconds_bucket{method=\"GET\",route=\"/api/v3/zen\""));
    }

    /// The embedded UI browsing signed out is not held to the anonymous 60/h: same-origin
    /// fetches and requests with the session cookie draw from the web budget of the address.
    #[tokio::test]
    async fn the_web_ui_has_its_own_budget() {
        let s = super::testing::state("webbudget", |_| {}).await;
        let app = super::router(s.clone());
        let ui = |path: &str, how: &str| {
            let mut r = get(path, "203.0.113.20", None);
            match how {
                "fetch" => r.headers_mut().insert("sec-fetch-site", "same-origin".parse().unwrap()),
                _ => r.headers_mut().insert("cookie", format!("theme=dark; {}=expired", crate::auth::COOKIE).parse().unwrap()),
            };
            r
        };
        for i in 0..70u32 {
            for (path, how) in [("/api/reef/session", "fetch"), ("/api/v3/zen", "cookie")] {
                let res = app.clone().oneshot(ui(path, how)).await.unwrap();
                assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS, "{path} request {i}");
                assert_eq!(res.headers()["x-ratelimit-limit"], "5000", "{path}");
            }
        }
        // The anonymous budget of the same address is untouched, and a cross-site call is anonymous.
        let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.20", None)).await.unwrap();
        assert_eq!(
            (res.headers()["x-ratelimit-limit"].to_str().unwrap(), res.headers()["x-ratelimit-remaining"].to_str().unwrap()),
            ("60", "59")
        );
        let mut cross = get("/api/v3/zen", "203.0.113.20", None);
        cross.headers_mut().insert("sec-fetch-site", "cross-site".parse().unwrap());
        let res = app.clone().oneshot(cross).await.unwrap();
        assert_eq!(res.headers()["x-ratelimit-limit"], "60");
    }

    /// Request ids: an inbound id is kept only from a trusted (or loopback) peer and only
    /// when well formed.
    #[tokio::test]
    async fn inbound_request_ids_are_kept_only_when_trusted_and_well_formed() {
        let s = super::testing::state("reqid", |c| c.http.trusted_proxies = vec!["10.0.0.0/8".into()]).await;
        let app = super::router(s.clone());
        let with_id = |ip: &str, id: &str| {
            let mut r = get("/healthz", ip, None);
            r.headers_mut().insert("x-request-id", id.parse().unwrap());
            r
        };
        for (ip, id, kept) in [
            ("127.0.0.1", "abc-1.2_3", true),
            ("10.1.2.3", "edge-42", true),
            ("203.0.113.30", "abc-1.2_3", false),
            ("127.0.0.1", "has space", false),
            ("127.0.0.1", &"x".repeat(65), false),
        ] {
            let res = app.clone().oneshot(with_id(ip, id)).await.unwrap();
            let got = res.headers()["x-request-id"].to_str().unwrap().to_string();
            assert_eq!(got == id, kept, "{ip} {id:?} -> {got}");
            assert!(!got.is_empty());
        }
        // The Unix socket (no peer address) is trusted only when trusted_proxies is set.
        let unix = || {
            let mut r = Request::get("/healthz").body(Body::empty()).unwrap();
            r.headers_mut().insert("x-request-id", "via-socket".parse().unwrap());
            r
        };
        let res = app.clone().oneshot(unix()).await.unwrap();
        assert_eq!(res.headers()["x-request-id"], "via-socket");
        let plain = super::router(super::testing::state("reqid-unix", |_| {}).await);
        let res = plain.oneshot(unix()).await.unwrap();
        assert_ne!(res.headers()["x-request-id"], "via-socket", "no trusted proxies: the socket is not trusted");
    }

    /// Credentials are resolved only when the request still has budget: unknown credentials
    /// are charged to the source first (and refunded once they resolve), verified ones to
    /// their user first. A request that ends in 429 never reaches the resolver (no password
    /// hash).
    #[tokio::test]
    async fn credentials_are_resolved_only_while_there_is_budget() {
        let s = super::testing::state("precheck", |c| c.limits.requests_per_hour = 3).await;
        let app = super::router(s.clone());
        let (_, token) = super::testing::user_with_token(&s, "prechecked").await;
        let basic = format!("basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "prechecked:correct horse"));
        for _ in 0..60 {
            app.clone().oneshot(get("/api/v3/zen", "203.0.113.40", None)).await.unwrap();
        }
        let resolved = || s.limiter.credential_resolutions();
        let before = resolved();
        // Unknown credentials from an exhausted address: refused before resolving them.
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.40", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let mut pw = get("/api/v3/user", "203.0.113.40", None);
        pw.headers_mut().insert("authorization", basic.parse().unwrap());
        assert_eq!(app.clone().oneshot(pw).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resolved(), before, "no password hash for a request that is refused");
        // From an address with budget: resolved, charged to the user, the address refunded.
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.41", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            (&res.headers()["x-ratelimit-limit"], &res.headers()["x-ratelimit-remaining"]),
            (&"3".parse().unwrap(), &"2".parse().unwrap())
        );
        assert_eq!(resolved(), before + 1);
        let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.41", None)).await.unwrap();
        assert_eq!(res.headers()["x-ratelimit-remaining"], "59", "the verification token was given back");
        // Verified now: charged to the user first, from any address.
        for left in ["1", "0"] {
            let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.40", Some(&token))).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(res.headers()["x-ratelimit-remaining"], left);
        }
        let n = resolved();
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.41", Some(&token))).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(res.headers()["x-ratelimit-limit"], "3", "refused on the user's budget");
        assert_eq!(resolved(), n, "not resolved: the user's budget was spent");
        // Bad credentials cost the source one request each, and are resolved while it has budget.
        let res = app.clone().oneshot(get("/api/v3/user", "203.0.113.42", Some("fgp_nope"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(res.headers()["x-ratelimit-remaining"], "59");
    }

    /// A dual-stack listener reports IPv4 clients as `::ffff:a.b.c.d`: they are matched against
    /// `trusted_proxies`, loopback and the rate-limit keys as the IPv4 address.
    #[tokio::test]
    async fn a_dual_stack_listener_sees_ipv4_peers_as_ipv4() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let Ok(main) = tokio::net::TcpListener::bind("[::]:0").await else { return };
        let port = main.local_addr().unwrap().port();
        let v4: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        if tokio::net::TcpStream::connect(v4).await.is_err() {
            eprintln!("no dual-stack sockets here; skipping");
            return;
        }
        let s = super::testing::state("dualstack", |c| c.http.trusted_proxies = vec!["127.0.0.1".into()]).await;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(super::serve(super::Listeners { main, extra: vec![], unix: None, metrics: None }, None, s, async move {
            let _ = stopped.await;
        }));
        async fn call(addr: std::net::SocketAddr, path: &str) -> String {
            let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Forwarded-For: 198.51.100.77\r\nX-Request-Id: edge-77\r\nConnection: close\r\n\r\n"
            );
            c.write_all(req.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = c.read_to_end(&mut out).await;
            String::from_utf8_lossy(&out).to_ascii_lowercase()
        }
        let first = call(v4, "/api/v3/zen").await;
        assert!(first.starts_with("http/1.1 200"), "{first}");
        assert!(first.contains("x-request-id: edge-77"), "the mapped loopback proxy is trusted for request ids: {first}");
        let report = call(v4, "/api/v3/rate_limit").await;
        assert!(report.contains("\"used\":1"), "charged to the forwarded address, so the mapped peer matched 127.0.0.1: {report}");
        let _ = stop.send(());
        let _ = server.await;
    }

    /// Readiness: `{"status":"ok"}`, or 503 naming what failed and nothing else; the
    /// database is pinged over one kept-open health connection.
    #[tokio::test]
    async fn readyz_bodies_and_the_health_connection() {
        let s = super::testing::state("readyz", |_| {}).await;
        let app = super::router(s.clone());
        let body = |res: axum::response::Response| async move {
            serde_json::from_slice::<serde_json::Value>(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
        };
        assert_eq!(s.db.health_connections(), 0, "opened on the first probe");
        for _ in 0..3 {
            let res = app.clone().oneshot(get("/readyz", "127.0.0.1", None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(body(res).await, serde_json::json!({"status": "ok"}));
        }
        assert_eq!(s.db.health_connections(), 1, "one connection, reused by every probe");
        s.mark_ssh_failed();
        let res = app.clone().oneshot(get("/readyz", "127.0.0.1", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body(res).await, serde_json::json!({"status": "unavailable", "failed": "ssh"}));
        s.begin_shutdown();
        let res = app.clone().oneshot(get("/readyz", "127.0.0.1", None)).await.unwrap();
        assert_eq!(body(res).await, serde_json::json!({"status": "unavailable", "failed": "shutdown"}));
    }

    /// `[http] proxy_protocol` on a real listener: a trusted proxy's v2 header sets the client
    /// address (seen here through the rate-limit budget it is charged to); the same peer
    /// without a header is served as itself; an untrusted peer with a header is dropped,
    /// without one it is served as usual.
    #[tokio::test]
    async fn the_http_listener_speaks_proxy_protocol() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn call(addr: std::net::SocketAddr, header: Option<Vec<u8>>, path: &str) -> String {
            let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
            if let Some(h) = header {
                c.write_all(&h).await.unwrap();
            }
            c.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = c.read_to_end(&mut out).await;
            String::from_utf8_lossy(&out).into_owned()
        }
        for (trusted, tag) in [(vec!["127.0.0.1".to_string()], "pp-trusted"), (vec!["10.0.0.0/8".to_string()], "pp-untrusted")] {
            let s = super::testing::state(tag, |c| {
                c.http.proxy_protocol = true;
                c.http.trusted_proxies = trusted.clone();
            })
            .await;
            let main = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = main.local_addr().unwrap();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let server =
                tokio::spawn(super::serve(super::Listeners { main, extra: vec![], unix: None, metrics: None }, None, s, async move {
                    let _ = stopped.await;
                }));
            let client: std::net::SocketAddr = "198.51.100.7:40000".parse().unwrap();
            let header = || Some(reef_proxy_header(client, addr));
            if tag == "pp-trusted" {
                assert!(call(addr, header(), "/api/v3/zen").await.starts_with("HTTP/1.1 200"));
                let proxied = call(addr, header(), "/api/v3/rate_limit").await;
                assert!(proxied.contains("\"used\":1"), "charged to the carried address: {proxied}");
                let direct = call(addr, None, "/api/v3/rate_limit").await;
                assert!(
                    direct.starts_with("HTTP/1.1 200") && direct.contains("\"used\":0"),
                    "no header: served as the peer itself: {direct}"
                );
            } else {
                assert_eq!(call(addr, header(), "/healthz").await, "", "an untrusted peer's header: dropped");
                assert!(call(addr, None, "/healthz").await.starts_with("HTTP/1.1 200"), "no header: served as usual");
            }
            let _ = stop.send(());
            let _ = server.await;
        }
    }

    /// A connection that never completes its handshake is dropped after the timeout, and no
    /// more than the cap are set up at once: the next one waits for a free slot.
    #[tokio::test]
    async fn handshakes_are_timed_and_capped() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let started = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = started.clone();
        // A "handshake" that never finishes, like a client that opens TLS and goes silent.
        let _accepted = super::accept_loop_with(
            listener,
            None,
            move |s: tokio::net::TcpStream| {
                counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    std::future::pending::<()>().await;
                    Ok(s)
                }
            },
            Duration::from_millis(300),
            1,
        )
        .unwrap();
        let mut first = tokio::net::TcpStream::connect(addr).await.unwrap();
        let _second = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(started.load(Ordering::SeqCst), 1, "at the cap the second connection waits");
        let t0 = std::time::Instant::now();
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), first.read(&mut buf)).await.expect("dropped by the timeout");
        assert!(matches!(n, Ok(0) | Err(_)), "closed: {n:?}");
        assert!(t0.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(started.load(Ordering::SeqCst), 2, "the freed slot goes to the waiting connection");
    }

    fn reef_proxy_header(src: std::net::SocketAddr, dst: std::net::SocketAddr) -> Vec<u8> {
        crate::proxy_protocol::v2_header(src, dst)
    }

    #[tokio::test]
    async fn a_zero_limit_turns_rate_limiting_off() {
        let s = super::testing::state("nolimit", |c| c.limits.unauthenticated_requests_per_hour = 0).await;
        let app = super::router(s.clone());
        for _ in 0..70 {
            let res = app.clone().oneshot(get("/api/v3/zen", "203.0.113.5", None)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert!(!res.headers().contains_key("x-ratelimit-limit"));
        }
        let res = app.clone().oneshot(get("/api/v3/rate_limit", "203.0.113.5", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "GitHub answers 404 when rate limiting is off");
    }

    fn req(m: Method, path: &str) -> Request<Body> {
        let mut req = Request::builder().method(m).uri(path).body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 77], 40000))));
        req
    }

    /// Every route family and whether it opts out of the request timeout, driven through the
    /// real router: the guard's decision is read back from the response, so a route that
    /// loses its marker, or a new exempt route that forgot one, fails here.
    #[tokio::test]
    async fn timeout_exemptions_are_attached_to_routes() {
        const GET: Method = Method::GET;
        const POST: Method = Method::POST;
        const PUT: Method = Method::PUT;
        const PATCH: Method = Method::PATCH;
        const DELETE: Method = Method::DELETE;
        let s = super::testing::state("timeouts", |c| {
            c.limits.requests_per_hour = 0;
            c.limits.unauthenticated_requests_per_hour = 0;
        })
        .await;
        let app = super::router(s.clone());
        // Only routes that stream or long-poll opt out; git-writing work is detached instead.
        let exempt = [
            (GET, "/healthz"),
            (GET, "/readyz"),
            (GET, "/metrics"),
            (GET, "/o/r/info/refs"),
            (POST, "/o/r/git-upload-pack"),
            (POST, "/o/r/git-receive-pack"),
            (POST, "/o/r/info/lfs/objects/batch"),
            (PUT, "/o/r/info/lfs/objects/abc"),
            (GET, "/_actions/repo/1/_apis/distributedtask/pools/1/messages"),
            (GET, "/api/v3/repos/yadvlabs/reef/tarball"),
            (GET, "/api/v3/repos/yadvlabs/reef/tarball/main"),
            (GET, "/api/v3/repos/yadvlabs/reef/zipball"),
            (GET, "/api/v3/repos/yadvlabs/reef/zipball/main"),
        ];
        let timed = [
            (POST, "/api/v3/user/repos"),
            (POST, "/api/v3/orgs/yadvlabs/repos"),
            (DELETE, "/api/v3/repos/yadvlabs/reef"),
            (DELETE, "/api/v3/orgs/yadvlabs"),
            (POST, "/api/v3/repos/yadvlabs/reef/forks"),
            (PUT, "/api/v3/repos/yadvlabs/reef/import"),
            (PATCH, "/api/v3/repos/yadvlabs/reef/import"),
            (DELETE, "/api/v3/repos/yadvlabs/reef/import"),
            (PUT, "/api/reef/repos/yadvlabs/reef/mirror"),
            (DELETE, "/api/reef/repos/yadvlabs/reef/mirror"),
            (POST, "/api/reef/repos/yadvlabs/reef/mirror/sync"),
            (POST, "/api/reef/repos/yadvlabs/reef/push-mirrors"),
            (DELETE, "/api/reef/repos/yadvlabs/reef/push-mirrors/1"),
            (POST, "/api/reef/repos/yadvlabs/reef/push-mirrors/1/push"),
            (PUT, "/api/v3/repos/yadvlabs/reef/pulls/7/merge"),
            (PUT, "/api/v3/repos/yadvlabs/reef/contents/README.md"),
            (DELETE, "/api/v3/repos/yadvlabs/reef/contents/README.md"),
            (POST, "/api/v3/repos/yadvlabs/reef/git/refs"),
            (PATCH, "/api/v3/repos/yadvlabs/reef/git/refs/heads/main"),
            (DELETE, "/api/v3/repos/yadvlabs/reef/git/refs/heads/main"),
            (POST, "/api/v3/repos/yadvlabs/reef/releases"),
            (POST, "/api/reef/repos/yadvlabs/reef/wiki"),
            (PUT, "/api/reef/repos/yadvlabs/reef/wiki/Home"),
            (DELETE, "/api/reef/repos/yadvlabs/reef/wiki/Home"),
            (POST, "/api/reef/orgs/yadvlabs/wiki"),
            (PUT, "/api/reef/orgs/yadvlabs/wiki/Home"),
            (DELETE, "/api/reef/orgs/yadvlabs/wiki/Home"),
            (GET, "/api/v3/repos/yadvlabs/reef/stats/contributors"),
            (GET, "/api/v3/repos/yadvlabs/reef/stats/commit_activity"),
            (GET, "/api/v3/repos/yadvlabs/reef/stats/code_frequency"),
            (GET, "/api/v3/repos/yadvlabs/reef/stats/participation"),
            (GET, "/api/v3/repos/yadvlabs/reef/stats/punch_card"),
            (GET, "/api/reef/repos/yadvlabs/reef/pulse"),
            (GET, "/api/reef/repos/yadvlabs/reef/tree-commits"),
            (GET, "/api/reef/users/rohit/contributions"),
            (GET, "/api/reef/repos/yadvlabs/reef/security"),
            (POST, "/api/graphql"),
            (GET, "/api/v3/user/repos"),
            (GET, "/api/v3/repos/yadvlabs/reef"),
            (PATCH, "/api/v3/repos/yadvlabs/reef"),
            (GET, "/api/v3/orgs/yadvlabs"),
            (GET, "/api/v3/repos/yadvlabs/reef/import"),
            (GET, "/api/v3/repos/yadvlabs/reef/contents/README.md"),
            (GET, "/api/v3/repos/yadvlabs/reef/pulls/7/merge"),
            (GET, "/api/v3/repos/yadvlabs/reef/git/refs"),
            (GET, "/api/v3/repos/yadvlabs/reef/releases"),
            (GET, "/api/v3/repos/yadvlabs/reef/issues"),
            (POST, "/api/v3/repos/yadvlabs/reef/issues"),
            (GET, "/api/v3/repos/yadvlabs/reef/actions/artifacts/1/zip"),
            (GET, "/api/reef/repos/yadvlabs/reef/mirror"),
            (GET, "/api/reef/repos/yadvlabs/reef/wiki"),
            (GET, "/api/reef/session"),
            (GET, "/api/graphql"),
            (GET, "/login/oidc"),
            (GET, "/yadvlabs/reef"),
            (GET, "/"),
        ];
        for (m, p) in exempt {
            let res = app.clone().oneshot(req(m.clone(), p)).await.unwrap();
            let policy = res.extensions().get::<super::timeout::TimeoutPolicy>().copied();
            assert_eq!(policy.map(|x| x.exempt), Some(true), "{m} {p} should be exempt (status {})", res.status());
        }
        for (m, p) in timed {
            let res = app.clone().oneshot(req(m.clone(), p)).await.unwrap();
            let policy = res.extensions().get::<super::timeout::TimeoutPolicy>().copied();
            assert_eq!(
                policy,
                Some(super::timeout::TimeoutPolicy { exempt: false, timed_out: false }),
                "{m} {p} should be timed (status {})",
                res.status()
            );
        }
    }

    #[tokio::test]
    async fn runner_registration_passes_the_rate_limiter() {
        let s = super::testing::state("runnerreg", |_| {}).await;
        let (u, _) = super::testing::user_with_token(&s, "runneradmin").await;
        let repo = s.db.create_repo(u.id(), reef_db::repos::NewRepo { name: "ci".into(), ..Default::default() }).await.unwrap();
        let app = super::router(s.clone());
        let post = |token: String, event: &str| {
            let body = serde_json::json!({"url": "http://localhost:4920/runneradmin/ci", "runner_event": event}).to_string();
            let mut r = Request::post("/api/v3/actions/runner-registration")
                .header("authorization", format!("RemoteAuth {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
            r.extensions_mut().insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 78], 40000))));
            r
        };
        // `RemoteAuth` is not a scheme the API resolver knows: the limiter charges the request
        // as anonymous and leaves the decision to the handler, which accepts the token.
        let (register, _) = s.db.create_runner_token("repo", repo.id(), "register", Some(u.id()), 3_600_000).await.unwrap();
        let res = app.clone().oneshot(post(register.clone(), "register")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "registration reaches the handler");
        assert_eq!(res.headers()["x-ratelimit-limit"], "60", "charged as an unauthenticated caller");
        let body: serde_json::Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert!(body["token"].is_string() && body["url"].as_str().unwrap().contains("/_actions/repo/"), "{body}");
        let (remove, _) = s.db.create_runner_token("repo", repo.id(), "remove", Some(u.id()), 3_600_000).await.unwrap();
        let res = app.clone().oneshot(post(remove, "remove")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "removal works the same way");
        // A token for the wrong purpose is refused by the handler, with its own message.
        let res = app.clone().oneshot(post(register, "remove")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(body.contains("this token is for register"), "{body}");
    }
}

//! API rate limiting, GitHub style: authenticated callers get `[limits] requests_per_hour`
//! per user (every token and session of that user draws from one budget; a job token draws
//! from its job's), unauthenticated callers get `unauthenticated_requests_per_hour` per
//! client address. Each budget is a token bucket that refills continuously at `limit / 3600`
//! per second, so a client that stays under the rate never sees a 429 and a burst of up to
//! `limit` is absorbed.
//!
//! The limiter is attached to the API routers (`/api/v3`, `/api/graphql`, `/api/reef`) by the
//! application router, so a new API route is limited by construction and nothing outside those
//! routers (health, git smart HTTP, Git LFS, the runner protocol, the UI) ever is.
//!
//! Signed-out calls from the embedded web UI (a same-origin browser request, `Sec-Fetch-Site:
//! same-origin`, or one carrying the session cookie) are charged to a separate per-address
//! budget, `web_requests_per_hour` (default 5000), so browsing public repositories in the UI
//! is not held to the anonymous 60/h. Those headers can be sent by any client; what that buys
//! is the web budget per address, the same size as a signed-in user's, never more.
//!
//! Anonymous callers are keyed by address: an IPv4 address, or the /64 of an IPv6 one (a
//! single host usually holds a whole /64, so per-address keys would let it rotate freely).
//! IPv4-mapped IPv6 peers (`::ffff:a.b.c.d`, what a dual-stack listener reports for IPv4
//! clients) are the IPv4 address everywhere: trusted-proxy matching, loopback checks, keys.
//!
//! Credentials are resolved only when the request still has budget: a request carrying
//! credentials this instance has not verified recently is first charged to its source
//! address (refunded once the credentials resolve to a user or job); one carrying credentials
//! verified in the last few minutes is charged to that user's budget first. A request that
//! will be answered 429 therefore never costs a password hash.
//!
//! Buckets are in memory, per instance, by design: behind a load balancer each instance
//! enforces the limit on the traffic it sees, so the effective limit is up to N times the
//! configured one. They live in sharded maps capped at `MAX_BUCKETS` in total (the least
//! recently used bucket of a full shard makes room), and idle buckets are swept by the
//! server's periodic tick (`RateLimiter::sweep`), not on the request path.
//!
//! Every limited response carries `X-RateLimit-Limit`, `-Remaining`, `-Used`, `-Reset`
//! (epoch seconds when the bucket is full again) and `-Resource: core`; a refusal is `429`
//! with `Retry-After`. `GET /api/v3/rate_limit` reports the state without spending a request.

use crate::AppState;
use crate::auth::Auth;
use crate::metrics::registry;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Extension, FromRequestParts, MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const RESOURCE: &str = "core";
const WINDOW: Duration = Duration::from_secs(3600);
/// The matched route of `GET /api/v3/rate_limit`, which reports without spending.
const REPORT_ROUTE: &str = "/api/v3/rate_limit";
/// Buckets kept at most, over all shards.
pub const MAX_BUCKETS: usize = 100_000;
/// Independent locks over the bucket map, so concurrent requests rarely contend.
const SHARDS: usize = 64;
/// How long a verified credential is remembered for the pre-check (see `rate_limit`).
const VERIFIED_TTL: Duration = Duration::from_secs(300);
/// Remembered credentials at most.
const MAX_VERIFIED: usize = 100_000;

/// The peer of a request as an extractor: the socket address, absent on the Unix socket.
pub type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

pub fn peer_ip(peer: &Peer) -> Option<IpAddr> {
    peer.as_ref().map(|p| canonical_ip(p.0.0.ip()))
}

/// An address as the rest of Reef compares it: an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`,
/// what a dual-stack socket reports for an IPv4 client, or what a PROXY v2 header may carry)
/// is the IPv4 address. Shared by the client-address walk, the trusted-proxy and loopback
/// checks and the PROXY protocol policy.
pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    ip.to_canonical()
}

/// The address an anonymous caller is keyed by: an IPv4 address, or the /64 of an IPv6 one.
pub fn key_addr(ip: IpAddr) -> IpAddr {
    match canonical_ip(ip) {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !(u64::MAX as u128))),
        v4 => v4,
    }
}

/// Who a request is charged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A signed-in user: every token and session of theirs shares one budget.
    User(i64),
    /// An Actions job token: one budget per job.
    Job(i64),
    /// An unauthenticated caller by address (`key_addr`: IPv6 by /64).
    Ip(IpAddr),
    /// An unauthenticated caller on the Unix socket listener with no forwarded address to
    /// go by: the listener shares one budget (local proxies should forward the address and be
    /// listed in `[http] trusted_proxies`).
    Unix,
    /// A signed-out call from the embedded web UI, by address (`None`: the Unix socket).
    Web(Option<IpAddr>),
}

impl Key {
    /// The web-UI budget of an anonymous key.
    fn web(self) -> Key {
        match self {
            Key::Ip(ip) => Key::Web(Some(ip)),
            Key::Unix => Key::Web(None),
            other => other,
        }
    }
}

/// The key the middleware charged a request to, left in the request extensions for
/// `/rate_limit` and GraphQL's `rateLimit`.
#[derive(Debug, Clone, Copy)]
pub struct RateKey(pub Key);

/// One caller's budget as of the last decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub limit: u32,
    pub remaining: u32,
    pub used: u32,
    /// Epoch seconds when the budget is full again.
    pub reset: i64,
    /// Seconds until one more request is allowed; `Some` only when refused.
    pub retry_after: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The limit for this class of caller is `0`: nothing is counted, no headers.
    Unlimited,
    Allowed(Status),
    Refused(Status),
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// A credential, by its SHA-256 (the secret itself is never kept).
type Fingerprint = [u8; 32];

pub struct RateLimiter {
    authenticated: u32,
    anonymous: u32,
    web: u32,
    trusted: Vec<ipnet::IpNet>,
    /// `[http] forwarded_header`: `Forwarded` (RFC 7239) instead of `X-Forwarded-For`.
    rfc7239: bool,
    shards: Vec<Mutex<HashMap<Key, Bucket>>>,
    shard_cap: usize,
    hasher: RandomState,
    /// Credentials that resolved recently, and to whom (see `rate_limit`).
    verified: Mutex<HashMap<Fingerprint, (Key, Instant)>>,
    /// Credential resolutions the middleware ran (tests read it).
    resolutions: AtomicU64,
}

impl RateLimiter {
    pub fn new(cfg: &crate::Config) -> RateLimiter {
        Self::with_capacity(cfg, MAX_BUCKETS)
    }

    fn with_capacity(cfg: &crate::Config, max_buckets: usize) -> RateLimiter {
        RateLimiter {
            authenticated: cfg.limits.requests_per_hour,
            anonymous: cfg.limits.unauthenticated_requests_per_hour,
            web: cfg.limits.web_requests_per_hour,
            trusted: cfg.http.trusted_proxy_nets().unwrap_or_default(),
            rfc7239: cfg.http.forwarded_header == "forwarded",
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            shard_cap: max_buckets.div_ceil(SHARDS).max(1),
            hasher: RandomState::new(),
            verified: Mutex::new(HashMap::new()),
            resolutions: AtomicU64::new(0),
        }
    }

    fn shard(&self, key: &Key) -> std::sync::MutexGuard<'_, HashMap<Key, Bucket>> {
        let i = (self.hasher.hash_one(key) as usize) % self.shards.len();
        self.shards[i].lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a bucket can be forgotten: idle for a window, it is back at capacity.
    fn idle(&self, key: &Key, b: &Bucket, now: Instant) -> bool {
        now.duration_since(b.at) >= WINDOW || b.tokens >= self.limit_for(*key) as f64
    }

    /// Drop idle buckets and expired credential entries. Run from the server's periodic tick;
    /// each shard is locked on its own, briefly.
    pub fn sweep(&self) {
        self.sweep_at(Instant::now());
    }

    fn sweep_at(&self, now: Instant) {
        for shard in &self.shards {
            let mut m = shard.lock().unwrap_or_else(|e| e.into_inner());
            m.retain(|k, b| !self.idle(k, b, now));
        }
        let mut v = self.verified.lock().unwrap_or_else(|e| e.into_inner());
        v.retain(|_, (_, at)| now.duration_since(*at) < VERIFIED_TTL);
    }

    /// Buckets currently kept.
    pub fn bucket_count(&self) -> usize {
        self.shards.iter().map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).len()).sum()
    }

    pub fn limit_for(&self, key: Key) -> u32 {
        match key {
            Key::User(_) | Key::Job(_) => self.authenticated,
            Key::Web(_) => self.web,
            Key::Ip(_) | Key::Unix => self.anonymous,
        }
    }

    /// The client address of a request, shared by the rate limits, the request log,
    /// request-id trust and sessions. The socket peer is the client unless it is a trusted
    /// proxy; then the configured header (`[http] forwarded_header`) is walked from the right
    /// past trusted hops, so a client cannot forge an entry. An untrusted hop is the client; an
    /// unparsable hop (or a `Forwarded` element without `for=`) stops the walk and the nearest
    /// trusted hop is the client; when every hop is trusted the leftmost entry is. A
    /// Unix-socket peer counts as trusted when `trusted_proxies` is set. Addresses are
    /// canonical (`canonical_ip`). `None`: the Unix socket with nothing to go by.
    pub fn client_ip(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        let peer = peer.map(canonical_ip);
        if self.trusted.is_empty() || peer.is_some_and(|p| !self.is_trusted(p)) {
            return peer;
        }
        let chain = self.forwarded_chain(headers);
        let mut nearest = peer;
        for hop in chain.iter().rev() {
            match hop {
                None => return nearest,
                Some(ip) if !self.is_trusted(*ip) => return Some(*ip),
                Some(ip) => nearest = Some(*ip),
            }
        }
        nearest
    }

    /// The key an unauthenticated request is charged to: its client address (`key_addr`), or
    /// the shared `Key::Unix` budget of the Unix socket.
    pub fn client(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> Key {
        self.client_ip(peer, headers).map(|ip| Key::Ip(key_addr(ip))).unwrap_or(Key::Unix)
    }

    /// The hops of the configured forwarding header, left to right (every instance of the
    /// header, in order): `Some(address)`, or `None` for an entry that is not an address
    /// (`unknown`, obfuscated, garbage, a `Forwarded` element without `for=`).
    fn forwarded_chain(&self, headers: &HeaderMap) -> Vec<Option<IpAddr>> {
        let name = if self.rfc7239 { "forwarded" } else { "x-forwarded-for" };
        let mut out = Vec::new();
        for v in headers.get_all(name).iter() {
            let Ok(v) = v.to_str() else {
                out.push(None);
                continue;
            };
            for element in v.split(',') {
                let raw = if self.rfc7239 {
                    match element.split(';').filter_map(|p| p.split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("for")) {
                        Some((_, v)) => v.trim().trim_matches('"'),
                        None => {
                            out.push(None);
                            continue;
                        }
                    }
                } else {
                    element.trim()
                };
                out.push(parse_node(raw).map(canonical_ip));
            }
        }
        out
    }

    /// Whether `ip` (canonicalised) is one of `[http] trusted_proxies`.
    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        let ip = canonical_ip(ip);
        self.trusted.iter().any(|n| n.contains(&ip))
    }

    /// Whether a request's peer may vouch for headers such as `X-Request-Id`: a trusted
    /// proxy or loopback over TCP (canonical, so `::ffff:127.0.0.1` is loopback), and the Unix
    /// socket (`None`) only when `trusted_proxies` is set.
    pub fn trusts_peer(&self, peer: Option<IpAddr>) -> bool {
        match peer.map(canonical_ip) {
            Some(ip) => ip.is_loopback() || self.is_trusted(ip),
            None => self.trusts_proxies(),
        }
    }

    /// Whether any proxy is trusted (a Unix-socket peer then counts as one).
    pub fn trusts_proxies(&self) -> bool {
        !self.trusted.is_empty()
    }

    /// Spend one request of `key`'s budget.
    pub fn check(&self, key: Key) -> Decision {
        self.check_at(key, Instant::now(), reef_core::time::now_ms() / 1000)
    }

    /// Report `key`'s budget without spending anything.
    pub fn status(&self, key: Key) -> Decision {
        self.peek_at(key, Instant::now(), reef_core::time::now_ms() / 1000)
    }

    fn check_at(&self, key: Key, now: Instant, epoch: i64) -> Decision {
        let limit = self.limit_for(key);
        if limit == 0 {
            return Decision::Unlimited;
        }
        let rate = limit as f64 / WINDOW.as_secs_f64();
        let mut buckets = self.shard(&key);
        if !buckets.contains_key(&key) && buckets.len() >= self.shard_cap {
            // Full: forget the idle buckets, else the least recently used one.
            buckets.retain(|k, b| !self.idle(k, b, now));
            if buckets.len() >= self.shard_cap
                && let Some(lru) = buckets.iter().min_by_key(|(_, b)| b.at).map(|(k, _)| *k)
            {
                buckets.remove(&lru);
            }
        }
        let b = buckets.entry(key).or_insert(Bucket { tokens: limit as f64, at: now });
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * rate).min(limit as f64);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Decision::Allowed(status(limit, b.tokens, rate, epoch, None))
        } else {
            let wait = ((1.0 - b.tokens) / rate).ceil().max(1.0) as u64;
            Decision::Refused(status(limit, b.tokens, rate, epoch, Some(wait)))
        }
    }

    /// Give back the request `check` took from `key` (it was charged to the wrong budget).
    fn refund(&self, key: Key) {
        let limit = self.limit_for(key) as f64;
        if let Some(b) = self.shard(&key).get_mut(&key) {
            b.tokens = (b.tokens + 1.0).min(limit);
        }
    }

    fn peek_at(&self, key: Key, now: Instant, epoch: i64) -> Decision {
        let limit = self.limit_for(key);
        if limit == 0 {
            return Decision::Unlimited;
        }
        let rate = limit as f64 / WINDOW.as_secs_f64();
        let buckets = self.shard(&key);
        let tokens = match buckets.get(&key) {
            Some(b) => (b.tokens + now.duration_since(b.at).as_secs_f64() * rate).min(limit as f64),
            None => limit as f64,
        };
        Decision::Allowed(status(limit, tokens, rate, epoch, None))
    }

    /// Whom a credential resolved to recently, if it did.
    fn verified_key(&self, fp: &Fingerprint) -> Option<Key> {
        let v = self.verified.lock().unwrap_or_else(|e| e.into_inner());
        v.get(fp).filter(|(_, at)| at.elapsed() < VERIFIED_TTL).map(|(k, _)| *k)
    }

    /// Remember (a user or job) or forget (anything else) what a credential resolved to.
    fn record_verified(&self, fp: Fingerprint, key: Key) {
        let mut v = self.verified.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(key, Key::User(_) | Key::Job(_)) {
            v.remove(&fp);
            return;
        }
        if v.len() >= MAX_VERIFIED && !v.contains_key(&fp) {
            v.retain(|_, (_, at)| at.elapsed() < VERIFIED_TTL);
            if v.len() >= MAX_VERIFIED {
                v.clear();
            }
        }
        v.insert(fp, (key, Instant::now()));
    }

    /// How many times the middleware resolved credentials.
    pub fn credential_resolutions(&self) -> u64 {
        self.resolutions.load(Ordering::Relaxed)
    }
}

/// An address as forwarding headers write it: `1.2.3.4`, `1.2.3.4:80`, `2001:db8::1`,
/// `[2001:db8::1]` or `[2001:db8::1]:443`.
fn parse_node(raw: &str) -> Option<IpAddr> {
    let raw = raw.trim();
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Some(rest) = raw.strip_prefix('[') {
        return rest.split(']').next()?.parse().ok();
    }
    raw.parse::<SocketAddr>().ok().map(|a| a.ip())
}

fn status(limit: u32, tokens: f64, rate: f64, epoch: i64, retry_after: Option<u64>) -> Status {
    let remaining = tokens.floor().max(0.0) as u32;
    let refill = ((limit as f64 - tokens) / rate).ceil().max(0.0) as i64;
    Status { limit, remaining, used: limit - remaining, reset: epoch + refill, retry_after }
}

impl Status {
    /// The `X-RateLimit-*` headers (and `Retry-After` when refused).
    pub fn apply(&self, h: &mut HeaderMap) {
        let set = |h: &mut HeaderMap, k: &'static str, v: String| {
            if let Ok(v) = HeaderValue::from_str(&v) {
                h.insert(k, v);
            }
        };
        set(h, "x-ratelimit-limit", self.limit.to_string());
        set(h, "x-ratelimit-remaining", self.remaining.to_string());
        set(h, "x-ratelimit-used", self.used.to_string());
        set(h, "x-ratelimit-reset", self.reset.to_string());
        h.insert("x-ratelimit-resource", HeaderValue::from_static(RESOURCE));
        if let Some(s) = self.retry_after {
            set(h, "retry-after", s.to_string());
        }
    }

    /// The `/rate_limit` resource object.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"limit": self.limit, "used": self.used, "remaining": self.remaining, "reset": self.reset})
    }
}

impl Decision {
    pub fn status(&self) -> Option<Status> {
        match self {
            Decision::Unlimited => None,
            Decision::Allowed(s) | Decision::Refused(s) => Some(*s),
        }
    }
}

/// The client address to record with a session: the socket peer, or the forwarded address
/// behind a trusted proxy (never from an untrusted peer's header). `None` on the Unix socket.
pub fn client_address(s: &AppState, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<String> {
    s.limiter.client_ip(peer, headers).map(|ip| ip.to_string())
}

/// Whether a signed-out request comes from the embedded web UI: a same-origin browser fetch,
/// or one carrying the session cookie.
fn from_web_ui(headers: &HeaderMap) -> bool {
    let same_origin = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("same-origin");
    let cookie = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .any(|kv| kv.trim().split_once('=').is_some_and(|(k, _)| k == crate::auth::COOKIE));
    same_origin || cookie
}

/// Who an authenticated request is charged to; `anonymous` when nobody is signed in.
fn key_for(auth: &Auth, anonymous: Key) -> Key {
    match &auth.user {
        Some(u) => match u.job_id {
            Some(job) => Key::Job(job),
            None => Key::User(u.id),
        },
        None => anonymous,
    }
}

/// The credential a request carries, fingerprinted: the `Authorization` header, else the
/// session cookie (what `Auth` resolves, in the same order). `None`: nothing to resolve.
fn credential(headers: &HeaderMap) -> Option<Fingerprint> {
    use sha2_010::{Digest, Sha256};
    let (kind, value) = match headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        Some(v) => ("authorization", v.to_string()),
        None => (
            "cookie",
            headers
                .get_all(header::COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(';'))
                .filter_map(|kv| kv.trim().split_once('='))
                .find(|(k, _)| *k == crate::auth::COOKIE)
                .map(|(_, v)| v.to_string())?,
        ),
    };
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(value.as_bytes());
    Some(h.finalize().into())
}

/// The middleware, attached to the API routers. Charges a bucket *before* resolving
/// credentials, so a request that will be refused never costs a password
/// hash:
/// - no credentials: the source (client address, web budget or Unix socket) is charged;
/// - credentials verified in the last `VERIFIED_TTL`: their user's (or job's) bucket is
///   charged first, then they are resolved (the handler needs the `Auth`);
/// - credentials not verified recently: the source is charged first; when they resolve to a
///   user or job that token is given back and the user's budget charged instead.
///
/// The resolved `Auth` is cached on the request for the handler. Credentials that do not
/// resolve here (bad tokens, or schemes this resolver does not know such as the runner's
/// `RemoteAuth`) are never refused here: the request stays charged to the source and the
/// handler decides.
pub async fn rate_limit(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| canonical_ip(c.0.ip()));
    let mut source = s.limiter.client(peer, req.headers());
    if from_web_ui(req.headers()) {
        source = source.web();
    }
    let report_only = req.extensions().get::<MatchedPath>().is_some_and(|m| m.as_str() == REPORT_ROUTE);
    let charge = |key: Key| if report_only { s.limiter.status(key) } else { s.limiter.check(key) };
    let fingerprint = credential(req.headers());
    let (mut parts, body) = req.into_parts();
    let (key, decision) = match fingerprint {
        None => (source, charge(source)),
        Some(fp) => {
            let known = s.limiter.verified_key(&fp);
            // Unknown credentials are a verification attempt by the source: it pays, even
            // on `/rate_limit`, until they resolve.
            let gate = known.unwrap_or(source);
            let spent = known.is_none() || !report_only;
            let pre = if spent { s.limiter.check(gate) } else { s.limiter.status(gate) };
            if let Decision::Refused(st) = pre {
                registry().http_rate_limited.inc();
                return refused(gate, st);
            }
            s.limiter.resolutions.fetch_add(1, Ordering::Relaxed);
            let resolved = match Auth::from_request_parts(&mut parts, &s).await {
                Ok(a) => key_for(&a, source),
                Err(_) => source,
            };
            s.limiter.record_verified(fp, resolved);
            if resolved == gate {
                (gate, pre)
            } else {
                if spent {
                    s.limiter.refund(gate);
                }
                (resolved, charge(resolved))
            }
        }
    };
    parts.extensions.insert(RateKey(key));
    let req = Request::from_parts(parts, body);
    match decision {
        Decision::Unlimited => next.run(req).await,
        Decision::Allowed(st) => {
            let mut res = next.run(req).await;
            st.apply(res.headers_mut());
            res
        }
        Decision::Refused(st) => {
            registry().http_rate_limited.inc();
            refused(key, st)
        }
    }
}

/// A keyed address as a person reads it: IPv6 keys are /64 prefixes.
fn shown(ip: IpAddr) -> String {
    match ip {
        IpAddr::V6(_) => format!("{ip}/64"),
        v4 => v4.to_string(),
    }
}

fn refused(key: Key, st: Status) -> Response {
    let who = match key {
        Key::User(_) => "this user".to_string(),
        Key::Job(_) => "this job".to_string(),
        Key::Ip(ip) | Key::Web(Some(ip)) => shown(ip),
        Key::Unix => "the local socket".to_string(),
        Key::Web(None) => "the local socket".to_string(),
    };
    let body = serde_json::json!({
        "message": format!("API rate limit exceeded for {who}. Retry after {} seconds.", st.retry_after.unwrap_or(1)),
        "documentation_url": "https://docs.github.com/rest/overview/rate-limits-for-the-rest-api",
    });
    let mut res = (StatusCode::TOO_MANY_REQUESTS, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response();
    st.apply(res.headers_mut());
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn limiter(authenticated: u32, anonymous: u32, trusted: &[&str]) -> RateLimiter {
        let mut cfg = Config::default();
        cfg.limits.requests_per_hour = authenticated;
        cfg.limits.unauthenticated_requests_per_hour = anonymous;
        cfg.http.trusted_proxies = trusted.iter().map(|s| s.to_string()).collect();
        RateLimiter::new(&cfg)
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn buckets_drain_refill_and_report_github_headers() {
        let l = limiter(5000, 60, &[]);
        let t0 = Instant::now();
        let key = Key::Ip(ip("203.0.113.9"));
        for i in 0..60 {
            match l.check_at(key, t0, 1_000_000) {
                Decision::Allowed(st) => {
                    assert_eq!((st.limit, st.remaining, st.used), (60, 59 - i, i + 1));
                    assert_eq!(st.retry_after, None);
                }
                other => panic!("request {i}: {other:?}"),
            }
        }
        let Decision::Refused(st) = l.check_at(key, t0, 1_000_000) else { panic!("61st request is refused") };
        assert_eq!(st.remaining, 0);
        assert_eq!(st.used, 60);
        assert_eq!(st.retry_after, Some(60), "one token every 60 s at 60/h");
        assert_eq!(st.reset, 1_000_000 + 3600, "full again in an hour");
        // 60 s later one request is allowed again, then refused again.
        assert!(matches!(l.check_at(key, t0 + Duration::from_secs(60), 1_000_060), Decision::Allowed(_)));
        assert!(matches!(l.check_at(key, t0 + Duration::from_secs(60), 1_000_060), Decision::Refused(_)));
        // An hour later the bucket is full.
        let Decision::Allowed(st) = l.check_at(key, t0 + Duration::from_secs(3700), 1_003_700) else { panic!() };
        assert_eq!(st.remaining, 59);
        // Users and jobs have their own, larger budgets; other addresses are independent.
        let Decision::Allowed(st) = l.check_at(Key::User(7), t0, 1_000_000) else { panic!() };
        assert_eq!((st.limit, st.remaining), (5000, 4999));
        let Decision::Allowed(st) = l.check_at(Key::Job(7), t0, 1_000_000) else { panic!() };
        assert_eq!((st.limit, st.remaining), (5000, 4999), "a job's budget is not the user's");
        let Decision::Allowed(st) = l.check_at(Key::Ip(ip("203.0.113.10")), t0, 1_000_000) else { panic!() };
        assert_eq!(st.remaining, 59);
        let Decision::Allowed(st) = l.check_at(Key::Unix, t0, 1_000_000) else { panic!() };
        assert_eq!((st.limit, st.remaining), (60, 59));
        // Peeking does not spend.
        let Decision::Allowed(st) = l.peek_at(Key::User(7), t0, 1_000_000) else { panic!() };
        assert_eq!(st.remaining, 4999);
        let Decision::Allowed(st) = l.peek_at(Key::User(8), t0, 1_000_000) else { panic!() };
        assert_eq!((st.remaining, st.used, st.reset), (5000, 0, 1_000_000));
        let mut h = HeaderMap::new();
        Status { limit: 60, remaining: 0, used: 60, reset: 42, retry_after: Some(9) }.apply(&mut h);
        assert_eq!(h["x-ratelimit-limit"], "60");
        assert_eq!(h["x-ratelimit-remaining"], "0");
        assert_eq!(h["x-ratelimit-used"], "60");
        assert_eq!(h["x-ratelimit-reset"], "42");
        assert_eq!(h["x-ratelimit-resource"], "core");
        assert_eq!(h["retry-after"], "9");
    }

    #[test]
    fn zero_means_unlimited_and_idle_buckets_are_swept() {
        let l = limiter(0, 0, &[]);
        assert_eq!(l.check(Key::User(1)), Decision::Unlimited);
        assert_eq!(l.check(Key::Ip(ip("10.0.0.1"))), Decision::Unlimited);
        let l = limiter(100, 10, &[]);
        let t0 = Instant::now();
        for i in 0..500 {
            l.check_at(Key::User(i), t0, 0);
        }
        assert_eq!(l.bucket_count(), 500, "nothing is swept on the request path");
        l.check_at(Key::User(-1), t0 + WINDOW / 2, 1800);
        // The periodic sweep, an hour later: every bucket idle for a window is full again and gone.
        l.sweep_at(t0 + WINDOW);
        assert_eq!(l.bucket_count(), 1);
    }

    /// The map is capped: a full shard makes room by dropping its least recently used bucket.
    #[test]
    fn the_bucket_map_is_capped_least_recently_used_first() {
        let mut cfg = Config::default();
        cfg.limits.requests_per_hour = 100;
        let l = RateLimiter::with_capacity(&cfg, SHARDS * 4);
        let t0 = Instant::now();
        for i in 0..(SHARDS * 40) as i64 {
            l.check_at(Key::User(i), t0 + Duration::from_millis(i as u64), 0);
        }
        assert!(l.bucket_count() <= SHARDS * 4, "capped: {}", l.bucket_count());
        // The most recent callers kept their (spent) budgets.
        let last = Key::User((SHARDS * 40) as i64 - 1);
        let Decision::Allowed(st) = l.peek_at(last, t0 + Duration::from_millis((SHARDS * 40) as u64), 0) else { panic!() };
        assert_eq!(st.remaining, 99);
        assert_eq!(MAX_BUCKETS, 100_000);
        assert_eq!(RateLimiter::new(&cfg).shard_cap * SHARDS, MAX_BUCKETS.div_ceil(SHARDS) * SHARDS);
    }

    /// Anonymous IPv6 callers are keyed by /64; IPv4-mapped peers are IPv4 everywhere.
    #[test]
    fn ipv6_clients_are_keyed_by_prefix_and_mapped_peers_are_ipv4() {
        let l = limiter(5000, 60, &["10.0.0.0/8", "127.0.0.1"]);
        let none = HeaderMap::new();
        let a = l.client(Some(ip("2001:db8:1:2:aaaa::1")), &none);
        let b = l.client(Some(ip("2001:db8:1:2:bbbb::9")), &none);
        let c = l.client(Some(ip("2001:db8:1:3::1")), &none);
        assert_eq!(a, Key::Ip(ip("2001:db8:1:2::")));
        assert_eq!(a, b, "one /64, one budget");
        assert_ne!(a, c);
        assert_eq!(shown(ip("2001:db8:1:2::")), "2001:db8:1:2::/64");
        // The client address itself (logs, sessions) keeps the full address.
        assert_eq!(l.client_ip(Some(ip("2001:db8:1:2:aaaa::1")), &none), Some(ip("2001:db8:1:2:aaaa::1")));
        // A dual-stack listener reports IPv4 peers as ::ffff:a.b.c.d.
        assert_eq!(l.client(Some(ip("::ffff:203.0.113.9")), &none), Key::Ip(ip("203.0.113.9")));
        assert!(l.is_trusted(ip("::ffff:10.1.2.3")), "trusted-proxy matching sees the IPv4 address");
        let xff = hdr("x-forwarded-for", Some("198.51.100.7, ::ffff:10.0.0.9"));
        assert_eq!(
            l.client(Some(ip("::ffff:10.0.0.5")), &xff),
            Key::Ip(ip("198.51.100.7")),
            "a mapped proxy peer is trusted, mapped hops too"
        );
        assert!(l.trusts_peer(Some(ip("::ffff:127.0.0.1"))), "loopback, mapped");
        assert!(!l.trusts_peer(Some(ip("::ffff:203.0.113.9"))));
        assert!(l.trusts_peer(None), "the Unix socket, with proxies configured");
        assert!(!limiter(5000, 60, &[]).trusts_peer(None), "the Unix socket, without");
    }

    fn hdr(name: &str, value: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = value {
            h.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn forwarded_for_is_honoured_only_from_trusted_proxies() {
        let xff = |v: Option<&str>| hdr("x-forwarded-for", v);
        let l = limiter(5000, 60, &[]);
        assert_eq!(l.client(Some(ip("10.0.0.5")), &xff(Some("198.51.100.7"))), Key::Ip(ip("10.0.0.5")), "no trusted proxies: peer wins");
        assert_eq!(l.client(None, &xff(Some("198.51.100.7"))), Key::Unix, "unix socket, no trusted proxies: one shared budget");
        let l = limiter(5000, 60, &["10.0.0.0/8", "192.0.2.1"]);
        assert_eq!(l.client(Some(ip("10.0.0.5")), &xff(Some("198.51.100.7"))), Key::Ip(ip("198.51.100.7")));
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &xff(Some("203.0.113.1, 198.51.100.7, 10.1.1.1"))),
            Key::Ip(ip("198.51.100.7")),
            "walk from the right past trusted hops; the client's own entry is not believed"
        );
        assert_eq!(l.client(Some(ip("192.0.2.1")), &xff(Some("[2001:db8::1]"))), Key::Ip(ip("2001:db8::")));
        assert_eq!(l.client(Some(ip("192.0.2.1")), &xff(Some("198.51.100.9:443"))), Key::Ip(ip("198.51.100.9")), "ports are dropped");
        assert_eq!(l.client(Some(ip("10.0.0.5")), &xff(Some("garbage"))), Key::Ip(ip("10.0.0.5")), "unparseable: the peer");
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &xff(Some("198.51.100.7, garbage, 10.0.0.9"))),
            Key::Ip(ip("10.0.0.9")),
            "an unparsable hop stops the walk: the nearest trusted hop, not an entry beyond it"
        );
        assert_eq!(l.client(Some(ip("10.0.0.5")), &xff(None)), Key::Ip(ip("10.0.0.5")));
        assert_eq!(
            l.client(Some(ip("203.0.113.9")), &xff(Some("198.51.100.7"))),
            Key::Ip(ip("203.0.113.9")),
            "untrusted peer: header ignored"
        );
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &xff(Some("10.0.0.7, 10.0.0.9"))),
            Key::Ip(ip("10.0.0.7")),
            "every hop trusted: the leftmost"
        );
        // The Unix socket is a local hop: with proxies configured its forwarded address counts.
        assert_eq!(l.client(None, &xff(Some("198.51.100.7, 10.0.0.9"))), Key::Ip(ip("198.51.100.7")));
        assert_eq!(l.client(None, &xff(Some("10.0.0.9"))), Key::Ip(ip("10.0.0.9")), "only trusted hops: the leftmost");
        assert_eq!(l.client(None, &xff(None)), Key::Unix);
        // `forwarded_header = "forwarded"`: RFC 7239 `for=` parameters, X-Forwarded-For ignored.
        let mut cfg = Config::default();
        cfg.http.trusted_proxies = vec!["10.0.0.0/8".into()];
        cfg.http.forwarded_header = "forwarded".into();
        let l = RateLimiter::new(&cfg);
        let fwd = |v: &str| hdr("forwarded", Some(v));
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &fwd(r#"for=198.51.100.7;proto=https, for="[2001:db8::2]:4711""#)),
            Key::Ip(ip("2001:db8::")),
            "keyed by /64"
        );
        assert_eq!(l.client(Some(ip("10.0.0.5")), &fwd("for=198.51.100.7, for=10.0.0.3;by=10.0.0.5")), Key::Ip(ip("198.51.100.7")));
        assert_eq!(l.client(Some(ip("10.0.0.5")), &fwd("for=unknown")), Key::Ip(ip("10.0.0.5")));
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &fwd("for=198.51.100.7, proto=https, for=10.0.0.3")),
            Key::Ip(ip("10.0.0.3")),
            "an element without for= stops the walk"
        );
        assert_eq!(
            l.client(Some(ip("10.0.0.5")), &fwd("for=198.51.100.7, for=_hidden")),
            Key::Ip(ip("10.0.0.5")),
            "an obfuscated hop right after the peer: the peer"
        );
        assert_eq!(l.client(Some(ip("10.0.0.5")), &xff(Some("198.51.100.7"))), Key::Ip(ip("10.0.0.5")), "the other header is not read");
    }

    #[tokio::test]
    async fn keys_follow_the_caller() {
        let s = crate::testing::state("ratekeys", |_| {}).await;
        let (u, _) = crate::testing::user_with_token(&s, "keyed").await;
        let mut a = Auth::anonymous();
        assert_eq!(key_for(&a, Key::Unix), Key::Unix);
        assert_eq!(key_for(&a, Key::Ip(ip("203.0.113.1"))), Key::Ip(ip("203.0.113.1")));
        a.user = Some(u.clone());
        assert_eq!(key_for(&a, Key::Unix), Key::User(u.id));
        let mut job = u;
        job.job_id = Some(9);
        a.user = Some(job);
        assert_eq!(key_for(&a, Key::Unix), Key::Job(9), "job tokens are charged per job, not to the shared bot user");
    }
}

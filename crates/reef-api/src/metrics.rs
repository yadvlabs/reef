//! Product metrics on `/metrics`, on top of the process metrics the
//! `prometheus` crate collects on Linux. See `docs/metrics.md` for the series.
//!
//! - Request counters and a duration histogram are labelled with the *matched route
//!   pattern* (`/api/v3/repos/{owner}/{repo}`), never the raw path, so cardinality is bounded
//!   by the router. Health endpoints are not counted (they would only measure the scraper).
//! - Gauges (repositories, users, organizations, jobs by state, runners online, pending
//!   webhook deliveries) are refreshed from the database every 30 s by [`run`], on every
//!   instance; they read the shared database, so every instance reports the same values.
//! - `reef_actions_logs_bytes` and `reef_actions_artifacts_bytes` walk the shared
//!   `<data_dir>/actions` tree and are refreshed by the retention job (hourly), not every
//!   30 s, because the walk is the expensive part on a network filesystem.

use crate::AppState;
use axum::extract::{MatchedPath, Request};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How often [`run`] refreshes the database gauges.
pub const REFRESH: Duration = Duration::from_secs(30);

pub struct Metrics {
    pub http_requests: IntCounterVec,
    pub http_duration: HistogramVec,
    pub http_rate_limited: IntCounter,
    pub repositories: IntGauge,
    pub users: IntGauge,
    pub organizations: IntGauge,
    pub actions_jobs: IntGaugeVec,
    pub actions_runners_online: IntGauge,
    pub actions_logs_bytes: IntGauge,
    pub actions_artifacts_bytes: IntGauge,
    pub actions_runs_purged: IntCounter,
    pub webhook_deliveries_pending: IntGauge,
    pub webhook_deliveries: IntCounterVec,
    pub mirror_syncs: IntCounterVec,
    pub build_info: IntGaugeVec,
    pub start_time: IntGauge,
}

static REGISTRY: OnceLock<Metrics> = OnceLock::new();

/// The metrics, registered with the default registry the first time they are needed.
pub fn registry() -> &'static Metrics {
    REGISTRY.get_or_init(|| {
        let r = prometheus::default_registry();
        fn reg<C: prometheus::core::Collector + Clone + 'static>(r: &prometheus::Registry, c: C) -> C {
            // Registering twice (tests build several routers) is harmless: the first wins.
            let _ = r.register(Box::new(c.clone()));
            c
        }
        let m = Metrics {
            http_requests: reg(
                r,
                IntCounterVec::new(
                    Opts::new("reef_http_requests_total", "HTTP requests by method, matched route and status"),
                    &["method", "route", "status"],
                )
                .expect("metric"),
            ),
            http_duration: reg(
                r,
                HistogramVec::new(
                    // Prometheus' default buckets.
                    HistogramOpts::new("reef_http_request_duration_seconds", "HTTP request duration by method and matched route"),
                    &["method", "route"],
                )
                .expect("metric"),
            ),
            http_rate_limited: reg(r, IntCounter::new("reef_http_rate_limited_total", "API requests refused with 429").expect("metric")),
            repositories: reg(r, IntGauge::new("reef_repositories_total", "Repositories").expect("metric")),
            users: reg(r, IntGauge::new("reef_users_total", "User accounts").expect("metric")),
            organizations: reg(r, IntGauge::new("reef_organizations_total", "Organizations").expect("metric")),
            actions_jobs: reg(
                r,
                IntGaugeVec::new(Opts::new("reef_actions_jobs", "Workflow jobs by state (queued, running, completed, failed)"), &["state"])
                    .expect("metric"),
            ),
            actions_runners_online: reg(
                r,
                IntGauge::new("reef_actions_runners_online", "Runners that polled in the last two minutes").expect("metric"),
            ),
            actions_logs_bytes: reg(
                r,
                IntGauge::new("reef_actions_logs_bytes", "Bytes of job logs under <data_dir>/actions/logs").expect("metric"),
            ),
            actions_artifacts_bytes: reg(
                r,
                IntGauge::new("reef_actions_artifacts_bytes", "Bytes of artifacts under <data_dir>/actions/artifacts").expect("metric"),
            ),
            actions_runs_purged: reg(
                r,
                IntCounter::new("reef_actions_runs_purged_total", "Workflow runs removed by the retention job").expect("metric"),
            ),
            webhook_deliveries_pending: reg(
                r,
                IntGauge::new("reef_webhook_deliveries_pending", "Webhook deliveries waiting to be posted (queued or retrying)")
                    .expect("metric"),
            ),
            webhook_deliveries: reg(
                r,
                IntCounterVec::new(
                    Opts::new("reef_webhook_deliveries_total", "Webhook delivery attempts by result (delivered, retried, failed)"),
                    &["result"],
                )
                .expect("metric"),
            ),
            mirror_syncs: reg(
                r,
                IntCounterVec::new(Opts::new("reef_mirror_syncs_total", "Pull mirror syncs by result (ok, error)"), &["result"])
                    .expect("metric"),
            ),
            build_info: reg(
                r,
                IntGaugeVec::new(Opts::new("reef_build_info", "Build information"), &["version", "commit"]).expect("metric"),
            ),
            start_time: reg(r, IntGauge::new("reef_start_time_seconds", "Unix time the process started").expect("metric")),
        };
        m.build_info.with_label_values(&[crate::BUILD_VERSION, crate::BUILD_COMMIT]).set(1);
        m.start_time.set(reef_core::time::now_ms() / 1000);
        for state in ["queued", "running", "completed", "failed"] {
            m.actions_jobs.with_label_values(&[state]).set(0);
        }
        for result in ["delivered", "retried", "failed"] {
            m.webhook_deliveries.with_label_values(&[result]).reset();
        }
        for result in ["ok", "error"] {
            m.mirror_syncs.with_label_values(&[result]).reset();
        }
        m
    })
}

fn method_label(m: &Method) -> &'static str {
    match *m {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        Method::CONNECT => "CONNECT",
        Method::TRACE => "TRACE",
        _ => "other",
    }
}

/// The `route` label: the matched pattern, or `unmatched` for the SPA fallback and 404s.
pub fn route_label(req: &Request) -> String {
    req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_string()).unwrap_or_else(|| "unmatched".to_string())
}

/// Count and time every request, the health, readiness and metrics endpoints included.
/// `method` is one of the nine standard verbs or `other`; `route` the matched pattern or
/// `unmatched`.
pub async fn http_metrics(req: Request, next: Next) -> Response {
    let method = method_label(req.method());
    let route = route_label(&req);
    let start = Instant::now();
    let res = next.run(req).await;
    let m = registry();
    m.http_requests.with_label_values(&[method, &route, res.status().as_str()]).inc();
    m.http_duration.with_label_values(&[method, &route]).observe(start.elapsed().as_secs_f64());
    res
}

/// Refresh the database gauges once.
pub async fn refresh(s: &AppState) {
    let m = registry();
    if let Ok(n) = s.db.count_repos().await {
        m.repositories.set(n);
    }
    if let Ok(n) = s.db.count_accounts("user").await {
        m.users.set(n);
    }
    if let Ok(n) = s.db.count_accounts("org").await {
        m.organizations.set(n);
    }
    if let Ok(c) = s.db.count_jobs_by_state().await {
        m.actions_jobs.with_label_values(&["queued"]).set(c.queued);
        m.actions_jobs.with_label_values(&["running"]).set(c.running);
        m.actions_jobs.with_label_values(&["completed"]).set(c.completed);
        m.actions_jobs.with_label_values(&["failed"]).set(c.failed);
    }
    if let Ok(n) = s.db.count_online_runners().await {
        m.actions_runners_online.set(n);
    }
    if let Ok(n) = s.db.count_pending_deliveries().await {
        m.webhook_deliveries_pending.set(n);
    }
}

/// Refresh the gauges every `REFRESH` until the process ends.
pub async fn run(s: AppState) {
    loop {
        refresh(&s).await;
        tokio::time::sleep(REFRESH).await;
    }
}

/// Walk `<data_dir>/actions/{logs,artifacts}` (off the async runtime) and set the byte gauges.
pub async fn refresh_actions_bytes(s: &AppState) {
    let root = s.actions.files.root().to_path_buf();
    let walk = |p: std::path::PathBuf| reef_git::store::dir_size(&p, true).unwrap_or(0);
    let sizes = tokio::task::spawn_blocking(move || (walk(root.join("logs")), walk(root.join("artifacts")))).await;
    if let Ok((logs, artifacts)) = sizes {
        registry().actions_logs_bytes.set(logs as i64);
        registry().actions_artifacts_bytes.set(artifacts as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_every_series_once() {
        let m = registry();
        m.http_requests.with_label_values(&["GET", "/api/v3/zen", "200"]).inc();
        m.http_duration.with_label_values(&["GET", "/api/v3/zen"]).observe(0.01);
        let text = prometheus::TextEncoder::new().encode_to_string(&prometheus::default_registry().gather()).unwrap();
        for name in [
            "reef_http_requests_total",
            "reef_http_request_duration_seconds",
            "reef_repositories_total",
            "reef_users_total",
            "reef_organizations_total",
            "reef_actions_jobs{state=\"queued\"}",
            "reef_actions_jobs{state=\"failed\"}",
            "reef_actions_runners_online",
            "reef_actions_logs_bytes",
            "reef_webhook_deliveries_pending",
            "reef_webhook_deliveries_total{result=\"delivered\"}",
            "reef_mirror_syncs_total{result=\"ok\"}",
            "reef_build_info{commit=",
        ] {
            assert!(text.contains(name), "missing {name} in:\n{text}");
        }
        assert!(text.contains("reef_http_requests_total{method=\"GET\",route=\"/api/v3/zen\",status=\"200\"}"));
        assert_eq!(method_label(&Method::from_bytes(b"PROPFIND").unwrap()), "other");
        assert_eq!(method_label(&Method::TRACE), "TRACE");
        assert!(
            text.contains(&format!("reef_build_info{{commit=\"{}\",version=\"{}\"}} 1", crate::BUILD_COMMIT, crate::BUILD_VERSION)),
            "{text}"
        );
        assert!(
            text.contains("reef_http_request_duration_seconds_bucket{method=\"GET\",route=\"/api/v3/zen\",le=\"0.005\"}"),
            "default buckets"
        );
    }
}

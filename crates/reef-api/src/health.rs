//! Liveness, readiness and metrics: `GET /healthz` is 200 while the process runs,
//! `GET /readyz` is 200 when the database answers over its dedicated health connection and the
//! SSH server is up (503 with a JSON body naming what failed, also while shutting down),
//! `GET /metrics` is Prometheus text. None of them sit behind authentication, timeouts or
//! body limits. `/healthz` and `/readyz` are always on the primary listener; `/metrics` moves
//! to `[metrics].listen` when that is set (the primary then answers 404 for it).

use crate::AppState;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub const HEALTHZ: &str = "/healthz";
pub const READYZ: &str = "/readyz";
pub const METRICS: &str = "/metrics";

pub fn is_health_path(path: &str) -> bool {
    path == HEALTHZ || path == READYZ || path == METRICS
}

/// The primary listener's health routes: `/healthz` and `/readyz` always; `/metrics` unless it
/// is served on its own listener, in which case the path answers 404 here (never the SPA).
pub fn router(with_metrics: bool) -> Router<AppState> {
    let r = Router::new().route(HEALTHZ, get(healthz)).route(READYZ, get(readyz));
    if with_metrics { r.route(METRICS, get(metrics)) } else { r.route(METRICS, get(metrics_moved)) }
}

/// What `[metrics].listen` serves: only `/metrics`.
pub fn metrics_router() -> Router<AppState> {
    Router::new().route(METRICS, get(metrics))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

/// 200 `{"status":"ok"}`, or 503 `{"status":"unavailable","failed":"database|ssh|shutdown"}`
/// : the body names what failed and nothing more; the error itself is logged with the
/// request id.
async fn readyz(State(s): State<AppState>, request_id: Option<axum::Extension<tower_http::request_id::RequestId>>) -> Response {
    let unavailable =
        |failed: &str| (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"status": "unavailable", "failed": failed}))).into_response();
    if s.shutting_down.load(Ordering::Relaxed) {
        return unavailable("shutdown");
    }
    if s.ssh_failed.load(Ordering::Relaxed) {
        return unavailable("ssh");
    }
    let rid = request_id.as_ref().and_then(|r| r.0.header_value().to_str().ok()).unwrap_or("").to_string();
    // The dedicated health pool, not the application pool: readiness must not queue behind it.
    match tokio::time::timeout(Duration::from_secs(5), s.db.ping_dedicated()).await {
        Ok(Ok(())) => Json(json!({"status": "ok"})).into_response(),
        Ok(Err(e)) => {
            tracing::warn!(request_id = %rid, error = %e, "readyz: the database did not answer");
            unavailable("database")
        }
        Err(_) => {
            tracing::warn!(request_id = %rid, "readyz: the database ping timed out");
            unavailable("database")
        }
    }
}

async fn metrics_moved() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"message": "metrics are served on the [metrics].listen address"}))).into_response()
}

/// Prometheus text: the process collector (CPU, memory, fds; Linux), `reef_build_info` and
/// the product series of `crate::metrics` (registered on first use).
async fn metrics() -> Response {
    crate::metrics::registry();
    let families = prometheus::default_registry().gather();
    match prometheus::TextEncoder::new().encode_to_string(&families) {
        Ok(text) => ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], text).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

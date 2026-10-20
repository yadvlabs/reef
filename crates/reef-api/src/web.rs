//! The embedded single-page application and generated avatars.

use crate::AppState;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../web/dist"]
struct Assets;

pub fn router() -> Router<AppState> {
    Router::new().route("/avatars/{id}", get(avatar)).fallback(get(spa))
}

pub async fn spa(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && let Some(f) = Assets::get(path)
    {
        return asset_response(path, f, !path.ends_with(".html"));
    }
    match Assets::get("index.html") {
        Some(f) => asset_response("index.html", f, false),
        None => (StatusCode::SERVICE_UNAVAILABLE, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], PLACEHOLDER).into_response(),
    }
}

/// Shown when the binary was built without `web/dist` (development without `make web`).
/// The API, git and health endpoints work regardless.
const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Reef</title>\
<style>body{font:16px/1.5 system-ui,sans-serif;margin:4rem auto;max-width:36rem;color:#1f2328}code{background:#f6f8fa;padding:.1rem .3rem;border-radius:4px}</style></head>\
<body><h1>Reef is running</h1><p>The web UI has not been built into this binary: run <code>make web</code> and rebuild.</p>\
<p>The API (<code>/api/v3</code>), git over HTTP and SSH, and <code>/healthz</code>, <code>/readyz</code> work without it.</p></body></html>\n";

fn asset_response(path: &str, f: rust_embed::EmbeddedFile, immutable: bool) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut res = ([(header::CONTENT_TYPE, mime.as_ref())], f.data.into_owned()).into_response();
    let cache = if immutable && path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

/// A GitHub-style default avatar: a flat colour derived from the id with the initial.
async fn avatar(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let id: i64 = id.split('.').next().unwrap_or("0").parse().unwrap_or(0);
    if let Some((bytes, ct)) = crate::profile::stored_avatar(&s, id).await {
        return ([(header::CONTENT_TYPE, ct.as_str()), (header::CACHE_CONTROL, "public, max-age=86400")], bytes).into_response();
    }
    let (initial, hue) = match s.db.account_by_id(reef_core::Id(id)).await {
        Ok(a) => (a.login.chars().next().unwrap_or('?').to_ascii_uppercase(), (a.id as u64 % 360) as u32),
        Err(_) => ('?', 210),
    };
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\" viewBox=\"0 0 128 128\"><rect width=\"128\" height=\"128\" fill=\"hsl({hue} 45% 45%)\"/><text x=\"64\" y=\"84\" font-family=\"-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,Arial,sans-serif\" font-size=\"64\" font-weight=\"600\" fill=\"#fff\" text-anchor=\"middle\">{initial}</text></svg>"
    );
    ([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "public, max-age=3600")], svg).into_response()
}

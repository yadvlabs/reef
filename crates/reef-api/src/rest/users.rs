use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use reef_core::{Error, Id};
use reef_db::accounts::UserPatch;
use serde_json::{Value, json};

pub async fn current(State(s): State<AppState>, auth: Auth) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let n = s.db.count_repos_for_owner(u.id(), false).await?;
    let mut h = HeaderMap::new();
    h.insert("x-oauth-scopes", auth.scopes_header().parse().unwrap());
    h.insert("x-accepted-oauth-scopes", "".parse().unwrap());
    Ok((h, Json(shapes::full_user(&s.config, u, n, true))))
}

#[derive(serde::Deserialize)]
pub struct UpdateUser {
    name: Option<String>,
    email: Option<String>,
    blog: Option<String>,
    company: Option<String>,
    location: Option<String>,
    bio: Option<String>,
}

pub async fn update_current(State(s): State<AppState>, auth: Auth, Json(b): Json<UpdateUser>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let empty_to_none = |v: Option<String>| v.map(|s| if s.is_empty() { None } else { Some(s) });
    let a =
        s.db.update_user(
            u.id(),
            UserPatch {
                name: empty_to_none(b.name),
                email: b.email,
                bio: empty_to_none(b.bio),
                company: empty_to_none(b.company),
                location: empty_to_none(b.location),
                blog: empty_to_none(b.blog),
            },
        )
        .await?;
    let n = s.db.count_repos_for_owner(a.id(), false).await?;
    Ok(Json(shapes::full_user(&s.config, &a, n, true)))
}

pub async fn show(State(s): State<AppState>, auth: Auth, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let is_self = auth.user.as_ref().map(|u| u.id == a.id).unwrap_or(false);
    let n = s.db.count_repos_for_owner(a.id(), is_self).await?;
    if a.is_org() { Ok(Json(shapes::org_full(&s.config, &a, n, false))) } else { Ok(Json(shapes::full_user(&s.config, &a, n, is_self))) }
}

#[derive(serde::Deserialize)]
pub struct Since {
    #[serde(default)]
    pub since: i64,
    #[serde(default = "thirty")]
    pub per_page: i64,
}
fn thirty() -> i64 {
    30
}

pub async fn list(State(s): State<AppState>, Query(q): Query<Since>) -> ApiResult<Json<Value>> {
    let users = s.db.list_accounts("user", q.since, q.per_page.clamp(1, 100)).await?;
    Ok(Json(json!(users.iter().map(|u| shapes::simple_user(&s.config, u)).collect::<Vec<_>>())))
}

// ----- emails -----

fn email_json(e: &reef_db::emails::UserEmail) -> Value {
    json!({"email": e.email, "primary": e.is_primary(), "verified": e.is_verified(), "visibility": e.visibility})
}

/// `GET /user/emails`: every address of the signed-in user, primary first. Verification by
/// mail round-trip is not implemented; addresses the owner adds are stored as verified.
pub async fn emails(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let list = s.db.user_emails(u.id()).await?;
    Ok(Json(json!(list.iter().map(email_json).collect::<Vec<_>>())))
}

pub async fn public_emails(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let list = s.db.user_emails(u.id()).await?;
    Ok(Json(json!(list.iter().filter(|e| e.visibility == "public").map(email_json).collect::<Vec<_>>())))
}

/// Body of `POST`/`DELETE /user/emails`: GitHub accepts `{"emails": [...]}` or a bare array.
#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum EmailsBody {
    Wrapped { emails: Vec<String> },
    Bare(Vec<String>),
    One(String),
}

impl EmailsBody {
    fn list(self) -> Vec<String> {
        match self {
            EmailsBody::Wrapped { emails } => emails,
            EmailsBody::Bare(v) => v,
            EmailsBody::One(s) => vec![s],
        }
    }
}

pub async fn add_emails(State(s): State<AppState>, auth: Auth, Json(b): Json<EmailsBody>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    if !auth.has_scope("user") && !auth.has_scope("user:email") {
        return Err(crate::error::forbidden("Token requires the user scope"));
    }
    let list = s.db.add_user_emails(u.id(), &b.list()).await?;
    Ok((StatusCode::CREATED, Json(json!(list.iter().map(email_json).collect::<Vec<_>>()))))
}

pub async fn delete_emails(State(s): State<AppState>, auth: Auth, Json(b): Json<EmailsBody>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    if !auth.has_scope("user") && !auth.has_scope("user:email") {
        return Err(crate::error::forbidden("Token requires the user scope"));
    }
    s.db.delete_user_emails(u.id(), &b.list()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct PrimaryEmail {
    email: String,
}

/// Reef extension: `PATCH /user/emails/primary {email}` makes one of the addresses primary.
pub async fn set_primary_email(State(s): State<AppState>, auth: Auth, Json(b): Json<PrimaryEmail>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let list = s.db.set_primary_email(u.id(), &b.email).await?;
    Ok(Json(json!(list.iter().map(email_json).collect::<Vec<_>>())))
}

#[derive(serde::Deserialize)]
pub struct Visibility {
    visibility: String,
}

pub async fn set_email_visibility(State(s): State<AppState>, auth: Auth, Json(b): Json<Visibility>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let list = s.db.set_email_visibility(u.id(), &b.visibility).await?;
    Ok(Json(json!(list.iter().map(email_json).collect::<Vec<_>>())))
}

// ----- SSH keys -----

pub async fn list_keys(State(s): State<AppState>, auth: Auth, p: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let keys = s.db.list_ssh_keys(u.id()).await?;
    let all: Vec<Value> = keys.iter().map(|k| shapes::ssh_key(&s.config, k)).collect();
    let (items, h) = p.slice(&all, &format!("{}/user/keys", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

pub async fn public_keys(State(s): State<AppState>, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let keys = s.db.list_ssh_keys(a.id()).await?;
    Ok(Json(json!(keys.iter().map(|k| json!({"id": k.id, "key": k.public_key})).collect::<Vec<_>>())))
}

#[derive(serde::Deserialize)]
pub struct NewKey {
    title: Option<String>,
    key: String,
}

pub async fn add_key(State(s): State<AppState>, auth: Auth, Json(b): Json<NewKey>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    if !auth.has_scope("admin:public_key") && !auth.has_scope("write:public_key") {
        return Err(crate::error::forbidden("Token requires the admin:public_key scope"));
    }
    let (key_type, key_b64, comment) = parse_ssh_key(&b.key)?;
    let fingerprint = ssh_fingerprint(&key_b64)?;
    let title = b.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| if comment.is_empty() { "key".into() } else { comment.clone() });
    let k = s.db.add_ssh_key(u.id(), &title, &key_type, &format!("{key_type} {key_b64}"), &fingerprint).await?;
    Ok((StatusCode::CREATED, Json(shapes::ssh_key(&s.config, &k))))
}

pub async fn show_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let k = s.db.ssh_key_by_id(Id(id)).await?;
    if k.user_id != u.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(shapes::ssh_key(&s.config, &k)))
}

pub async fn delete_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.delete_ssh_key(u.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Split an OpenSSH public key line into (type, base64, comment) and check the blob's
/// declared type matches.
pub fn parse_ssh_key(line: &str) -> ApiResult<(String, String, String)> {
    let mut parts = line.split_whitespace();
    let ty = parts.next().unwrap_or("").to_string();
    let b64 = parts.next().unwrap_or("").to_string();
    let comment = parts.collect::<Vec<_>>().join(" ");
    let ok_type = matches!(
        ty.as_str(),
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "sk-ssh-ed25519@openssh.com"
            | "sk-ecdsa-sha2-nistp256@openssh.com"
    );
    use base64::Engine;
    let blob = base64::engine::general_purpose::STANDARD.decode(&b64).ok();
    let declared_matches = blob
        .as_ref()
        .and_then(|b| {
            if b.len() < 4 {
                return None;
            }
            let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
            b.get(4..4 + n).map(|t| t == ty.as_bytes())
        })
        .unwrap_or(false);
    if !ok_type || !declared_matches {
        return Err(ApiError(Error::field(
            "PublicKey",
            "key",
            "invalid",
            "key is invalid. It must begin with 'ssh-ed25519', 'ssh-rsa', 'ecdsa-sha2-nistp256', 'ecdsa-sha2-nistp384', or 'ecdsa-sha2-nistp521'. Check that you're copying the public half of the key",
        )));
    }
    Ok((ty, b64, comment))
}

pub fn ssh_fingerprint(b64: &str) -> ApiResult<String> {
    use base64::Engine;
    use sha2::Digest;
    let blob = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| ApiError(Error::validation("key is invalid")))?;
    let digest = sha2::Sha256::digest(&blob);
    Ok(format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)))
}

pub async fn empty_list() -> Json<Value> {
    Json(json!([]))
}

// ----- GPG keys -----

fn key_scope(auth: &Auth) -> ApiResult<()> {
    if !auth.has_scope("admin:gpg_key") && !auth.has_scope("write:gpg_key") {
        return Err(crate::error::forbidden("Token requires the admin:gpg_key scope"));
    }
    Ok(())
}

pub async fn list_gpg_keys(State(s): State<AppState>, auth: Auth, p: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let keys = s.db.gpg_keys(u.id()).await?;
    let all: Vec<Value> = keys.iter().map(crate::signing::gpg_key_json).collect();
    let (items, h) = p.slice(&all, &format!("{}/user/gpg_keys", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

pub async fn public_gpg_keys(State(s): State<AppState>, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let keys = s.db.gpg_keys(a.id()).await?;
    Ok(Json(json!(keys.iter().map(crate::signing::gpg_key_json).collect::<Vec<_>>())))
}

#[derive(serde::Deserialize)]
pub struct NewGpgKey {
    name: Option<String>,
    armored_public_key: String,
}

pub async fn add_gpg_key(State(s): State<AppState>, auth: Auth, Json(b): Json<NewGpgKey>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    key_scope(&auth)?;
    let parsed = crate::signing::parse_gpg_key(&b.armored_public_key).map_err(ApiError)?;
    let _ = b.name;
    let k =
        s.db.add_gpg_key(
            u.id(),
            reef_db::gpg::NewGpgKey {
                key_id: &parsed.key_id,
                fingerprint: &parsed.fingerprint,
                public_key: &parsed.raw_key,
                emails: &parsed.emails,
                subkeys: &parsed.subkeys,
                can_sign: parsed.can_sign,
                can_certify: parsed.can_certify,
                can_encrypt_comms: parsed.can_encrypt_comms,
                can_encrypt_storage: parsed.can_encrypt_storage,
                expires_at: parsed.expires_at,
            },
        )
        .await?;
    crate::signing::invalidate_cache();
    Ok((StatusCode::CREATED, Json(crate::signing::gpg_key_json(&k))))
}

pub async fn show_gpg_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let k = s.db.gpg_key_by_id(Id(id)).await?;
    if k.user_id != u.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(crate::signing::gpg_key_json(&k)))
}

pub async fn delete_gpg_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    key_scope(&auth)?;
    s.db.delete_gpg_key(u.id(), Id(id)).await?;
    crate::signing::invalidate_cache();
    Ok(StatusCode::NO_CONTENT)
}

// ----- SSH signing keys -----

fn signing_key_json(k: &reef_db::gpg::SshSigningKey) -> Value {
    json!({"id": k.id, "key": k.key, "title": k.title, "created_at": reef_core::time::iso(k.created_at)})
}

pub async fn list_signing_keys(State(s): State<AppState>, auth: Auth, p: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let keys = s.db.ssh_signing_keys(u.id()).await?;
    let all: Vec<Value> = keys.iter().map(signing_key_json).collect();
    let (items, h) = p.slice(&all, &format!("{}/user/ssh_signing_keys", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

pub async fn public_signing_keys(State(s): State<AppState>, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let keys = s.db.ssh_signing_keys(a.id()).await?;
    Ok(Json(json!(keys.iter().map(signing_key_json).collect::<Vec<_>>())))
}

pub async fn add_signing_key(State(s): State<AppState>, auth: Auth, Json(b): Json<NewKey>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    if !auth.has_scope("admin:ssh_signing_key") && !auth.has_scope("write:ssh_signing_key") {
        return Err(crate::error::forbidden("Token requires the admin:ssh_signing_key scope"));
    }
    let (key_type, key_b64, comment) = parse_ssh_key(&b.key)?;
    let fingerprint = ssh_fingerprint(&key_b64)?;
    let title =
        b.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| if comment.is_empty() { "signing key".into() } else { comment.clone() });
    let k = s.db.add_ssh_signing_key(u.id(), &title, &format!("{key_type} {key_b64}"), &fingerprint).await?;
    crate::signing::invalidate_cache();
    Ok((StatusCode::CREATED, Json(signing_key_json(&k))))
}

pub async fn show_signing_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let k = s.db.ssh_signing_key_by_id(Id(id)).await?;
    if k.user_id != u.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(signing_key_json(&k)))
}

pub async fn delete_signing_key(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.delete_ssh_signing_key(u.id(), Id(id)).await?;
    crate::signing::invalidate_cache();
    Ok(StatusCode::NO_CONTENT)
}

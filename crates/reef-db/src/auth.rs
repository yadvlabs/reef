//! Sessions, personal access tokens, OAuth tokens, and SSH keys.

use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result, token};
use sqlx::AssertSqlSafe;
use sqlx::Row;

/// Thirty days.
pub const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_seen_at: i64,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    /// The provider sign-in (ID token `sid`) the session was opened from; `None` for sessions
    /// not opened through OpenID Connect. A back-channel logout for it ends the session.
    pub sid: Option<String>,
}

/// The OpenID Connect sign-in behind a session: its `sid` and ID token (both optional).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignIn {
    pub sid: Option<String>,
    pub id_token: Option<String>,
}

impl SignIn {
    /// Whether the session came from an OpenID Connect sign-in.
    pub fn is_oidc(&self) -> bool {
        self.sid.is_some() || self.id_token.is_some()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccessToken {
    pub id: i64,
    pub user_id: i64,
    pub kind: String,
    pub name: String,
    pub token_last4: String,
    pub scopes: String,
    pub client_id: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

impl AccessToken {
    pub fn scope_list(&self) -> Vec<&str> {
        self.scopes.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SshKey {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub key_type: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

const TOKEN_COLS: &str = "id, user_id, kind, name, token_last4, scopes, client_id, created_at, expires_at, last_used_at";
const KEY_COLS: &str = "id, user_id, title, key_type, public_key, fingerprint, created_at, last_used_at";

impl Db {
    // ----- sessions -----

    /// Create a session and return the cookie value (plaintext, shown once).
    pub async fn create_session(&self, user: Id, ip: Option<&str>, ua: Option<&str>) -> Result<String> {
        self.create_sign_in_session(user, ip, ua, &SignIn::default()).await
    }

    /// Create a session opened from an OpenID Connect sign-in: a back-channel logout for `sid`
    /// ends it, and signing out of it sends the browser to the provider with `id_token` as the hint.
    pub async fn create_sign_in_session(&self, user: Id, ip: Option<&str>, ua: Option<&str>, sign_in: &SignIn) -> Result<String> {
        let plain = token::mint(token::SESSION_PREFIX);
        let ts = now();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, created_at, expires_at, last_seen_at, ip, user_agent, sid, id_token) VALUES ($1, $2, $3, $4, $3, $5, $6, $7, $8)",
        )
        .bind(token::hash(&plain))
        .bind(user.0)
        .bind(ts)
        .bind(ts + SESSION_TTL_MS)
        .bind(ip)
        .bind(ua.map(|s| s.chars().take(256).collect::<String>()))
        .bind(sign_in.sid.as_deref().filter(|s| !s.is_empty()))
        .bind(sign_in.id_token.as_deref().filter(|s| !s.is_empty()))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(plain)
    }

    /// The provider sign-in a session was opened from (empty for other sessions, and for
    /// unknown cookies).
    pub async fn session_sign_in(&self, plain: &str) -> Result<SignIn> {
        let row = sqlx::query("SELECT sid, id_token FROM sessions WHERE id = $1")
            .bind(token::hash(plain))
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(match row {
            Some(r) => SignIn { sid: r.try_get("sid").map_err(map_err)?, id_token: r.try_get("id_token").map_err(map_err)? },
            None => SignIn::default(),
        })
    }

    /// Resolve a cookie value to a live session, touching `last_seen_at` at most once a minute.
    pub async fn session_by_cookie(&self, plain: &str) -> Result<Session> {
        let s = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, created_at, expires_at, last_seen_at, ip, user_agent, sid FROM sessions WHERE id = $1",
        )
        .bind(token::hash(plain))
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        let ts = now();
        if s.expires_at < ts {
            let _ = self.delete_session(plain).await;
            return Err(Error::Unauthorized("session expired".into()));
        }
        if ts - s.last_seen_at > 60_000 {
            let _ = sqlx::query("UPDATE sessions SET last_seen_at = $1 WHERE id = $2").bind(ts).bind(&s.id).execute(&self.pool).await;
        }
        Ok(s)
    }

    pub async fn delete_session(&self, plain: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1").bind(token::hash(plain)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// End every session of an account. Returns how many ended.
    pub async fn delete_user_sessions(&self, user: Id) -> Result<u64> {
        let r = sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(user.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(r.rows_affected())
    }

    /// End every session opened from the provider sign-in `sid`. Returns how many ended.
    pub async fn delete_sessions_by_sid(&self, sid: &str) -> Result<u64> {
        if sid.is_empty() {
            return Ok(0);
        }
        let r = sqlx::query("DELETE FROM sessions WHERE sid = $1").bind(sid).execute(&self.pool).await.map_err(map_err)?;
        Ok(r.rows_affected())
    }

    // ----- tokens -----

    /// Mint a token. Returns the plaintext (shown once) and the stored record.
    pub async fn create_token(
        &self,
        user: Id,
        kind: &str,
        name: &str,
        scopes: &[&str],
        client_id: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<(String, AccessToken)> {
        let prefix = if kind == "oauth" { token::OAUTH_PREFIX } else { token::PAT_PREFIX };
        let plain = token::mint(prefix);
        let id = reef_core::id::next();
        sqlx::query(
            "INSERT INTO access_tokens (id, user_id, kind, name, token_hash, token_last4, scopes, client_id, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id.0)
        .bind(user.0)
        .bind(kind)
        .bind(name)
        .bind(token::hash(&plain))
        .bind(&plain[plain.len() - 4..])
        .bind(scopes.join(","))
        .bind(client_id)
        .bind(now())
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        let t = self.token_by_id(id).await?;
        Ok((plain, t))
    }

    pub async fn token_by_id(&self, id: Id) -> Result<AccessToken> {
        sqlx::query_as::<_, AccessToken>(AssertSqlSafe(format!("SELECT {TOKEN_COLS} FROM access_tokens WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Resolve a bearer secret. Rejects expired tokens and records use (rate-limited to once a minute).
    pub async fn token_by_plain(&self, plain: &str) -> Result<AccessToken> {
        let t = sqlx::query_as::<_, AccessToken>(AssertSqlSafe(format!("SELECT {TOKEN_COLS} FROM access_tokens WHERE token_hash = $1")))
            .bind(token::hash(plain))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::NotFound => Error::Unauthorized("Bad credentials".into()),
                o => o,
            })?;
        let ts = now();
        if matches!(t.expires_at, Some(e) if e < ts) {
            return Err(Error::Unauthorized("token expired".into()));
        }
        if t.last_used_at.is_none_or(|l| ts - l > 60_000) {
            let _ = sqlx::query("UPDATE access_tokens SET last_used_at = $1 WHERE id = $2").bind(ts).bind(t.id).execute(&self.pool).await;
        }
        Ok(t)
    }

    pub async fn list_tokens(&self, user: Id) -> Result<Vec<AccessToken>> {
        sqlx::query_as::<_, AccessToken>(AssertSqlSafe(format!(
            "SELECT {TOKEN_COLS} FROM access_tokens WHERE user_id = $1 ORDER BY id DESC"
        )))
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn delete_token(&self, user: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM access_tokens WHERE id = $1 AND user_id = $2")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    // ----- ssh keys -----

    pub async fn add_ssh_key(&self, user: Id, title: &str, key_type: &str, public_key: &str, fingerprint: &str) -> Result<SshKey> {
        let id = reef_core::id::next();
        sqlx::query(
            "INSERT INTO ssh_keys (id, user_id, title, key_type, public_key, fingerprint, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id.0)
        .bind(user.0)
        .bind(title)
        .bind(key_type)
        .bind(public_key)
        .bind(fingerprint)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("PublicKey", "key", "already_exists", "key is already in use"),
            o => o,
        })?;
        self.ssh_key_by_id(id).await
    }

    pub async fn ssh_key_by_id(&self, id: Id) -> Result<SshKey> {
        sqlx::query_as::<_, SshKey>(AssertSqlSafe(format!("SELECT {KEY_COLS} FROM ssh_keys WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn ssh_key_by_fingerprint(&self, fp: &str) -> Result<SshKey> {
        sqlx::query_as::<_, SshKey>(AssertSqlSafe(format!("SELECT {KEY_COLS} FROM ssh_keys WHERE fingerprint = $1")))
            .bind(fp)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn list_ssh_keys(&self, user: Id) -> Result<Vec<SshKey>> {
        sqlx::query_as::<_, SshKey>(AssertSqlSafe(format!("SELECT {KEY_COLS} FROM ssh_keys WHERE user_id = $1 ORDER BY id")))
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn delete_ssh_key(&self, user: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM ssh_keys WHERE id = $1 AND user_id = $2")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    pub async fn count_sessions(&self, user: Id) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM sessions WHERE user_id = $1")
            .bind(user.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        row.try_get::<i64, _>("n").map_err(map_err)
    }
}

// ----- OpenID Connect -----

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LoginState {
    pub state: String,
    pub nonce: String,
    pub pkce: String,
    pub return_to: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingIdentity {
    pub id: String,
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: i64,
    pub username: Option<String>,
    pub name: Option<String>,
    pub return_to: Option<String>,
    pub created_at: i64,
    /// The provider sign-in (`sid`) this identity arrived with, and its ID token.
    pub sid: Option<String>,
    pub id_token: Option<String>,
}

impl PendingIdentity {
    pub fn sign_in(&self) -> SignIn {
        SignIn { sid: self.sid.clone(), id_token: self.id_token.clone() }
    }
}

impl Db {
    pub async fn create_login_state(&self, state: &str, nonce: &str, pkce: &str, return_to: Option<&str>) -> Result<()> {
        sqlx::query("INSERT INTO login_states (state, nonce, pkce, return_to, created_at) VALUES ($1, $2, $3, $4, $5)")
            .bind(state)
            .bind(nonce)
            .bind(pkce)
            .bind(return_to)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        // Opportunistic cleanup of abandoned logins.
        let _ = sqlx::query("DELETE FROM login_states WHERE created_at < $1").bind(now() - 15 * 60 * 1000).execute(&self.pool).await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_pending_identity(
        &self,
        issuer: &str,
        subject: &str,
        email: Option<&str>,
        email_verified: bool,
        username: Option<&str>,
        name: Option<&str>,
        return_to: Option<&str>,
        sign_in: &SignIn,
    ) -> Result<String> {
        let id = reef_core::token::random_hex(16);
        sqlx::query("INSERT INTO pending_identities (id, issuer, subject, email, email_verified, username, name, return_to, created_at, sid, id_token) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)")
            .bind(&id)
            .bind(issuer)
            .bind(subject)
            .bind(email)
            .bind(crate::b(email_verified))
            .bind(username)
            .bind(name)
            .bind(return_to)
            .bind(now())
            .bind(sign_in.sid.as_deref().filter(|s| !s.is_empty()))
            .bind(sign_in.id_token.as_deref().filter(|s| !s.is_empty()))
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        let _ = sqlx::query("DELETE FROM pending_identities WHERE created_at < $1").bind(now() - 30 * 60 * 1000).execute(&self.pool).await;
        Ok(id)
    }

    /// Read a pending identity without consuming it (the login page shows the hint).
    pub async fn pending_identity(&self, id: &str) -> Result<PendingIdentity> {
        let p = sqlx::query_as::<_, PendingIdentity>("SELECT id, issuer, subject, email, email_verified, username, name, return_to, created_at, sid, id_token FROM pending_identities WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::NotFound => Error::Unauthorized("this sign-in link has expired; start again from the identity provider".into()),
                o => o,
            })?;
        if now() - p.created_at > 30 * 60 * 1000 {
            return Err(Error::Unauthorized("this sign-in link has expired; start again from the identity provider".into()));
        }
        Ok(p)
    }

    pub async fn delete_pending_identity(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM pending_identities WHERE id = $1").bind(id).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// Consume a login state (single use).
    pub async fn take_login_state(&self, state: &str) -> Result<LoginState> {
        let s = sqlx::query_as::<_, LoginState>("SELECT state, nonce, pkce, return_to, created_at FROM login_states WHERE state = $1")
            .bind(state)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::NotFound => Error::Unauthorized("unknown or expired login state".into()),
                o => o,
            })?;
        sqlx::query("DELETE FROM login_states WHERE state = $1").bind(state).execute(&self.pool).await.map_err(map_err)?;
        if now() - s.created_at > 15 * 60 * 1000 {
            return Err(Error::Unauthorized("login state expired".into()));
        }
        Ok(s)
    }

    pub async fn user_by_external_identity(&self, issuer: &str, subject: &str) -> Result<Option<crate::accounts::Account>> {
        let row = sqlx::query("SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = $2")
            .bind(issuer)
            .bind(subject)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        match row {
            Some(r) => {
                let id: i64 = r.try_get("user_id").map_err(map_err)?;
                let _ = sqlx::query("UPDATE external_identities SET last_login_at = $1 WHERE issuer = $2 AND subject = $3")
                    .bind(now())
                    .bind(issuer)
                    .bind(subject)
                    .execute(&self.pool)
                    .await;
                Ok(Some(self.account_by_id(Id(id)).await?))
            }
            None => Ok(None),
        }
    }

    /// The account linked to a provider identity, if any (a plain lookup: unlike
    /// `user_by_external_identity` it does not count as a sign-in).
    pub async fn external_identity_user(&self, issuer: &str, subject: &str) -> Result<Option<Id>> {
        let row = sqlx::query("SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = $2")
            .bind(issuer)
            .bind(subject)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        match row {
            Some(r) => Ok(Some(Id(r.try_get("user_id").map_err(map_err)?))),
            None => Ok(None),
        }
    }

    pub async fn link_external_identity(&self, user: Id, issuer: &str, subject: &str, email: Option<&str>) -> Result<()> {
        sqlx::query("INSERT INTO external_identities (user_id, issuer, subject, email, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (issuer, subject) DO UPDATE SET user_id = $1, email = $4, last_login_at = $5")
            .bind(user.0)
            .bind(issuer)
            .bind(subject)
            .bind(email)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Move one provider identity recorded under `from` (a former issuer URL of the same
    /// provider) to `to`, keeping its account. Returns whether it moved: false when nothing is
    /// recorded under (`from`, `subject`) or (`to`, `subject`) already exists.
    pub async fn rekey_external_identity(&self, from: &str, to: &str, subject: &str) -> Result<bool> {
        let r = sqlx::query(
            "UPDATE external_identities SET issuer = $1 WHERE issuer = $2 AND subject = $3 \
             AND NOT EXISTS (SELECT 1 FROM external_identities e WHERE e.issuer = $1 AND e.subject = $3)",
        )
        .bind(to)
        .bind(from)
        .bind(subject)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(r.rows_affected() > 0)
    }

    /// Move every identity recorded under the issuer `from` to `to` (the provider's issuer URL
    /// changed). Returns (moved, left): identities whose subject already exists under `to` stay.
    pub async fn rekey_issuer(&self, from: &str, to: &str) -> Result<(u64, u64)> {
        if from == to {
            return Ok((0, 0));
        }
        let r = sqlx::query(
            "UPDATE external_identities SET issuer = $1 WHERE issuer = $2 \
             AND NOT EXISTS (SELECT 1 FROM external_identities e WHERE e.issuer = $1 AND e.subject = external_identities.subject)",
        )
        .bind(to)
        .bind(from)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM external_identities WHERE issuer = $1")
            .bind(from)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((r.rows_affected(), left as u64))
    }

    pub async fn external_identities(&self, user: Id) -> Result<Vec<(String, String, Option<String>)>> {
        let rows = sqlx::query("SELECT issuer, subject, email FROM external_identities WHERE user_id = $1")
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows.iter().map(|r| (r.get("issuer"), r.get("subject"), r.get("email"))).collect())
    }
}

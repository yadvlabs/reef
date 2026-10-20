//! OAuth 2.0 device authorization grant (RFC 8628), the flow `gh auth login` uses.

use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result, token};
use sqlx::AssertSqlSafe;

/// Fifteen minutes, like GitHub.
pub const DEVICE_TTL_MS: i64 = 15 * 60 * 1000;
/// Minimum seconds between polls.
pub const DEVICE_INTERVAL_S: i64 = 5;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeviceGrant {
    pub device_code_hash: String,
    pub user_code: String,
    pub client_id: String,
    pub scopes: String,
    pub status: String,
    pub user_id: Option<i64>,
    pub token_plain: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_poll_at: Option<i64>,
}

const COLS: &str = "device_code_hash, user_code, client_id, scopes, status, user_id, token_plain, created_at, expires_at, last_poll_at";

pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: i64,
}

pub enum Poll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    /// The access token plaintext and the scopes granted.
    Granted {
        access_token: String,
        scopes: String,
    },
}

impl Db {
    pub async fn device_start(&self, client_id: &str, scopes: &str) -> Result<DeviceStart> {
        let device_code = token::random_hex(20);
        let ts = now();
        // Retry on the (unlikely) user-code collision.
        for _ in 0..5 {
            let user_code = token::user_code();
            let r = sqlx::query(
                "INSERT INTO device_codes (device_code_hash, user_code, client_id, scopes, status, created_at, expires_at) VALUES ($1, $2, $3, $4, 'pending', $5, $6)",
            )
            .bind(token::hash(&device_code))
            .bind(&user_code)
            .bind(client_id)
            .bind(scopes)
            .bind(ts)
            .bind(ts + DEVICE_TTL_MS)
            .execute(&self.pool)
            .await
            .map_err(map_err);
            match r {
                Ok(_) => {
                    return Ok(DeviceStart { device_code, user_code, expires_in: DEVICE_TTL_MS / 1000, interval: DEVICE_INTERVAL_S });
                }
                Err(Error::Conflict(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::msg("could not allocate a device user code"))
    }

    pub async fn device_by_user_code(&self, user_code: &str) -> Result<DeviceGrant> {
        let g = sqlx::query_as::<_, DeviceGrant>(AssertSqlSafe(format!("SELECT {COLS} FROM device_codes WHERE user_code = $1")))
            .bind(user_code.trim().to_uppercase())
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        if g.expires_at < now() {
            return Err(Error::NotFound);
        }
        Ok(g)
    }

    /// The signed-in user approves (or denies) the grant shown by its user code. On approval an
    /// OAuth token is minted and parked on the grant until the client polls.
    pub async fn device_decide(&self, user_code: &str, user: Id, approve: bool) -> Result<()> {
        let g = self.device_by_user_code(user_code).await?;
        if g.status != "pending" {
            return Err(Error::Conflict("this code was already used".into()));
        }
        if !approve {
            sqlx::query("UPDATE device_codes SET status = 'denied' WHERE device_code_hash = $1")
                .bind(&g.device_code_hash)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
            return Ok(());
        }
        let scopes: Vec<&str> = g.scopes.split([',', ' ']).filter(|s| !s.is_empty()).collect();
        let name = format!("{} (device flow)", client_name(&g.client_id));
        let (plain, _t) = self.create_token(user, "oauth", &name, &scopes, Some(&g.client_id), None).await?;
        sqlx::query(
            "UPDATE device_codes SET status = 'authorized', user_id = $1, token_plain = $2 WHERE device_code_hash = $3 AND status = 'pending'",
        )
        .bind(user.0)
        .bind(plain)
        .bind(&g.device_code_hash)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    /// The client polls with its device code.
    pub async fn device_poll(&self, device_code: &str) -> Result<Poll> {
        let hash = token::hash(device_code);
        let g =
            match sqlx::query_as::<_, DeviceGrant>(AssertSqlSafe(format!("SELECT {COLS} FROM device_codes WHERE device_code_hash = $1")))
                .bind(&hash)
                .fetch_one(&self.pool)
                .await
            {
                Ok(g) => g,
                Err(sqlx::Error::RowNotFound) => return Ok(Poll::Expired),
                Err(e) => return Err(map_err(e)),
            };
        let ts = now();
        if g.expires_at < ts {
            return Ok(Poll::Expired);
        }
        if let Some(last) = g.last_poll_at
            && ts - last < (DEVICE_INTERVAL_S * 1000) - 500
        {
            return Ok(Poll::SlowDown);
        }
        sqlx::query("UPDATE device_codes SET last_poll_at = $1 WHERE device_code_hash = $2")
            .bind(ts)
            .bind(&hash)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        match g.status.as_str() {
            "pending" => Ok(Poll::Pending),
            "denied" => Ok(Poll::Denied),
            "authorized" => {
                let plain = g.token_plain.clone().unwrap_or_default();
                sqlx::query("UPDATE device_codes SET status = 'redeemed', token_plain = NULL WHERE device_code_hash = $1")
                    .bind(&hash)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                Ok(Poll::Granted { access_token: plain, scopes: g.scopes })
            }
            _ => Ok(Poll::Expired),
        }
    }

    pub async fn device_gc(&self) -> Result<u64> {
        let r = sqlx::query("DELETE FROM device_codes WHERE expires_at < $1")
            .bind(now() - 3_600_000)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(r.rows_affected())
    }
}

/// A friendly name for well-known OAuth client ids.
pub fn client_name(client_id: &str) -> &str {
    match client_id {
        // GitHub CLI's client id, sent unchanged to enterprise hosts.
        "178c6fc778ccc68e1d6a" => "GitHub CLI",
        _ => "OAuth app",
    }
}

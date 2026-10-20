//! GPG keys and SSH signing keys registered by users, used to verify commit signatures.

use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::AssertSqlSafe;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GpgKey {
    pub id: i64,
    pub user_id: i64,
    pub key_id: String,
    pub primary_key_id: Option<i64>,
    pub fingerprint: String,
    pub public_key: String,
    /// JSON array of user-id email addresses.
    pub emails: String,
    /// JSON array of subkey descriptions.
    pub subkeys: String,
    pub can_sign: i64,
    pub can_certify: i64,
    pub can_encrypt_comms: i64,
    pub can_encrypt_storage: i64,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

impl GpgKey {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn emails(&self) -> Vec<String> {
        serde_json::from_str(&self.emails).unwrap_or_default()
    }
    pub fn subkeys(&self) -> Vec<serde_json::Value> {
        serde_json::from_str(&self.subkeys).unwrap_or_default()
    }
}

/// Everything parsed from an armored key that the table stores.
pub struct NewGpgKey<'a> {
    pub key_id: &'a str,
    pub fingerprint: &'a str,
    pub public_key: &'a str,
    pub emails: &'a [String],
    pub subkeys: &'a serde_json::Value,
    pub can_sign: bool,
    pub can_certify: bool,
    pub can_encrypt_comms: bool,
    pub can_encrypt_storage: bool,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SshSigningKey {
    pub id: i64,
    pub user_id: i64,
    pub key: String,
    pub title: String,
    pub fingerprint: String,
    pub created_at: i64,
}

const GPG_COLS: &str = "id, user_id, key_id, primary_key_id, fingerprint, public_key, emails, subkeys, can_sign, can_certify, can_encrypt_comms, can_encrypt_storage, expires_at, created_at";
const SSK_COLS: &str = "id, user_id, key, title, fingerprint, created_at";

impl Db {
    // ----- GPG keys -----

    pub async fn add_gpg_key(&self, user: Id, k: NewGpgKey<'_>) -> Result<GpgKey> {
        let id = reef_core::id::next();
        sqlx::query(
            "INSERT INTO gpg_keys (id, user_id, key_id, primary_key_id, fingerprint, public_key, emails, subkeys, can_sign, can_certify, can_encrypt_comms, can_encrypt_storage, expires_at, created_at) \
             VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(id.0)
        .bind(user.0)
        .bind(k.key_id)
        .bind(k.fingerprint)
        .bind(k.public_key)
        .bind(serde_json::to_string(k.emails).unwrap_or_else(|_| "[]".into()))
        .bind(k.subkeys.to_string())
        .bind(k.can_sign as i64)
        .bind(k.can_certify as i64)
        .bind(k.can_encrypt_comms as i64)
        .bind(k.can_encrypt_storage as i64)
        .bind(k.expires_at)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("GpgKey", "armored_public_key", "already_exists", "key is already in use"),
            o => o,
        })?;
        self.gpg_key_by_id(id).await
    }

    pub async fn gpg_key_by_id(&self, id: Id) -> Result<GpgKey> {
        sqlx::query_as::<_, GpgKey>(AssertSqlSafe(format!("SELECT {GPG_COLS} FROM gpg_keys WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn gpg_keys(&self, user: Id) -> Result<Vec<GpgKey>> {
        sqlx::query_as::<_, GpgKey>(AssertSqlSafe(format!("SELECT {GPG_COLS} FROM gpg_keys WHERE user_id = $1 ORDER BY id")))
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Keys whose primary key id, fingerprint, or any subkey id/fingerprint equals `needle`
    /// (upper-case hex). Used to find the signer of a commit across all users.
    pub async fn gpg_keys_matching(&self, needle: &str) -> Result<Vec<GpgKey>> {
        let n = needle.to_uppercase();
        let short = if n.len() > 16 { n[n.len() - 16..].to_string() } else { n.clone() };
        sqlx::query_as::<_, GpgKey>(AssertSqlSafe(format!(
            "SELECT {GPG_COLS} FROM gpg_keys WHERE key_id = $1 OR fingerprint = $2 OR subkeys LIKE $3 OR subkeys LIKE $4"
        )))
        .bind(&short)
        .bind(&n)
        .bind(format!("%\"{short}\"%"))
        .bind(format!("%\"{n}\"%"))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn delete_gpg_key(&self, user: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM gpg_keys WHERE id = $1 AND user_id = $2")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    // ----- SSH signing keys -----

    pub async fn add_ssh_signing_key(&self, user: Id, title: &str, key: &str, fingerprint: &str) -> Result<SshSigningKey> {
        let id = reef_core::id::next();
        sqlx::query("INSERT INTO ssh_signing_keys (id, user_id, key, title, fingerprint, created_at) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(id.0)
            .bind(user.0)
            .bind(key)
            .bind(title)
            .bind(fingerprint)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::Conflict(_) => Error::field("SshSigningKey", "key", "already_exists", "key is already in use"),
                o => o,
            })?;
        self.ssh_signing_key_by_id(id).await
    }

    pub async fn ssh_signing_key_by_id(&self, id: Id) -> Result<SshSigningKey> {
        sqlx::query_as::<_, SshSigningKey>(AssertSqlSafe(format!("SELECT {SSK_COLS} FROM ssh_signing_keys WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn ssh_signing_keys(&self, user: Id) -> Result<Vec<SshSigningKey>> {
        sqlx::query_as::<_, SshSigningKey>(AssertSqlSafe(format!("SELECT {SSK_COLS} FROM ssh_signing_keys WHERE user_id = $1 ORDER BY id")))
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn delete_ssh_signing_key(&self, user: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM ssh_signing_keys WHERE id = $1 AND user_id = $2")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }
}

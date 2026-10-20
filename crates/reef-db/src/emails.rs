//! Email addresses of a user. Each account has one primary address (mirrored into
//! `accounts.email`) and any number of secondary ones; commits authored under any of them
//! resolve to the account. Address verification (mail round-trip) is not implemented: rows
//! added by the account owner are stored as verified.

use crate::accounts::Account;
use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::AssertSqlSafe;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserEmail {
    pub id: i64,
    pub user_id: i64,
    pub email: String,
    pub email_lower: String,
    pub verified: i64,
    pub is_primary: i64,
    pub visibility: String,
    pub created_at: i64,
}

impl UserEmail {
    pub fn is_primary(&self) -> bool {
        self.is_primary != 0
    }
    pub fn is_verified(&self) -> bool {
        self.verified != 0
    }
}

const COLS: &str = "id, user_id, email, email_lower, verified, is_primary, visibility, created_at";

fn valid(email: &str) -> Result<()> {
    let e = email.trim();
    if e.len() < 3 || e.len() > 254 || !e.contains('@') || e.starts_with('@') || e.ends_with('@') || e.chars().any(char::is_whitespace) {
        return Err(Error::field("Email", "email", "invalid", format!("{email} is not a valid email address")));
    }
    Ok(())
}

impl Db {
    /// All addresses of a user, primary first.
    pub async fn user_emails(&self, user: Id) -> Result<Vec<UserEmail>> {
        sqlx::query_as::<_, UserEmail>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM user_emails WHERE user_id = $1 ORDER BY is_primary DESC, id"
        )))
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// The user owning `email` (any of their addresses, case-insensitive).
    pub async fn user_by_any_email(&self, email: &str) -> Result<Account> {
        let row = sqlx::query_as::<_, UserEmail>(AssertSqlSafe(format!("SELECT {COLS} FROM user_emails WHERE email_lower = $1")))
            .bind(email.trim().to_lowercase())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        match row {
            Some(r) => {
                let a = self.account_by_id(Id(r.user_id)).await?;
                if a.is_user() { Ok(a) } else { Err(Error::NotFound) }
            }
            None => self.user_by_login_or_email(email).await.and_then(|a| {
                // Fall back to the account's primary email when it has no user_emails row.
                if a.email.as_deref().map(|e| e.eq_ignore_ascii_case(email.trim())).unwrap_or(false) { Ok(a) } else { Err(Error::NotFound) }
            }),
        }
    }

    /// Add addresses to a user. Addresses already on the account are ignored; an address on
    /// another account is a conflict.
    pub async fn add_user_emails(&self, user: Id, emails: &[String]) -> Result<Vec<UserEmail>> {
        let existing = self.user_emails(user).await?;
        let has_primary = existing.iter().any(|e| e.is_primary());
        let mut first = !has_primary;
        for raw in emails {
            let email = raw.trim();
            valid(email)?;
            let lower = email.to_lowercase();
            if existing.iter().any(|e| e.email_lower == lower) {
                continue;
            }
            let owner = sqlx::query_scalar::<_, i64>("SELECT user_id FROM user_emails WHERE email_lower = $1")
                .bind(&lower)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?;
            if let Some(o) = owner {
                if o != user.0 {
                    return Err(Error::Conflict(format!("{email} is already in use by another account")));
                }
                continue;
            }
            // Also refuse an address that is another account's legacy primary.
            if let Ok(a) = self.user_by_login_or_email(&lower).await
                && a.id != user.0
                && a.email.as_deref().map(|e| e.eq_ignore_ascii_case(email)).unwrap_or(false)
            {
                return Err(Error::Conflict(format!("{email} is already in use by another account")));
            }
            let id = reef_core::id::next();
            sqlx::query("INSERT INTO user_emails (id, user_id, email, email_lower, verified, is_primary, visibility, created_at) VALUES ($1, $2, $3, $4, 1, $5, 'private', $6)")
                .bind(id.0)
                .bind(user.0)
                .bind(email)
                .bind(&lower)
                .bind(if first { 1i64 } else { 0 })
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(|e| match map_err(e) {
                    Error::Conflict(_) => Error::Conflict(format!("{email} is already in use by another account")),
                    o => o,
                })?;
            if first {
                self.sync_primary_email(user, email).await?;
                first = false;
            }
        }
        self.user_emails(user).await
    }

    /// Remove addresses. The primary address cannot be removed.
    pub async fn delete_user_emails(&self, user: Id, emails: &[String]) -> Result<()> {
        for raw in emails {
            let lower = raw.trim().to_lowercase();
            let row = sqlx::query_as::<_, UserEmail>(AssertSqlSafe(format!(
                "SELECT {COLS} FROM user_emails WHERE user_id = $1 AND email_lower = $2"
            )))
            .bind(user.0)
            .bind(&lower)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
            let Some(row) = row else { continue };
            if row.is_primary() {
                return Err(Error::validation(format!("{} is your primary email address; choose another primary first", row.email)));
            }
            sqlx::query("DELETE FROM user_emails WHERE id = $1").bind(row.id).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }

    /// Make one of the user's addresses primary; mirrors it into `accounts.email`.
    pub async fn set_primary_email(&self, user: Id, email: &str) -> Result<Vec<UserEmail>> {
        let lower = email.trim().to_lowercase();
        let n = sqlx::query("UPDATE user_emails SET is_primary = 1 WHERE user_id = $1 AND email_lower = $2")
            .bind(user.0)
            .bind(&lower)
            .execute(&self.pool)
            .await
            .map_err(map_err)?
            .rows_affected();
        if n == 0 {
            return Err(Error::NotFound);
        }
        sqlx::query("UPDATE user_emails SET is_primary = 0 WHERE user_id = $1 AND email_lower <> $2")
            .bind(user.0)
            .bind(&lower)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        let stored = sqlx::query_scalar::<_, String>("SELECT email FROM user_emails WHERE user_id = $1 AND email_lower = $2")
            .bind(user.0)
            .bind(&lower)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        self.sync_primary_email(user, &stored).await?;
        self.user_emails(user).await
    }

    /// Visibility of the primary address ('public' or 'private'), as on GitHub.
    pub async fn set_email_visibility(&self, user: Id, visibility: &str) -> Result<Vec<UserEmail>> {
        if visibility != "public" && visibility != "private" {
            return Err(Error::validation("visibility must be public or private"));
        }
        sqlx::query("UPDATE user_emails SET visibility = $1 WHERE user_id = $2")
            .bind(visibility)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.user_emails(user).await
    }

    /// Keep `accounts.email` equal to the primary address without going through `update_user`
    /// (which would recurse into the email table).
    pub(crate) async fn sync_primary_email(&self, user: Id, email: &str) -> Result<()> {
        sqlx::query("UPDATE accounts SET email = $1, email_lower = $2, updated_at = $3 WHERE id = $4")
            .bind(email)
            .bind(email.to_lowercase())
            .bind(now())
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::Conflict(_) => Error::Conflict(format!("{email} is already in use by another account")),
                o => o,
            })?;
        Ok(())
    }

    /// Ensure a primary row exists for `email` (used when the account email changes through
    /// the profile API): adds the address if missing and makes it the primary.
    pub(crate) async fn ensure_primary_email_row(&self, user: Id, email: &str) -> Result<()> {
        let lower = email.trim().to_lowercase();
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM user_emails WHERE user_id = $1 AND email_lower = $2")
            .bind(user.0)
            .bind(&lower)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        if exists == 0 {
            let id = reef_core::id::next();
            sqlx::query("INSERT INTO user_emails (id, user_id, email, email_lower, verified, is_primary, visibility, created_at) VALUES ($1, $2, $3, $4, 1, 1, 'private', $5)")
                .bind(id.0)
                .bind(user.0)
                .bind(email.trim())
                .bind(&lower)
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(|e| match map_err(e) {
                    Error::Conflict(_) => Error::Conflict(format!("{email} is already in use by another account")),
                    o => o,
                })?;
        } else {
            sqlx::query("UPDATE user_emails SET is_primary = 1 WHERE user_id = $1 AND email_lower = $2")
                .bind(user.0)
                .bind(&lower)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        sqlx::query("UPDATE user_emails SET is_primary = 0 WHERE user_id = $1 AND email_lower <> $2")
            .bind(user.0)
            .bind(&lower)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }
}

//! Users and organizations. Both live in `accounts`; `kind` tells them apart.

use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result, names, token};
use sqlx::AssertSqlSafe;
use sqlx::Row;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    pub id: i64,
    pub kind: String,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub blog: Option<String>,
    pub avatar_url: Option<String>,
    pub is_admin: i64,
    pub suspended: i64,
    pub description: Option<String>,
    pub billing_email: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub avatar_updated_at: Option<i64>,
    /// Identity provider's id for a mirrored organization.
    pub external_org_id: Option<String>,
    /// Set on the Actions bot identity when a job token is used: the only repository it may write to.
    #[sqlx(skip)]
    pub job_repo: Option<i64>,
    /// Set when the caller is a job token: the job it belongs to (rate limits are per job).
    #[sqlx(skip)]
    pub job_id: Option<i64>,
}

impl Account {
    pub fn is_org(&self) -> bool {
        self.kind == "org"
    }
    pub fn is_user(&self) -> bool {
        self.kind == "user"
    }
    pub fn is_admin(&self) -> bool {
        self.is_admin != 0
    }
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

const COLS: &str = "id, kind, login, name, email, bio, company, location, blog, avatar_url, is_admin, suspended, description, billing_email, created_at, updated_at, avatar_updated_at, external_org_id";

pub struct NewUser<'a> {
    pub login: &'a str,
    pub email: &'a str,
    pub password: Option<&'a str>,
    pub name: Option<&'a str>,
    pub is_admin: bool,
}

#[derive(Default)]
pub struct UserPatch {
    pub name: Option<Option<String>>,
    pub email: Option<String>,
    pub bio: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub blog: Option<Option<String>>,
}

impl Db {
    pub async fn create_user(&self, u: NewUser<'_>) -> Result<Account> {
        names::validate_login(u.login)?;
        if !u.email.contains('@') {
            return Err(Error::field("User", "email", "invalid", "email is invalid"));
        }
        let hash = match u.password {
            Some(p) => Some(hash_password(p)?),
            None => None,
        };
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query(
            "INSERT INTO accounts (id, kind, login, login_lower, name, email, email_lower, password_hash, is_admin, suspended, created_at, updated_at) \
             VALUES ($1, 'user', $2, $3, $4, $5, $6, $7, $8, 0, $9, $9)",
        )
        .bind(id.0)
        .bind(u.login)
        .bind(u.login.to_lowercase())
        .bind(u.name)
        .bind(u.email)
        .bind(u.email.to_lowercase())
        .bind(hash)
        .bind(b(u.is_admin))
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("User", "login", "already_exists", "login or email already taken"),
            other => other,
        })?;
        // The account email is also the primary row of the user's address list.
        let _ = self.ensure_primary_email_row(id, u.email).await;
        self.account_by_id(id).await
    }

    pub async fn create_org(&self, login: &str, owner: Id, name: Option<&str>, description: Option<&str>) -> Result<Account> {
        names::validate_login(login)?;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query(
            "INSERT INTO accounts (id, kind, login, login_lower, name, description, created_at, updated_at) \
             VALUES ($1, 'org', $2, $3, $4, $5, $6, $6)",
        )
        .bind(id.0)
        .bind(login)
        .bind(login.to_lowercase())
        .bind(name)
        .bind(description)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("Organization", "login", "already_exists", "login already taken"),
            other => other,
        })?;
        self.add_org_member(id, owner, "admin").await?;
        self.account_by_id(id).await
    }

    /// Create (once) a system account such as the Actions bot; bypasses login validation.
    pub async fn ensure_system_user(&self, login: &str, name: &str) -> Result<Account> {
        if let Ok(a) = self.account_by_login(login).await {
            return Ok(a);
        }
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO accounts (id, kind, login, login_lower, name, email, email_lower, password_hash, is_admin, suspended, created_at, updated_at) VALUES ($1, 'user', $2, $3, $4, $5, $6, NULL, 0, 0, $7, $7)")
            .bind(id.0)
            .bind(login)
            .bind(login.to_lowercase())
            .bind(name)
            .bind(format!("{}@reef.invalid", login.replace(['[', ']'], "")))
            .bind(format!("{}@reef.invalid", login.replace(['[', ']'], "").to_lowercase()))
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.account_by_id(id).await
    }

    pub async fn org_by_external_id(&self, ext: &str) -> Result<Option<Account>> {
        sqlx::query_as::<_, Account>(AssertSqlSafe(format!("SELECT {COLS} FROM accounts WHERE kind = 'org' AND external_org_id = $1")))
            .bind(ext)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn set_external_org_id(&self, id: Id, ext: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE accounts SET external_org_id = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(ext)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn account_by_id(&self, id: Id) -> Result<Account> {
        sqlx::query_as::<_, Account>(AssertSqlSafe(format!("SELECT {COLS} FROM accounts WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn account_by_login(&self, login: &str) -> Result<Account> {
        sqlx::query_as::<_, Account>(AssertSqlSafe(format!("SELECT {COLS} FROM accounts WHERE login_lower = $1")))
            .bind(login.to_lowercase())
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn user_by_login_or_email(&self, s: &str) -> Result<Account> {
        sqlx::query_as::<_, Account>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM accounts WHERE kind = 'user' AND (login_lower = $1 OR email_lower = $1)"
        )))
        .bind(s.to_lowercase())
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn accounts_by_ids(&self, ids: &[i64]) -> Result<Vec<Account>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
        let sql = format!("SELECT {COLS} FROM accounts WHERE id IN ({})", placeholders.join(","));
        let mut q = sqlx::query_as::<_, Account>(AssertSqlSafe(sql.clone()));
        for id in ids {
            q = q.bind(*id);
        }
        q.fetch_all(&self.pool).await.map_err(map_err)
    }

    pub async fn list_accounts(&self, kind: &str, since: i64, limit: i64) -> Result<Vec<Account>> {
        sqlx::query_as::<_, Account>(AssertSqlSafe(format!("SELECT {COLS} FROM accounts WHERE kind = $1 AND id > $2 ORDER BY id LIMIT $3")))
            .bind(kind)
            .bind(since)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn count_accounts(&self, kind: &str) -> Result<i64> {
        let row =
            sqlx::query("SELECT COUNT(*) AS n FROM accounts WHERE kind = $1").bind(kind).fetch_one(&self.pool).await.map_err(map_err)?;
        row.try_get::<i64, _>("n").map_err(map_err)
    }

    pub async fn update_user(&self, id: Id, p: UserPatch) -> Result<Account> {
        // Build a dynamic SET list; every value is bound positionally.
        let mut sets: Vec<String> = Vec::new();
        let mut args: Vec<Option<String>> = Vec::new();
        let push = |col: &str, v: Option<String>, sets: &mut Vec<String>, args: &mut Vec<Option<String>>| {
            args.push(v);
            sets.push(format!("{col} = ${}", args.len()));
        };
        if let Some(v) = p.name {
            push("name", v, &mut sets, &mut args);
        }
        if let Some(v) = p.bio {
            push("bio", v, &mut sets, &mut args);
        }
        if let Some(v) = p.company {
            push("company", v, &mut sets, &mut args);
        }
        if let Some(v) = p.location {
            push("location", v, &mut sets, &mut args);
        }
        if let Some(v) = p.blog {
            push("blog", v, &mut sets, &mut args);
        }
        let mut new_email: Option<String> = None;
        if let Some(e) = p.email {
            if !e.contains('@') {
                return Err(Error::field("User", "email", "invalid", "email is invalid"));
            }
            push("email", Some(e.clone()), &mut sets, &mut args);
            push("email_lower", Some(e.to_lowercase()), &mut sets, &mut args);
            new_email = Some(e);
        }
        if sets.is_empty() {
            return self.account_by_id(id).await;
        }
        let n = args.len();
        let sql = format!("UPDATE accounts SET {}, updated_at = ${} WHERE id = ${}", sets.join(", "), n + 1, n + 2);
        let mut q = sqlx::query(AssertSqlSafe(sql.clone()));
        for a in args {
            q = q.bind(a);
        }
        q.bind(now()).bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if let Some(e) = new_email {
            self.ensure_primary_email_row(id, &e).await?;
        }
        self.account_by_id(id).await
    }

    pub async fn set_password(&self, id: Id, password: &str) -> Result<()> {
        let hash = hash_password(password)?;
        sqlx::query("UPDATE accounts SET password_hash = $1, updated_at = $2 WHERE id = $3")
            .bind(hash)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn has_password(&self, id: Id) -> Result<bool> {
        let row =
            sqlx::query("SELECT password_hash FROM accounts WHERE id = $1").bind(id.0).fetch_one(&self.pool).await.map_err(map_err)?;
        Ok(row.try_get::<Option<String>, _>("password_hash").map_err(map_err)?.is_some())
    }

    pub async fn set_admin(&self, id: Id, admin: bool) -> Result<()> {
        sqlx::query("UPDATE accounts SET is_admin = $1, updated_at = $2 WHERE id = $3")
            .bind(b(admin))
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Check a login/password pair. Returns the account on success. Timing is kept close
    /// to constant by always running the verifier, even for unknown logins.
    pub async fn verify_password(&self, login: &str, password: &str) -> Result<Account> {
        let acct = self.user_by_login_or_email(login).await.ok();
        let stored: Option<String> = match &acct {
            Some(a) => sqlx::query("SELECT password_hash FROM accounts WHERE id = $1")
                .bind(a.id)
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?
                .try_get::<Option<String>, _>("password_hash")
                .map_err(map_err)?,
            None => None,
        };
        let hash = stored.unwrap_or_else(|| DUMMY_HASH.to_string());
        let password = password.to_string();
        let ok = tokio::task::spawn_blocking(move || verify_hash(&password, &hash)).await.map_err(Error::internal)?;
        match (ok, acct) {
            (true, Some(a)) if a.suspended == 0 => Ok(a),
            _ => Err(Error::Unauthorized("Bad credentials".into())),
        }
    }
}

/// A valid argon2id hash of a random string, used to keep verification time constant
/// for unknown logins.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$Q2oq3n3vFh6c6mZ6b7l9aU0h6wHqm4C3Kk5tYq2n7hE";

pub fn hash_password(password: &str) -> Result<String> {
    use argon2::password_hash::PasswordHasher;
    if password.len() < 8 {
        return Err(Error::field("User", "password", "invalid", "password must be at least 8 characters"));
    }
    argon2::Argon2::default().hash_password(password.as_bytes()).map(|h| h.to_string()).map_err(|e| Error::msg(format!("hash: {e}")))
}

pub fn verify_hash(password: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordVerifier, phc::PasswordHash};
    match PasswordHash::new(hash) {
        Ok(parsed) => argon2::Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A short-lived, single-use secret for password resets and similar; not stored here.
pub fn one_time_secret() -> String {
    token::random_string(32)
}

//! Renaming users and organizations. Everything references accounts by id (repositories,
//! memberships, sessions, git storage), so a rename only touches the login columns.

use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result, names};

impl Db {
    /// Change an account's login. Fails with a validation error for invalid or reserved
    /// names and with a conflict when another account already uses it.
    pub async fn rename_account(&self, id: Id, new_login: &str) -> Result<()> {
        names::validate_login(new_login)?;
        let lower = new_login.to_ascii_lowercase();
        if let Ok(other) = self.account_by_login(new_login).await
            && other.id != id.0
        {
            return Err(Error::Conflict(format!("the name '{new_login}' is already taken")));
        }
        sqlx::query("UPDATE accounts SET login = $1, login_lower = $2, updated_at = $3 WHERE id = $4")
            .bind(new_login)
            .bind(&lower)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }
}

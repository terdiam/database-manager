//! Account recovery from the server console, for when nobody can sign in.

use argon2::{Argon2, PasswordHasher, password_hash::phc::PasswordHash};
use sqlx::PgPool;

use crate::{Error, audit, auth::check_password_rules};

/// Sets a new password and ends every session of that account.
/// Whoever can run this already controls the server and its database.
pub async fn reset_password(db: &PgPool, email: &str, password: &str) -> Result<(), String> {
    check_password_rules(password).map_err(|_| "the password must be 12 to 256 characters")?;
    let email = email.trim().to_lowercase();
    let hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h: PasswordHash| h.to_string())
        .map_err(|e| e.to_string())?;

    let fail = |e: sqlx::Error| e.to_string();
    let mut tx = db.begin().await.map_err(fail)?;
    let id: Option<uuid::Uuid> =
        sqlx::query_scalar("UPDATE users SET password_hash = $1 WHERE email = $2 RETURNING id")
            .bind(hash)
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await
            .map_err(fail)?;
    let Some(id) = id else {
        let admins: Vec<String> =
            sqlx::query_scalar("SELECT email FROM users WHERE role = 'admin' ORDER BY email")
                .fetch_all(&mut *tx)
                .await
                .map_err(fail)?;
        return Err(format!(
            "no account {email}. Administrators: {}",
            admins.join(", ")
        ));
    };
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(fail)?;
    let target = format!("password of {email} reset from the server console");
    audit::record(&mut *tx, None, "update", &target, "console", true)
        .await
        .map_err(|e: Error| match e {
            Error::Internal(detail) => detail,
            _ => "could not write the audit log".into(),
        })?;
    tx.commit().await.map_err(fail)
}

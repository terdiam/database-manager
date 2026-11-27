use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip, Role, User, check_password_rules, hash_password, normalize_account},
};

pub async fn list(State(app): State<AppState>, Auth(me): Auth) -> Result<Json<Vec<User>>, Error> {
    me.require_admin()?;
    let users = sqlx::query_as("SELECT id, name, email, role FROM users ORDER BY name LIMIT 500")
        .fetch_all(&app.db)
        .await?;
    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct NewUser {
    name: String,
    email: String,
    password: String,
    role: Role,
}

pub async fn create(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewUser>,
) -> Result<(StatusCode, Json<User>), Error> {
    me.require_admin()?;
    let (name, email) = normalize_account(&body.name, &body.email)?;
    check_password_rules(&body.password)?;
    let hash = hash_password(&app, body.password).await?;
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (name, email, password_hash, role) VALUES ($1, $2, $3, $4)
         ON CONFLICT (email) DO NOTHING RETURNING id, name, email, role",
    )
    .bind(name)
    .bind(email)
    .bind(hash)
    .bind(body.role.as_str())
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::Conflict("email_taken"))?;
    audit::record(
        &app.db,
        Some(&me),
        "create",
        &format!("user {}", user.email),
        &ip,
        true,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(user)))
}

#[derive(Deserialize)]
pub struct UserEdit {
    name: String,
    email: String,
    role: Role,
}

pub async fn update(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<UserEdit>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    let (name, email) = normalize_account(&body.name, &body.email)?;
    // Nobody changes their own role: that is how the last admin would disappear.
    if id == me.id && body.role.as_str() != "admin" {
        return Err(Error::Invalid("cannot_change_own_role"));
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('dbm-users'))")
        .execute(&mut *tx)
        .await?;
    // Re-check under the lock: two admins demoting each other must not leave none.
    let still_admin: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND role = 'admin')")
            .bind(me.id)
            .fetch_one(&mut *tx)
            .await?;
    if !still_admin {
        return Err(Error::Forbidden);
    }
    let found = sqlx::query("UPDATE users SET name = $2, email = $3, role = $4 WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(&email)
        .bind(body.role.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|e| match Error::from(e) {
            Error::Conflict("duplicate") => Error::Conflict("email_taken"),
            other => other,
        })?;
    if found.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    let target = format!("user {email}, role {}", body.role.as_str());
    audit::record(&mut *tx, Some(&me), "update", &target, &ip, true).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    if id == me.id {
        return Err(Error::Invalid("cannot_delete_self"));
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('dbm-users'))")
        .execute(&mut *tx)
        .await?;
    // Re-check under the lock: two admins deleting each other must not leave none.
    let still_admin: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND role = 'admin')")
            .bind(me.id)
            .fetch_one(&mut *tx)
            .await?;
    if !still_admin {
        return Err(Error::Forbidden);
    }
    let email: String = sqlx::query_scalar("DELETE FROM users WHERE id = $1 RETURNING email")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
    audit::record(
        &mut *tx,
        Some(&me),
        "delete",
        &format!("user {email}"),
        &ip,
        true,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

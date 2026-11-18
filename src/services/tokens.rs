//! Persönliche API-Tokens: Zugang für Programme (MCP-Server unter `/mcp`, z. B. nrzDashboard)
//! ohne Passwort und Cookie. Das Token (`nrznews_` + 32 zufällige Byte) gibt es nur beim
//! Anlegen; gespeichert wird nur sein SHA-256. Jedes Token gehört einem Benutzer und sieht
//! genau dessen Feeds und Artikel.

use crate::auth::session::{hash_token, new_token};
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::services::users;
use nrznews_types::{ApiToken, ApiTokenCreated, User};
use sqlx::SqlitePool;

pub const PREFIX: &str = "nrznews_";
/// Höchstens so viele Tokens je Benutzer.
const MAX_PER_USER: i64 = 20;
/// `last_used_at` seltener als einmal pro Minute schreiben.
const TOUCH_AFTER_SECS: i64 = 60;

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    name: String,
    prefix: String,
    created_at: i64,
    last_used_at: Option<i64>,
}

impl From<Row> for ApiToken {
    fn from(r: Row) -> Self {
        ApiToken {
            id: r.id,
            name: r.name,
            prefix: r.prefix,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        }
    }
}

pub async fn list(pool: &SqlitePool, user_id: i64) -> AppResult<Vec<ApiToken>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at FROM api_tokens
         WHERE user_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ApiToken::from).collect())
}

pub async fn create(pool: &SqlitePool, user_id: i64, name: &str) -> AppResult<ApiTokenCreated> {
    let name: String = name.trim().chars().take(60).collect();
    if name.is_empty() {
        return Err(AppError::bad(
            "Bitte einen Namen angeben, z. B. „nrzDashboard“.",
        ));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM api_tokens WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    if count >= MAX_PER_USER {
        return Err(AppError::bad(
            "Höchstens 20 Tokens – bitte zuerst ein altes löschen.",
        ));
    }
    let secret = format!("{PREFIX}{}", new_token());
    let prefix: String = secret.chars().take(PREFIX.len() + 4).collect();
    let ts = now();
    let id = sqlx::query("INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(user_id)
        .bind(&name)
        .bind(hash_token(&secret))
        .bind(&prefix)
        .bind(ts)
        .execute(pool)
        .await?
        .last_insert_rowid();
    Ok(ApiTokenCreated {
        token: ApiToken {
            id,
            name,
            prefix,
            created_at: ts,
            last_used_at: None,
        },
        secret,
    })
}

pub async fn delete(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<()> {
    let done = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Benutzer zum Token – `None`, wenn es unbekannt ist oder das Konto gesperrt wurde.
pub async fn authenticate(pool: &SqlitePool, secret: &str) -> AppResult<Option<User>> {
    let secret = secret.trim();
    if !secret.starts_with(PREFIX) {
        return Ok(None);
    }
    let row: Option<(i64, i64, Option<i64>)> =
        sqlx::query_as("SELECT id, user_id, last_used_at FROM api_tokens WHERE token_hash = ?")
            .bind(hash_token(secret))
            .fetch_optional(pool)
            .await?;
    let Some((id, user_id, last_used)) = row else {
        return Ok(None);
    };
    let Some(user) = users::find(pool, user_id).await?.filter(|u| !u.disabled) else {
        return Ok(None);
    };
    let ts = now();
    if last_used.is_none_or(|t| ts - t >= TOUCH_AFTER_SECS) {
        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
            .bind(ts)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(Some(user))
}

//! Sitzungen: Der Browser bekommt ein zufälliges Token (32 Byte) in einem privaten (von
//! Rocket verschlüsselten) Cookie; in der Datenbank liegt nur dessen SHA-256. Wer die
//! Datenbank liest, kann damit keine Sitzung übernehmen. Sitzungen laufen 30 Tage und
//! verlängern sich bei Benutzung.

use crate::db::now;
use crate::error::AppResult;
use crate::services::users;
use base64::Engine;
use nrznews_types::User;
use rocket::http::{Cookie, CookieJar, SameSite};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

pub const COOKIE: &str = "nrznews_session";
pub const LIFETIME_SECS: i64 = 30 * 86_400;
/// Seltener als einmal pro Stunde wird die Sitzung nicht verlängert (spart Schreibzugriffe).
const REFRESH_AFTER_SECS: i64 = 3_600;

pub fn new_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Neue Sitzung für den Benutzer; zurück kommt das Token für das Cookie.
pub async fn create(pool: &SqlitePool, user_id: i64, user_agent: &str) -> AppResult<String> {
    let token = new_token();
    let ts = now();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_seen_at, expires_at, user_agent)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(ts)
    .bind(ts)
    .bind(ts + LIFETIME_SECS)
    .bind(user_agent.chars().take(300).collect::<String>())
    .execute(pool)
    .await?;
    Ok(token)
}

/// Ergebnis von [`check`]: der Benutzer und ob die Sitzung gerade verlängert wurde (dann
/// sollte auch das Cookie neu gesetzt werden, sonst läuft es trotzdem nach 30 Tagen ab).
pub struct Checked {
    pub user: User,
    pub extended: bool,
}

/// Sitzung prüfen und – höchstens einmal pro Stunde – um 30 Tage verlängern. `None`, wenn
/// sie unbekannt oder abgelaufen ist oder das Konto gesperrt wurde.
pub async fn check(pool: &SqlitePool, token: &str) -> AppResult<Option<Checked>> {
    let hash = hash_token(token);
    let ts = now();
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT user_id, last_seen_at FROM sessions WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(&hash)
    .bind(ts)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, last_seen)) = row else {
        return Ok(None);
    };
    let Some(user) = users::find(pool, user_id).await?.filter(|u| !u.disabled) else {
        return Ok(None);
    };
    let extended = ts - last_seen >= REFRESH_AFTER_SECS;
    if extended {
        sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE token_hash = ?")
            .bind(ts)
            .bind(ts + LIFETIME_SECS)
            .bind(&hash)
            .execute(pool)
            .await?;
    }
    Ok(Some(Checked { user, extended }))
}

/// Benutzer zur Sitzung (siehe [`check`]).
pub async fn lookup(pool: &SqlitePool, token: &str) -> AppResult<Option<User>> {
    Ok(check(pool, token).await?.map(|c| c.user))
}

pub async fn destroy(pool: &SqlitePool, token: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(hash_token(token))
        .execute(pool)
        .await?;
    Ok(())
}

/// Alle Sitzungen eines Benutzers beenden (z. B. nach einem neuen Passwort durch den Admin).
pub async fn destroy_all(pool: &SqlitePool, user_id: i64) -> AppResult<u64> {
    Ok(sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected())
}

/// Alle Sitzungen ausser der aktuellen beenden (eigenes Passwort geändert: andere Geräte
/// müssen sich neu anmelden, dieses bleibt angemeldet).
pub async fn destroy_others(pool: &SqlitePool, user_id: i64, keep_token: &str) -> AppResult<u64> {
    Ok(
        sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash <> ?")
            .bind(user_id)
            .bind(hash_token(keep_token))
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Abgelaufene Sitzungen löschen (stündlich im Hintergrund).
pub async fn purge_expired(pool: &SqlitePool) -> AppResult<u64> {
    Ok(sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now())
        .execute(pool)
        .await?
        .rows_affected())
}

/// Cookie setzen: privat (verschlüsselt + signiert), HttpOnly, SameSite=Lax, 30 Tage.
/// Beim Verlängern der Sitzung wird es mit demselben Token neu gesetzt.
/// SameSite=Lax schickt das Cookie bei fremden POST-Anfragen nicht mit – Grundschutz gegen CSRF.
pub fn set_cookie(cookies: &CookieJar<'_>, token: String, secure: bool) {
    let cookie = Cookie::build((COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(rocket::time::Duration::seconds(LIFETIME_SECS))
        // Ohne ausdrückliches Ablaufdatum setzte Rocket bei privaten Cookies eine Woche
        .expires(
            rocket::time::OffsetDateTime::now_utc()
                + rocket::time::Duration::seconds(LIFETIME_SECS),
        );
    cookies.add_private(cookie);
}

pub fn token_from(cookies: &CookieJar<'_>) -> Option<String> {
    cookies.get_private(COOKIE).map(|c| c.value().to_string())
}

pub fn clear_cookie(cookies: &CookieJar<'_>) {
    cookies.remove_private(Cookie::build(COOKIE).path("/"));
}

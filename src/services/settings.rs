//! Instanzweite Einstellungen in `app_settings` (JSON-Werte).

use crate::db::now;
use crate::error::{AppError, AppResult};
use base64::Engine;
use nrznews_types::InstanceSettings;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::SqlitePool;

/// [`InstanceSettings`] (Registrierung, Abrufintervall, Aufbewahrung)
pub const INSTANCE: &str = "instance";
const COOKIE_SECRET: &str = "cookie_secret";

pub async fn get<T: DeserializeOwned>(pool: &SqlitePool, key: &str) -> AppResult<Option<T>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(raw.and_then(|v| serde_json::from_str(&v).ok()))
}

pub async fn set<T: Serialize>(pool: &SqlitePool, key: &str, value: &T) -> AppResult<()> {
    let json = serde_json::to_string(value)
        .map_err(|e| AppError::Config(format!("Einstellung „{key}“: {e}")))?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(json)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Einstellungen der Instanz; ohne gespeicherten Wert gelten die Vorgaben
/// (Registrierung laut `NRZNEWS_REGISTRATION_OPEN`).
pub async fn instance(
    pool: &SqlitePool,
    config: &crate::config::Config,
) -> AppResult<InstanceSettings> {
    Ok(get::<InstanceSettings>(pool, INSTANCE)
        .await?
        .unwrap_or_else(|| InstanceSettings {
            registration_open: config.registration_open,
            ..InstanceSettings::default()
        }))
}

/// Schlüssel für Rockets private Cookies. Liegt in der Datenbank, damit Sitzungen einen
/// Neustart überstehen, ohne dass ein weiteres Geheimnis konfiguriert werden muss.
pub async fn cookie_secret(pool: &SqlitePool) -> AppResult<String> {
    if let Some(secret) = get::<String>(pool, COOKIE_SECRET).await? {
        return Ok(secret);
    }
    let secret = base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>());
    set(pool, COOKIE_SECRET, &secret).await?;
    Ok(secret)
}

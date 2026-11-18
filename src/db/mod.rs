//! Die eine Datenbank: SQLite (WAL) mit FTS5. Benutzer, Feeds, Artikel und Volltextindex.

use crate::config::Config;
use crate::error::AppError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::time::Duration;

/// Jetzt als Unix-Sekunden (UTC).
pub fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// Öffnet (bzw. erstellt) die Datenbank.
pub async fn open(config: &Config) -> Result<SqlitePool, AppError> {
    std::fs::create_dir_all(&config.data_dir)?;
    let options = SqliteConnectOptions::new()
        .filename(config.database_path())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    Ok(SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with(options)
        .await?)
}

/// Prüfsumme (SHA-384), mit der nrzNews 1.1.0 die erste Migration gespeichert hat: Dort war ein
/// Kommentar darin umbenannt (nrzRSS → nrzNews). Die Datei ist wieder wie in 1.0.0; Datenbanken,
/// die mit 1.1.0 angelegt wurden, bekommen hier die richtige Prüfsumme.
const MIGRATION_1_CHECKSUM_1_1_0: &str =
    "ceb70adc51bf64ed2f01ebdb92e7fea220d30bcf1ed294c53f340e5c34529f74a43676cac4fdb094d0332436ceb6dbd4";

/// Schema auf den neuesten Stand bringen. Die Migrationen (`migrations/`) sind ins Programm
/// eingebettet; sqlx merkt sich in `_sqlx_migrations`, was schon gelaufen ist – samt Prüfsumme.
/// Angewendete Migrationen darum nie ändern, auch keine Kommentare (tests/schema.rs prüft das).
pub async fn migrate(pool: &SqlitePool) -> Result<(), AppError> {
    let migrator = sqlx::migrate!("./migrations");
    if let Some(m) = migrator.iter().find(|m| m.version == 1) {
        let old = hex_decode(MIGRATION_1_CHECKSUM_1_1_0);
        // Neue Datenbank: die Tabelle gibt es noch nicht – dann ist nichts zu reparieren.
        let _ = sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1 AND checksum = ?",
        )
        .bind(m.checksum.as_ref())
        .bind(old)
        .execute(pool)
        .await;
    }
    migrator
        .run(pool)
        .await
        .map_err(|e| AppError::Config(format!("Migration fehlgeschlagen: {e}")))
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Öffnen und migrieren in einem Schritt.
pub async fn connect(config: &Config) -> Result<SqlitePool, AppError> {
    let pool = open(config).await?;
    if let Err(e) = migrate(&pool).await {
        pool.close().await;
        return Err(e);
    }
    Ok(pool)
}

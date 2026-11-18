//! Ruft im Hintergrund alle fälligen Feeds ab und räumt alte Artikel auf.
//!
//! * Jede Minute (oder sofort nach [`wake`], z. B. nach einem OPML-Import) werden die Feeds
//!   mit `next_fetch_at ≤ jetzt` geholt, höchstens `fetch_concurrency` gleichzeitig.
//! * Erfolg: nächster Abruf nach dem Intervall der Instanz. Fehler: kurze deutsche Meldung
//!   in `last_error` und exponentielles Warten (Intervall × 2^Fehler, höchstens 24 h) –
//!   ein toter Feed soll nicht jede halbe Stunde angefragt werden.
//! * Artikel werden per `ON CONFLICT (feed_id, guid) DO UPDATE` gespeichert: geändert werden
//!   nur Titel und Inhalt, nie Gelesen/Gemerkt.
//! * Beim allerersten Abruf eines Feeds gelten Artikel älter als 30 Tage als gelesen – sonst
//!   stünden nach dem Abonnieren hunderte alte Artikel als „ungelesen“ da.
//! * Einmal am Tag: gelesene, nicht gemerkte Artikel älter als `retention_days` löschen – aber
//!   je Feed immer die neuesten 20 behalten, damit deren IDs beim nächsten Abruf nicht als
//!   „neu“ zurückkommen.

use super::discover;
use super::fetch::{Fetched, Fetcher};
use super::parse::{self, ParsedFeed};
use crate::config::Config;
use crate::db::now;
use crate::error::AppResult;
use crate::services::settings;
use futures::StreamExt;
use nrznews_types::RefreshResult;
use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Orbit, Rocket};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Notify;

/// Beim ersten Abruf gelten ältere Artikel als gelesen.
const FIRST_FETCH_UNREAD_DAYS: i64 = 30;
/// So viele Artikel je Feed überleben das Aufräumen immer.
const KEEP_PER_FEED: i64 = 20;
/// Längste Wartezeit nach Fehlern.
const MAX_BACKOFF_SECS: i64 = 24 * 3600;
/// Höchstens so viele Feeds je Durchlauf (der Rest kommt in der nächsten Minute).
const BATCH: i64 = 1000;

static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Weckt den Hintergrund-Abruf sofort (statt erst in einer Minute).
pub fn wake() {
    WAKE.notify_one();
}

pub struct Refresher;

#[rocket::async_trait]
impl Fairing for Refresher {
    fn info(&self) -> Info {
        Info {
            name: "Feeds abrufen",
            kind: Kind::Liftoff,
        }
    }

    async fn on_liftoff(&self, rocket: &Rocket<Orbit>) {
        let (Some(pool), Some(config)) = (
            rocket.state::<SqlitePool>().cloned(),
            rocket.state::<Config>().cloned(),
        ) else {
            return;
        };
        if !config.background {
            return;
        }
        let fetcher = rocket
            .state::<Fetcher>()
            .cloned()
            .unwrap_or_else(|| Fetcher::from_config(&config));
        let mut shutdown = rocket.shutdown();
        tokio::spawn(async move {
            let mut last_cleanup = 0i64;
            loop {
                tokio::select! {
                    r = refresh_due(&pool, &fetcher, &config) => match r {
                        Ok(n) if n.feeds > 0 => tracing::info!(
                            "{} Feeds abgerufen, {} neue Artikel, {} Fehler",
                            n.feeds, n.new_articles, n.errors
                        ),
                        Err(e) => tracing::warn!("Feeds abrufen: {e}"),
                        _ => {}
                    },
                    _ = &mut shutdown => break,
                }
                if now() - last_cleanup >= 24 * 3600 {
                    last_cleanup = now();
                    match cleanup(&pool, &config).await {
                        Ok(n) if n > 0 => tracing::info!("{n} alte Artikel gelöscht"),
                        Err(e) => tracing::warn!("Artikel aufräumen: {e}"),
                        _ => {}
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                    _ = WAKE.notified() => {}
                    _ = &mut shutdown => break,
                }
            }
        });
    }
}

/// Was ein Abruf ergab.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub new_articles: u32,
    /// Meldung, wenn der Abruf fehlschlug (steht auch in `feeds.last_error`)
    pub error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FeedRow {
    id: i64,
    user_id: i64,
    feed_url: String,
    icon_url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    last_fetched_at: Option<i64>,
    error_count: i64,
}

/// Holt einen Feed und speichert neue Artikel. Fehler beim Abrufen oder Lesen landen im Feed
/// (`last_error`) und im Ergebnis; nur Datenbankfehler sind ein `Err`. Die Berechtigung prüft
/// der Aufrufer.
pub async fn refresh_feed(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    refresh_minutes: u32,
    feed_id: i64,
) -> AppResult<Outcome> {
    let row: FeedRow = sqlx::query_as(
        "SELECT id, user_id, feed_url, icon_url, etag, last_modified, last_fetched_at, error_count
         FROM feeds WHERE id = ?",
    )
    .bind(feed_id)
    .fetch_one(pool)
    .await?;
    let interval = i64::from(refresh_minutes.clamp(5, 1440)) * 60;
    let fetched = fetcher
        .fetch(
            &row.feed_url,
            row.etag.as_deref(),
            row.last_modified.as_deref(),
        )
        .await;
    let mut page = match fetched {
        Err(msg) => return record_error(pool, &row, msg, interval).await,
        Ok(Fetched::NotModified) => {
            let t = now();
            sqlx::query(
                "UPDATE feeds SET last_fetched_at = ?, next_fetch_at = ?, last_error = NULL,
                 error_count = 0 WHERE id = ?",
            )
            .bind(t)
            .bind(t + interval)
            .bind(row.id)
            .execute(pool)
            .await?;
            return Ok(Outcome::default());
        }
        Ok(Fetched::Page(page)) => page,
    };
    // Lesen und Säubern kostet bei grossen Feeds spürbar Rechenzeit – nicht im Async-Thread
    let (body, url) = (std::mem::take(&mut page.body), row.feed_url.clone());
    let parsed = tokio::task::spawn_blocking(move || parse::parse(&body, &url, now()))
        .await
        .unwrap_or_else(|_| Err("Der Feed liess sich nicht lesen.".into()));
    let feed = match parsed {
        Ok(f) => f,
        Err(msg) => return record_error(pool, &row, msg, interval).await,
    };
    // Icon nur beim ersten Abruf suchen – danach bliebe es sonst bei jedem Abruf leer und
    // würde jedes Mal neu gesucht.
    let icon = if row.icon_url.is_empty() && row.last_fetched_at.is_none() {
        discover::find_icon(fetcher, &feed.site_url, &row.feed_url, &feed.icon_url).await
    } else {
        row.icon_url.clone()
    };
    let new_articles = store(
        pool,
        &row,
        &feed,
        &icon,
        page.etag,
        page.last_modified,
        interval,
    )
    .await?;
    Ok(Outcome {
        new_articles,
        error: None,
    })
}

async fn record_error(
    pool: &SqlitePool,
    row: &FeedRow,
    msg: String,
    interval: i64,
) -> AppResult<Outcome> {
    let errors = row.error_count + 1;
    let backoff = interval
        .saturating_mul(1i64 << errors.clamp(0, 20))
        .min(MAX_BACKOFF_SECS);
    sqlx::query("UPDATE feeds SET last_error = ?, error_count = ?, next_fetch_at = ? WHERE id = ?")
        .bind(&msg)
        .bind(errors)
        .bind(now() + backoff)
        .bind(row.id)
        .execute(pool)
        .await?;
    Ok(Outcome {
        new_articles: 0,
        error: Some(msg),
    })
}

/// Artikel und Feed-Daten in einer Transaktion speichern. Gibt die Zahl neuer Artikel zurück.
async fn store(
    pool: &SqlitePool,
    row: &FeedRow,
    feed: &ParsedFeed,
    icon: &str,
    etag: Option<String>,
    last_modified: Option<String>,
    interval: i64,
) -> AppResult<u32> {
    let t = now();
    let first = row.last_fetched_at.is_none();
    let guids: Vec<&str> = feed.entries.iter().map(|e| e.guid.as_str()).collect();
    let guids_json = serde_json::to_string(&guids).unwrap_or_else(|_| "[]".into());

    // IMMEDIATE: gleich als Schreiber beginnen. Sonst scheitert das spätere Schreiben mit
    // „database is locked“, wenn ein paralleler Abruf inzwischen geschrieben hat.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing: HashSet<String> = sqlx::query_scalar(
        "SELECT guid FROM articles WHERE feed_id = ? AND guid IN (SELECT value FROM json_each(?))",
    )
    .bind(row.id)
    .bind(&guids_json)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();

    let mut new_guids: HashSet<&str> = HashSet::new();
    for e in &feed.entries {
        let read = first && e.published_at < t - FIRST_FETCH_UNREAD_DAYS * 86400;
        sqlx::query(
            "INSERT INTO articles (feed_id, user_id, guid, url, title, author, content_html,
                content_text, image_url, published_at, fetched_at, read, read_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (feed_id, guid) DO UPDATE SET
                title = excluded.title,
                content_html = excluded.content_html,
                content_text = excluded.content_text
             WHERE articles.title IS NOT excluded.title
                OR articles.content_html IS NOT excluded.content_html",
        )
        .bind(row.id)
        .bind(row.user_id)
        .bind(&e.guid)
        .bind(&e.url)
        .bind(&e.title)
        .bind(&e.author)
        .bind(&e.content_html)
        .bind(&e.content_text)
        .bind(&e.image_url)
        .bind(e.published_at)
        .bind(t)
        .bind(read)
        .bind(read.then_some(t))
        .execute(&mut *tx)
        .await?;
        if !existing.contains(&e.guid) {
            new_guids.insert(e.guid.as_str());
        }
    }

    sqlx::query(
        "UPDATE feeds SET
            title = CASE WHEN title = '' OR title = original_title THEN ?1 ELSE title END,
            original_title = ?1,
            site_url = CASE WHEN ?2 <> '' THEN ?2 ELSE site_url END,
            description = ?3,
            icon_url = ?4,
            etag = ?5,
            last_modified = ?6,
            last_fetched_at = ?7,
            next_fetch_at = ?8,
            last_error = NULL,
            error_count = 0
         WHERE id = ?9",
    )
    .bind(&feed.title)
    .bind(&feed.site_url)
    .bind(&feed.description)
    .bind(icon)
    .bind(etag)
    .bind(last_modified)
    .bind(t)
    .bind(t + interval)
    .bind(row.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(new_guids.len() as u32)
}

/// Mehrere Feeds gleichzeitig abrufen (höchstens `config.fetch_concurrency`).
pub async fn refresh_many(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    config: &Config,
    ids: Vec<i64>,
) -> AppResult<RefreshResult> {
    let minutes = settings::instance(pool, config).await?.refresh_minutes;
    let results: Vec<AppResult<Outcome>> = futures::stream::iter(ids)
        .map(|id| refresh_feed(pool, fetcher, minutes, id))
        .buffer_unordered(config.fetch_concurrency.clamp(1, 64))
        .collect()
        .await;
    let mut out = RefreshResult {
        feeds: 0,
        new_articles: 0,
        errors: 0,
    };
    for r in results {
        out.feeds += 1;
        match r {
            Ok(o) => {
                out.new_articles += o.new_articles;
                out.errors += u32::from(o.error.is_some());
            }
            // Z. B. gerade gelöscht – die anderen Feeds zählen trotzdem
            Err(e) => {
                tracing::warn!("Feed abrufen: {e}");
                out.errors += 1;
            }
        }
    }
    Ok(out)
}

/// Alle fälligen Feeds (aller Benutzer) abrufen.
pub async fn refresh_due(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    config: &Config,
) -> AppResult<RefreshResult> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM feeds WHERE next_fetch_at <= ? ORDER BY next_fetch_at, id LIMIT ?",
    )
    .bind(now())
    .bind(BATCH)
    .fetch_all(pool)
    .await?;
    refresh_many(pool, fetcher, config, ids).await
}

/// Aufräumen laut `retention_days` (0 = nie). Gibt die Zahl gelöschter Artikel zurück.
pub async fn cleanup(pool: &SqlitePool, config: &Config) -> AppResult<u64> {
    let days = settings::instance(pool, config).await?.retention_days;
    cleanup_older_than(pool, days).await
}

/// Löscht gelesene, nicht gemerkte Artikel, die vor mehr als `days` Tagen erschienen und
/// abgerufen wurden – ausser den neuesten [`KEEP_PER_FEED`] je Feed.
pub async fn cleanup_older_than(pool: &SqlitePool, days: u32) -> AppResult<u64> {
    if days == 0 {
        return Ok(0);
    }
    let cutoff = now() - i64::from(days) * 86400;
    let r = sqlx::query(
        "DELETE FROM articles
         WHERE read = 1 AND starred = 0 AND published_at < ?1 AND fetched_at < ?1
           AND id NOT IN (
             SELECT id FROM (
               SELECT id, ROW_NUMBER() OVER (
                 PARTITION BY feed_id ORDER BY published_at DESC, id DESC
               ) AS n FROM articles
             ) WHERE n <= ?2
           )",
    )
    .bind(cutoff)
    .bind(KEEP_PER_FEED)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

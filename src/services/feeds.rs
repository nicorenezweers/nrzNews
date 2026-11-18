//! Feeds eines Benutzers: abonnieren (auch über die Adresse einer Webseite), umbenennen,
//! verschieben, löschen, abrufen; der Überblick für die Seitenleiste; OPML.
//!
//! Jeder Benutzer hat eigene Abos (wie bei Miniflux) – jede Abfrage ist auf `user_id`
//! beschränkt, fremde Feeds sind „nicht gefunden“.

use crate::config::Config;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::feeds::fetch::Fetcher;
use crate::feeds::opml::{self, OpmlFeed};
use crate::feeds::{discover, refresher, sanitize};
use crate::services::{folders, settings};
use nrznews_types::{
    DiscoveredFeed, Feed, OpmlImportResult, Overview, RefreshResult, SubscribeInput,
    UpdateFeedInput,
};
use sqlx::SqlitePool;

/// Höchstens so viele Feeds je OPML-Import.
const MAX_IMPORT: usize = 2000;

/// Feeds mit Zählern. Die Zähler kommen aus einem Durchgang über die Artikel des Benutzers
/// (Index `articles_user_published`) statt aus zwei Unterabfragen je Feed.
macro_rules! select {
    () => {
        "SELECT f.id, f.folder_id, f.title, f.original_title, f.feed_url, f.site_url, f.icon_url,
                f.description, f.last_fetched_at, f.last_error, f.created_at,
                coalesce(c.unread, 0) AS unread, coalesce(c.total, 0) AS total
         FROM feeds f
         LEFT JOIN (SELECT feed_id, sum(read = 0) AS unread, count(*) AS total
                    FROM articles WHERE user_id = ?1 GROUP BY feed_id) c ON c.feed_id = f.id
         WHERE f.user_id = ?1"
    };
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    folder_id: Option<i64>,
    title: String,
    original_title: String,
    feed_url: String,
    site_url: String,
    icon_url: String,
    description: String,
    last_fetched_at: Option<i64>,
    last_error: Option<String>,
    created_at: i64,
    unread: i64,
    total: i64,
}

impl From<Row> for Feed {
    fn from(r: Row) -> Self {
        let title = if !r.title.trim().is_empty() {
            r.title
        } else if !r.original_title.trim().is_empty() {
            r.original_title
        } else {
            url::Url::parse(&r.feed_url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_else(|| r.feed_url.clone())
        };
        Feed {
            id: r.id,
            folder_id: r.folder_id,
            title,
            feed_url: r.feed_url,
            site_url: r.site_url,
            icon_url: r.icon_url,
            description: r.description,
            unread: r.unread,
            total: r.total,
            last_fetched_at: r.last_fetched_at,
            last_error: r.last_error.filter(|e| !e.is_empty()),
            created_at: r.created_at,
        }
    }
}

pub async fn list(pool: &SqlitePool, user_id: i64) -> AppResult<Vec<Feed>> {
    let rows: Vec<Row> = sqlx::query_as(concat!(
        select!(),
        " ORDER BY lower(CASE WHEN f.title <> '' THEN f.title ELSE f.original_title END), f.id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Feed::from).collect())
}

pub async fn get(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<Feed> {
    // Ein Feed: Zähler nur über dessen Artikel (Index `articles_feed_published`)
    let row: Option<Row> = sqlx::query_as(
        "SELECT f.id, f.folder_id, f.title, f.original_title, f.feed_url, f.site_url, f.icon_url,
                f.description, f.last_fetched_at, f.last_error, f.created_at,
                (SELECT count(*) FROM articles a WHERE a.feed_id = f.id AND a.read = 0) AS unread,
                (SELECT count(*) FROM articles a WHERE a.feed_id = f.id) AS total
         FROM feeds f WHERE f.user_id = ? AND f.id = ?",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(Feed::from).ok_or(AppError::NotFound)
}

/// Alles für die Seitenleiste: Ordner, Feeds und die Zähler der Ansichten.
pub async fn overview(pool: &SqlitePool, user_id: i64) -> AppResult<Overview> {
    let (unread, starred, today): (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(read = 0), 0), coalesce(sum(starred = 1), 0),
                coalesce(sum(read = 0 AND published_at >= ?2), 0)
         FROM articles WHERE user_id = ?1",
    )
    .bind(user_id)
    .bind(now() - 86400)
    .fetch_one(pool)
    .await?;
    Ok(Overview {
        folders: folders::list(pool, user_id).await?,
        feeds: list(pool, user_id).await?,
        unread,
        starred,
        today,
    })
}

/// Hat der Benutzer diese Feed-Adresse schon?
async fn subscribed(pool: &SqlitePool, user_id: i64, url: &str) -> AppResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM feeds WHERE user_id = ? AND feed_url = ?")
            .bind(user_id)
            .bind(url)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

const DUPLICATE: &str = "Diesen Feed hast du schon abonniert.";

fn clean_title(title: &str) -> String {
    sanitize::collapse(title).chars().take(200).collect()
}

/// Feeds zu einer Adresse finden (Webseite oder Feed).
pub async fn discover(fetcher: &Fetcher, url: &str) -> AppResult<Vec<DiscoveredFeed>> {
    discover::discover(fetcher, url)
        .await
        .map_err(AppError::BadRequest)
}

/// Abonnieren: Adresse einer Webseite oder eines Feeds; der Feed wird sofort abgerufen.
pub async fn subscribe(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    config: &Config,
    user_id: i64,
    input: &SubscribeInput,
) -> AppResult<Feed> {
    if let Some(folder) = input.folder_id {
        folders::ensure_owned(pool, user_id, folder).await?;
    }
    let normalized = discover::normalize_url(&input.url).map_err(AppError::BadRequest)?;
    if subscribed(pool, user_id, normalized.as_str()).await?
        || subscribed(pool, user_id, input.url.trim()).await?
    {
        return Err(AppError::Conflict(DUPLICATE.into()));
    }
    let found = discover::discover(fetcher, normalized.as_str())
        .await
        .map_err(AppError::BadRequest)?;
    let Some(feed) = found.into_iter().next() else {
        return Err(AppError::bad(
            "Unter dieser Adresse habe ich keinen Feed gefunden.",
        ));
    };
    if subscribed(pool, user_id, &feed.url).await? {
        return Err(AppError::Conflict(DUPLICATE.into()));
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO feeds (user_id, folder_id, title, feed_url, next_fetch_at, created_at)
         VALUES (?, ?, ?, ?, 0, ?) RETURNING id",
    )
    .bind(user_id)
    .bind(input.folder_id)
    .bind(clean_title(&input.title))
    .bind(&feed.url)
    .bind(now())
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::unique(e, DUPLICATE))?;
    let minutes = settings::instance(pool, config).await?.refresh_minutes;
    refresher::refresh_feed(pool, fetcher, minutes, id).await?;
    get(pool, user_id, id).await
}

/// Name und Ordner setzen; leerer Name = wieder der Titel laut Feed.
pub async fn update(
    pool: &SqlitePool,
    user_id: i64,
    id: i64,
    input: &UpdateFeedInput,
) -> AppResult<Feed> {
    if let Some(folder) = input.folder_id {
        folders::ensure_owned(pool, user_id, folder).await?;
    }
    let title = clean_title(&input.title);
    let r = sqlx::query(
        "UPDATE feeds SET title = CASE WHEN ?1 = '' THEN original_title ELSE ?1 END, folder_id = ?2
         WHERE id = ?3 AND user_id = ?4",
    )
    .bind(&title)
    .bind(input.folder_id)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    get(pool, user_id, id).await
}

/// Feed mitsamt Artikeln löschen (`ON DELETE CASCADE`, der Volltextindex folgt per Trigger).
pub async fn delete(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<()> {
    let r = sqlx::query("DELETE FROM feeds WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Einen eigenen Feed sofort abrufen.
pub async fn refresh(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    config: &Config,
    user_id: i64,
    id: i64,
) -> AppResult<Feed> {
    get(pool, user_id, id).await?;
    let minutes = settings::instance(pool, config).await?.refresh_minutes;
    refresher::refresh_feed(pool, fetcher, minutes, id).await?;
    get(pool, user_id, id).await
}

/// Alle eigenen Feeds sofort abrufen.
pub async fn refresh_all(
    pool: &SqlitePool,
    fetcher: &Fetcher,
    config: &Config,
    user_id: i64,
) -> AppResult<RefreshResult> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM feeds WHERE user_id = ? ORDER BY id")
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    refresher::refresh_many(pool, fetcher, config, ids).await
}

// ------------------------------------------------------------------ OPML

/// Alle Feeds des Benutzers als OPML.
pub async fn export_opml(pool: &SqlitePool, user_id: i64, owner: &str) -> AppResult<String> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT f.title, f.original_title, f.feed_url, f.site_url, coalesce(d.name, '')
         FROM feeds f LEFT JOIN folders d ON d.id = f.folder_id
         WHERE f.user_id = ?
         ORDER BY d.position, d.name COLLATE NOCASE, lower(f.title), f.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let feeds: Vec<OpmlFeed> = rows
        .into_iter()
        .map(|(title, original, xml_url, html_url, folder)| OpmlFeed {
            title: if title.is_empty() { original } else { title },
            xml_url,
            html_url,
            folder,
        })
        .collect();
    opml::export(&feeds, owner).map_err(AppError::Config)
}

/// OPML importieren: fehlende Ordner anlegen, vorhandene Feeds überspringen. Abgerufen wird
/// nicht hier (bei hunderten Feeds dauerte das Minuten), sondern gleich im Hintergrund.
pub async fn import_opml(
    pool: &SqlitePool,
    user_id: i64,
    text: &str,
) -> AppResult<OpmlImportResult> {
    let feeds = opml::parse(text).map_err(AppError::BadRequest)?;
    let mut result = OpmlImportResult {
        added: 0,
        skipped: 0,
        folders: 0,
        errors: Vec::new(),
    };
    if feeds.is_empty() {
        return Err(AppError::bad("In der OPML-Datei stehen keine Feeds."));
    }
    if feeds.len() > MAX_IMPORT {
        return Err(AppError::bad(format!(
            "Zu viele Feeds in der Datei (höchstens {MAX_IMPORT})."
        )));
    }
    for f in feeds {
        let name = if f.title.is_empty() {
            f.xml_url.clone()
        } else {
            f.title.clone()
        };
        let url = match discover::normalize_url(&f.xml_url) {
            Ok(u) => u.to_string(),
            Err(e) => {
                result.errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if subscribed(pool, user_id, &url).await? || subscribed(pool, user_id, &f.xml_url).await? {
            result.skipped += 1;
            continue;
        }
        let folder_id = if f.folder.is_empty() {
            None
        } else {
            match folders::find_or_create(pool, user_id, &f.folder).await {
                Ok((id, created)) => {
                    result.folders += u32::from(created);
                    Some(id)
                }
                Err(AppError::BadRequest(e)) => {
                    result.errors.push(format!("{name}: {e}"));
                    None
                }
                Err(e) => return Err(e),
            }
        };
        let site_url = sanitize::resolve(&f.html_url, None).unwrap_or_default();
        let inserted = sqlx::query(
            "INSERT INTO feeds (user_id, folder_id, title, feed_url, site_url, next_fetch_at, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(user_id)
        .bind(folder_id)
        .bind(clean_title(&f.title))
        .bind(&url)
        .bind(site_url)
        .bind(now())
        .execute(pool)
        .await;
        match inserted {
            Ok(_) => result.added += 1,
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => result.skipped += 1,
            Err(e) => return Err(e.into()),
        }
    }
    if result.added > 0 {
        refresher::wake();
    }
    Ok(result)
}

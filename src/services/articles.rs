//! Artikel: Liste mit Filtern und Volltextsuche, Lesen, Gelesen/Gemerkt, „alles gelesen“.
//!
//! Blättern per Keyset-Cursor (`<published_at>:<id>`) statt `OFFSET`: neue Artikel, die
//! während des Lesens eintreffen, verschieben die nächste Seite nicht, und tiefe Seiten
//! bleiben schnell. Jede Abfrage ist auf `user_id` beschränkt.

use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::feeds::sanitize;
use nrznews_types::{
    Article, ArticlePage, ArticleState, ArticleStateInput, ArticleSummary, MarkReadInput,
    MarkReadResult, View,
};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

pub const DEFAULT_LIMIT: u32 = 40;
pub const MAX_LIMIT: u32 = 100;
/// Länge des Auszugs in der Liste (Zeichen).
const EXCERPT: usize = 300;
/// „Heute“ = die letzten 24 Stunden.
const DAY: i64 = 86400;

/// Filter für [`list`] (`GET /articles?view=&feed=&folder=&q=&cursor=&limit=`).
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub view: View,
    pub feed: Option<i64>,
    pub folder: Option<i64>,
    pub q: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Suchwörter für FTS5: jedes Wort als Zeichenkette in Anführungszeichen (Syntaxfehler sind
/// so unmöglich) mit `*` für die Präfixsuche. Wörter ohne Buchstaben/Ziffern fallen weg.
fn fts_terms(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .take(12)
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect()
}

/// Suchanfrage, bei der alle Wörter vorkommen müssen. `None`, wenn nichts Suchbares bleibt.
pub fn fts_query(input: &str) -> Option<String> {
    let terms = fts_terms(input);
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Wie [`fts_query`], aber mit ODER verknüpft (für Dieter: mehr Treffer, der Rang entscheidet).
pub fn fts_query_any(input: &str) -> Option<String> {
    let terms = fts_terms(input);
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn parse_cursor(c: &str) -> AppResult<(i64, i64)> {
    let bad = || AppError::bad("Ungültiger Cursor.");
    let (p, i) = c.split_once(':').ok_or_else(bad)?;
    Ok((
        p.trim().parse().map_err(|_| bad())?,
        i.trim().parse().map_err(|_| bad())?,
    ))
}

#[derive(sqlx::FromRow)]
struct SummaryRow {
    id: i64,
    feed_id: i64,
    feed_title: String,
    feed_icon: String,
    title: String,
    url: String,
    author: String,
    text_start: String,
    image_url: String,
    published_at: i64,
    read: bool,
    starred: bool,
}

/// Bedingungen für Ansicht, Feed und Ordner (gemeinsam für Liste und „alles gelesen“).
fn push_filters(
    qb: &mut QueryBuilder<Sqlite>,
    view: View,
    feed: Option<i64>,
    folder: Option<i64>,
    user_id: i64,
) {
    match view {
        View::All => {}
        View::Unread => {
            qb.push(" AND a.read = 0");
        }
        View::Starred => {
            qb.push(" AND a.starred = 1");
        }
        View::Today => {
            qb.push(" AND a.published_at >= ").push_bind(now() - DAY);
        }
    }
    if let Some(f) = feed {
        qb.push(" AND a.feed_id = ").push_bind(f);
    }
    if let Some(f) = folder {
        qb.push(" AND a.feed_id IN (SELECT id FROM feeds WHERE user_id = ")
            .push_bind(user_id)
            .push(" AND folder_id = ")
            .push_bind(f)
            .push(")");
    }
}

pub async fn list(pool: &SqlitePool, user_id: i64, q: &ListQuery) -> AppResult<ArticlePage> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as i64;
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT a.id, a.feed_id,
                CASE WHEN f.title <> '' THEN f.title WHEN f.original_title <> '' THEN f.original_title
                     ELSE f.feed_url END AS feed_title,
                f.icon_url AS feed_icon, a.title, a.url, a.author,
                substr(a.content_text, 1, 600) AS text_start, a.image_url, a.published_at,
                a.read, a.starred
         FROM articles a JOIN feeds f ON f.id = a.feed_id
         WHERE a.user_id = ",
    );
    qb.push_bind(user_id);
    push_filters(&mut qb, q.view, q.feed, q.folder, user_id);
    if let Some(search) = q.q.as_deref().filter(|s| !s.trim().is_empty()) {
        match fts_query(search) {
            Some(m) => {
                qb.push(" AND a.id IN (SELECT rowid FROM articles_fts WHERE articles_fts MATCH ")
                    .push_bind(m)
                    .push(")");
            }
            // Nur Satzzeichen gesucht: nichts kann passen
            None => {
                return Ok(ArticlePage {
                    items: Vec::new(),
                    next_cursor: None,
                })
            }
        }
    }
    if let Some(c) = q.cursor.as_deref().filter(|c| !c.is_empty()) {
        let (p, id) = parse_cursor(c)?;
        qb.push(" AND (a.published_at < ")
            .push_bind(p)
            .push(" OR (a.published_at = ")
            .push_bind(p)
            .push(" AND a.id < ")
            .push_bind(id)
            .push("))");
    }
    qb.push(" ORDER BY a.published_at DESC, a.id DESC LIMIT ")
        .push_bind(limit + 1);
    let mut rows: Vec<SummaryRow> = qb.build_query_as().fetch_all(pool).await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|r| format!("{}:{}", r.published_at, r.id))
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|r| ArticleSummary {
            id: r.id,
            feed_id: r.feed_id,
            feed_title: r.feed_title,
            feed_icon: r.feed_icon,
            title: r.title,
            url: r.url,
            author: r.author,
            excerpt: sanitize::excerpt(&r.text_start, EXCERPT),
            image_url: r.image_url,
            published_at: r.published_at,
            read: r.read,
            starred: r.starred,
        })
        .collect();
    Ok(ArticlePage { items, next_cursor })
}

#[derive(sqlx::FromRow)]
struct ArticleRow {
    id: i64,
    feed_id: i64,
    feed_title: String,
    feed_icon: String,
    feed_site_url: String,
    title: String,
    url: String,
    author: String,
    content_html: String,
    content_text: String,
    image_url: String,
    published_at: i64,
    read: bool,
    starred: bool,
    ai_summary: Option<String>,
}

/// Ein Artikel zum Lesen.
pub async fn get(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<Article> {
    let row: Option<ArticleRow> = sqlx::query_as(
        "SELECT a.id, a.feed_id,
                CASE WHEN f.title <> '' THEN f.title WHEN f.original_title <> '' THEN f.original_title
                     ELSE f.feed_url END AS feed_title,
                f.icon_url AS feed_icon, f.site_url AS feed_site_url, a.title, a.url, a.author,
                a.content_html, a.content_text, a.image_url, a.published_at, a.read, a.starred,
                a.ai_summary
         FROM articles a JOIN feeds f ON f.id = a.feed_id
         WHERE a.id = ? AND a.user_id = ?",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let r = row.ok_or(AppError::NotFound)?;
    Ok(Article {
        reading_minutes: sanitize::reading_minutes(&r.content_text),
        id: r.id,
        feed_id: r.feed_id,
        feed_title: r.feed_title,
        feed_icon: r.feed_icon,
        feed_site_url: r.feed_site_url,
        title: r.title,
        url: r.url,
        author: r.author,
        content_html: r.content_html,
        image_url: r.image_url,
        published_at: r.published_at,
        read: r.read,
        starred: r.starred,
        ai_summary: r.ai_summary.filter(|s| !s.is_empty()),
    })
}

/// Gelesen/Gemerkt setzen (nur gesetzte Felder).
pub async fn set_state(
    pool: &SqlitePool,
    user_id: i64,
    id: i64,
    input: &ArticleStateInput,
) -> AppResult<ArticleState> {
    let t = now();
    let row: Option<(i64, bool, bool)> = sqlx::query_as(
        "UPDATE articles SET
            read = coalesce(?1, read),
            read_at = CASE WHEN ?1 IS NULL THEN read_at WHEN ?1 = read THEN read_at
                           WHEN ?1 = 1 THEN ?2 ELSE NULL END,
            starred = coalesce(?3, starred),
            starred_at = CASE WHEN ?3 IS NULL THEN starred_at WHEN ?3 = starred THEN starred_at
                              WHEN ?3 = 1 THEN ?2 ELSE NULL END
         WHERE id = ?4 AND user_id = ?5
         RETURNING id, read, starred",
    )
    .bind(input.read)
    .bind(t)
    .bind(input.starred)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let (id, read, starred) = row.ok_or(AppError::NotFound)?;
    Ok(ArticleState { id, read, starred })
}

/// Alles als gelesen markieren – eingeschränkt auf Feed, Ordner oder Ansicht, und nur bis
/// `before` (damit während des Lesens eingetroffene Artikel ungelesen bleiben).
pub async fn mark_read(
    pool: &SqlitePool,
    user_id: i64,
    input: &MarkReadInput,
) -> AppResult<MarkReadResult> {
    let t = now();
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("UPDATE articles AS a SET read = 1, read_at = ");
    qb.push_bind(t)
        .push(" WHERE a.user_id = ")
        .push_bind(user_id)
        .push(" AND a.read = 0 AND a.published_at <= ")
        .push_bind(input.before.unwrap_or(t));
    push_filters(
        &mut qb,
        input.view.unwrap_or_default(),
        input.feed_id,
        input.folder_id,
        user_id,
    );
    let r = qb.build().execute(pool).await?;
    Ok(MarkReadResult {
        updated: r.rows_affected() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_terms_are_quoted() {
        assert_eq!(
            fts_query("rust rocket").as_deref(),
            Some("\"rust\"* \"rocket\"*")
        );
        assert_eq!(
            fts_query("a\"b OR (").as_deref(),
            Some("\"a\"\"b\"* \"OR\"*")
        );
        assert_eq!(fts_query(" - * "), None);
        assert_eq!(fts_query_any("a b").as_deref(), Some("\"a\"* OR \"b\"*"));
    }

    #[test]
    fn cursor_roundtrip() {
        assert_eq!(parse_cursor("1700000000:42").unwrap(), (1700000000, 42));
        assert!(parse_cursor("kaputt").is_err());
    }
}

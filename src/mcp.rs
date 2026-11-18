//! MCP-Server (Model Context Protocol, JSON-RPC 2.0) unter `/mcp` – für nrzDashboard und
//! KI-Agenten. Zugang mit einem persönlichen API-Token (`Authorization: Bearer nrznews_…`);
//! jedes Token sieht nur die Feeds und Artikel seines Benutzers. Nur lesend: kein Werkzeug
//! ändert etwas.
//!
//! Werkzeuge:
//! * `dashboard_overview` – Kennzahlen für eine Übersicht: Zähler, eine fertige Liste von
//!   Werten (`metrics`), Feeds mit den meisten ungelesenen, Feeds mit Fehlern, neueste Artikel
//! * `list_articles` – Artikel einer Ansicht, eines Feeds oder Ordners, mit Volltextsuche
//! * `get_article` – ein Artikel als reiner Text (mit gespeicherter KI-Zusammenfassung)
//! * `list_feeds` – Ordner und Feeds mit Zählern und Fehlern
//!
//! Die HTTP-Seite (Streamable HTTP, nur POST, Antworten als `application/json`) steht in
//! `routes/mcp.rs`; hier ist nur das Protokoll.

use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::services::{articles, feeds};
use nrznews_types::{ArticleSummary, User, View};
use serde_json::{json, Map, Value};
use sqlx::SqlitePool;

/// Unterstützte Protokollversionen, neueste zuerst.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "nrzNews ist ein RSS-/News-Reader. Für Kennzahlen (Ungelesen, Heute, \
Gemerkt, neue Artikel, Feeds mit Fehlern) und die neuesten Schlagzeilen: dashboard_overview. \
Artikel auflisten oder durchsuchen: list_articles; einen Artikel lesen: get_article. \
Zeiten sind Unix-Sekunden (UTC). Alle Werkzeuge lesen nur.";

// JSON-RPC-Fehlercodes
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// Antwort auf eine nicht lesbare Anfrage.
pub fn parse_error(message: impl Into<String>) -> Value {
    rpc_error(Value::Null, PARSE_ERROR, message)
}

/// Eine JSON-RPC-Nachricht (oder ein Stapel) beantworten. `None` bei reinen Benachrichtigungen
/// und Antworten des Clients – dann gibt es nichts zurückzuschicken (HTTP 202).
pub async fn handle(pool: &SqlitePool, user: &User, msg: Value) -> Option<Value> {
    match msg {
        Value::Array(items) => {
            if items.is_empty() {
                return Some(rpc_error(Value::Null, INVALID_REQUEST, "Leerer Stapel"));
            }
            let mut out = Vec::new();
            for m in items {
                if let Some(r) = handle_one(pool, user, m).await {
                    out.push(r);
                }
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
        m => handle_one(pool, user, m).await,
    }
}

async fn handle_one(pool: &SqlitePool, user: &User, msg: Value) -> Option<Value> {
    let Value::Object(obj) = msg else {
        return Some(rpc_error(
            Value::Null,
            INVALID_REQUEST,
            "Erwartet wird ein JSON-RPC-Objekt",
        ));
    };
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // Antwort oder Fehler des Clients auf eine Anfrage von uns – wir stellen keine
        return match id {
            Some(_) if obj.contains_key("result") || obj.contains_key("error") => None,
            Some(id) => Some(rpc_error(id, INVALID_REQUEST, "„method“ fehlt")),
            None => None,
        };
    };
    // Ohne id: Benachrichtigung (z. B. notifications/initialized) – nie beantworten
    let id = id?;
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => return Some(call_tool(pool, user, id, &params).await),
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err(rpc_error(
            id.clone(),
            METHOD_NOT_FOUND,
            format!("Unbekannte Methode: {other}"),
        )),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => e,
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "nrzNews", "title": "nrzNews", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

// ------------------------------------------------------------------ Werkzeuge

fn read_only() -> Value {
    json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
}

fn tool_list() -> Value {
    json!([
        {
            "name": "dashboard_overview",
            "title": "Überblick für ein Dashboard",
            "description": "Kennzahlen für eine Übersicht (z. B. nrzDashboard): Ungelesen, Heute, Gemerkt, \
    neue Artikel im Zeitraum, Anzahl Feeds und Feeds mit Fehlern – als `counts` und als fertige Liste \
    `metrics` (key, label, value, status). Dazu `topFeeds` (meiste ungelesene), `feedErrors`, \
    `latest` (neueste ungelesene Artikel), `lastFetchedAt`/`nextFetchAt`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "headlines": { "type": "integer", "minimum": 0, "maximum": 20, "default": 5,
                                   "description": "Wie viele neueste ungelesene Artikel in `latest`" },
                    "hours": { "type": "integer", "minimum": 1, "maximum": 168, "default": 24,
                               "description": "Zeitraum für „neue Artikel“ in Stunden" }
                },
                "additionalProperties": false
            },
            "annotations": read_only(),
        },
        {
            "name": "list_articles",
            "title": "Artikel auflisten oder suchen",
            "description": "Artikel, neueste zuerst. Ansicht unread (Standard), all, today (letzte 24 h) oder \
    starred; optional nur ein Feed oder Ordner, optional Volltextsuche in Titel und Inhalt. Für weitere \
    Seiten `nextCursor` als `cursor` mitschicken.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["unread", "all", "today", "starred"], "default": "unread" },
                    "feedId": { "type": "integer", "description": "Nur dieser Feed (IDs aus list_feeds)" },
                    "folderId": { "type": "integer", "description": "Nur dieser Ordner" },
                    "query": { "type": "string", "description": "Suchwörter (Präfixsuche, Umlaute egal)" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 },
                    "cursor": { "type": "string" }
                },
                "additionalProperties": false
            },
            "annotations": read_only(),
        },
        {
            "name": "get_article",
            "title": "Artikel lesen",
            "description": "Ein Artikel als reiner Text, mit Feed, Autor, Datum, Link zum Original, Lesezeit \
    und – falls schon erzeugt – der KI-Zusammenfassung.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Artikel-ID (aus list_articles oder dashboard_overview)" },
                    "maxChars": { "type": "integer", "minimum": 500, "maximum": 100000, "default": 20000,
                                  "description": "Text höchstens so lang (Zeichen)" }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": read_only(),
        },
        {
            "name": "list_feeds",
            "title": "Feeds und Ordner",
            "description": "Alle Ordner und abonnierten Feeds mit ungelesenen und gesamten Artikeln, letztem \
    Abruf und letztem Fehler.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": read_only(),
        },
    ])
}

/// Werkzeug aufrufen. Fehler beim Ausführen kommen als Ergebnis mit `isError` zurück (die KI
/// soll sie sehen), falsche Namen und Argumente als JSON-RPC-Fehler.
async fn call_tool(pool: &SqlitePool, user: &User, id: Value, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, INVALID_PARAMS, "„name“ fehlt");
    };
    let empty = Map::new();
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return rpc_error(id, INVALID_PARAMS, "„arguments“ muss ein Objekt sein"),
    };
    let res = match name {
        "dashboard_overview" => dashboard_overview(pool, user, args).await,
        "list_articles" => list_articles(pool, user, args).await,
        "get_article" => get_article(pool, user, args).await,
        "list_feeds" => list_feeds(pool, user).await,
        other => return rpc_error(id, INVALID_PARAMS, format!("Unbekanntes Werkzeug: {other}")),
    };
    let result = match res {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": v.to_string() }],
            "structuredContent": v,
            "isError": false,
        }),
        Err(ToolError::Invalid(msg)) => return rpc_error(id, INVALID_PARAMS, msg),
        Err(ToolError::App(e)) => {
            if matches!(e.status().code, 500..) {
                tracing::error!("MCP {name}: {e}");
                return rpc_error(id, INTERNAL_ERROR, e.public_message());
            }
            json!({ "content": [{ "type": "text", "text": e.public_message() }], "isError": true })
        }
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

enum ToolError {
    /// Falsche Argumente
    Invalid(String),
    App(AppError),
}

impl From<AppError> for ToolError {
    fn from(e: AppError) -> Self {
        ToolError::App(e)
    }
}

impl From<sqlx::Error> for ToolError {
    fn from(e: sqlx::Error) -> Self {
        ToolError::App(e.into())
    }
}

type ToolResult = Result<Value, ToolError>;

/// Ganzzahl-Argument in [min, max]; fehlt es, gilt `default`.
fn int_arg(
    args: &Map<String, Value>,
    key: &str,
    default: Option<i64>,
    min: i64,
    max: i64,
) -> Result<Option<i64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v
                .as_i64()
                .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or_else(|| ToolError::Invalid(format!("„{key}“ muss eine ganze Zahl sein")))?;
            if n < min || n > max {
                return Err(ToolError::Invalid(format!(
                    "„{key}“ muss zwischen {min} und {max} liegen"
                )));
            }
            Ok(Some(n))
        }
    }
}

fn str_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str()).filter(|s| !s.trim().is_empty())),
        Some(_) => Err(ToolError::Invalid(format!("„{key}“ muss ein Text sein"))),
    }
}

fn summary_json(a: &ArticleSummary) -> Value {
    json!({
        "id": a.id,
        "title": a.title,
        "feedId": a.feed_id,
        "feedTitle": a.feed_title,
        "url": a.url,
        "author": a.author,
        "excerpt": a.excerpt,
        "imageUrl": a.image_url,
        "publishedAt": a.published_at,
        "read": a.read,
        "starred": a.starred,
    })
}

fn metric(key: &str, label: &str, value: i64, status: &str, hint: &str) -> Value {
    json!({ "key": key, "label": label, "value": value, "status": status, "hint": hint })
}

async fn dashboard_overview(
    pool: &SqlitePool,
    user: &User,
    args: &Map<String, Value>,
) -> ToolResult {
    let headlines = int_arg(args, "headlines", Some(5), 0, 20)?.unwrap_or(5);
    let hours = int_arg(args, "hours", Some(24), 1, 168)?.unwrap_or(24);
    let ov = feeds::overview(pool, user.id).await?;
    let ts = now();
    let (new_in_period, next_fetch): (i64, Option<i64>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM articles WHERE user_id = ?1 AND fetched_at >= ?2),
                (SELECT min(next_fetch_at) FROM feeds WHERE user_id = ?1)",
    )
    .bind(user.id)
    .bind(ts - hours * 3600)
    .fetch_one(pool)
    .await?;
    let articles_total: i64 = ov.feeds.iter().map(|f| f.total).sum();
    let last_fetched = ov.feeds.iter().filter_map(|f| f.last_fetched_at).max();
    let errors: Vec<_> = ov
        .feeds
        .iter()
        .filter(|f| {
            f.last_error
                .as_deref()
                .is_some_and(|e| !e.trim().is_empty())
        })
        .collect();
    let mut top: Vec<_> = ov.feeds.iter().filter(|f| f.unread > 0).collect();
    top.sort_by(|a, b| b.unread.cmp(&a.unread).then_with(|| a.title.cmp(&b.title)));
    let latest = if headlines > 0 {
        let q = articles::ListQuery {
            view: View::Unread,
            limit: Some(headlines as u32),
            ..Default::default()
        };
        articles::list(pool, user.id, &q).await?.items
    } else {
        Vec::new()
    };

    let feeds_count = ov.feeds.len() as i64;
    let error_count = errors.len() as i64;
    Ok(json!({
        "app": "nrzNews",
        "version": env!("CARGO_PKG_VERSION"),
        "user": user.name(),
        "generatedAt": ts,
        "periodHours": hours,
        "counts": {
            "unread": ov.unread,
            "today": ov.today,
            "starred": ov.starred,
            "newInPeriod": new_in_period,
            "feeds": feeds_count,
            "folders": ov.folders.len(),
            "feedsWithErrors": error_count,
            "articles": articles_total,
        },
        "metrics": [
            metric("unread", "Ungelesen", ov.unread, "ok", "Ungelesene Artikel"),
            metric("today", "Heute", ov.today, "ok", "Ungelesen aus den letzten 24 Stunden"),
            metric("new", &format!("Neu ({hours} h)"), new_in_period, "ok", "Neu abgerufene Artikel im Zeitraum"),
            metric("starred", "Gemerkt", ov.starred, "ok", "Gemerkte Artikel"),
            metric("feeds", "Feeds", feeds_count, "ok", "Abonnierte Feeds"),
            metric("feed_errors", "Feed-Fehler", error_count, if error_count > 0 { "warning" } else { "ok" },
                   "Feeds, deren letzter Abruf fehlschlug"),
        ],
        "lastFetchedAt": last_fetched,
        "nextFetchAt": next_fetch,
        "topFeeds": top.iter().take(5).map(|f| json!({
            "id": f.id, "title": f.title, "unread": f.unread, "iconUrl": f.icon_url, "siteUrl": f.site_url,
        })).collect::<Vec<_>>(),
        "feedErrors": errors.iter().map(|f| json!({
            "id": f.id, "title": f.title, "error": f.last_error, "lastFetchedAt": f.last_fetched_at,
        })).collect::<Vec<_>>(),
        "latest": latest.iter().map(summary_json).collect::<Vec<_>>(),
    }))
}

async fn list_articles(pool: &SqlitePool, user: &User, args: &Map<String, Value>) -> ToolResult {
    let view = match str_arg(args, "view")?.unwrap_or("unread") {
        "unread" => View::Unread,
        "all" => View::All,
        "today" => View::Today,
        "starred" => View::Starred,
        other => {
            return Err(ToolError::Invalid(format!(
                "Unbekannte Ansicht „{other}“ (unread, all, today, starred)"
            )))
        }
    };
    let q = articles::ListQuery {
        view,
        feed: int_arg(args, "feedId", None, 1, i64::MAX)?,
        folder: int_arg(args, "folderId", None, 1, i64::MAX)?,
        q: str_arg(args, "query")?.map(str::to_string),
        cursor: str_arg(args, "cursor")?.map(str::to_string),
        limit: int_arg(args, "limit", Some(20), 1, 50)?.map(|n| n as u32),
    };
    let page = articles::list(pool, user.id, &q).await?;
    Ok(json!({
        "items": page.items.iter().map(summary_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

async fn get_article(pool: &SqlitePool, user: &User, args: &Map<String, Value>) -> ToolResult {
    let id = int_arg(args, "id", None, 1, i64::MAX)?
        .ok_or_else(|| ToolError::Invalid("„id“ fehlt".into()))?;
    let max = int_arg(args, "maxChars", Some(20_000), 500, 100_000)?.unwrap_or(20_000) as usize;
    let a = articles::get(pool, user.id, id).await?;
    let text: String =
        sqlx::query_scalar("SELECT content_text FROM articles WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user.id)
            .fetch_one(pool)
            .await?;
    let truncated = text.chars().count() > max;
    let text: String = if truncated {
        text.chars().take(max).collect()
    } else {
        text
    };
    Ok(json!({
        "id": a.id,
        "title": a.title,
        "feedId": a.feed_id,
        "feedTitle": a.feed_title,
        "url": a.url,
        "author": a.author,
        "publishedAt": a.published_at,
        "read": a.read,
        "starred": a.starred,
        "readingMinutes": a.reading_minutes,
        "aiSummary": a.ai_summary,
        "text": text,
        "truncated": truncated,
    }))
}

async fn list_feeds(pool: &SqlitePool, user: &User) -> ToolResult {
    let ov = feeds::overview(pool, user.id).await?;
    let folder_name = |id: Option<i64>| {
        id.and_then(|id| ov.folders.iter().find(|f| f.id == id))
            .map(|f| f.name.clone())
    };
    Ok(json!({
        "folders": ov.folders.iter().map(|f| json!({ "id": f.id, "name": f.name, "unread": f.unread })).collect::<Vec<_>>(),
        "feeds": ov.feeds.iter().map(|f| json!({
            "id": f.id,
            "title": f.title,
            "folderId": f.folder_id,
            "folder": folder_name(f.folder_id),
            "siteUrl": f.site_url,
            "feedUrl": f.feed_url,
            "unread": f.unread,
            "total": f.total,
            "lastFetchedAt": f.last_fetched_at,
            "error": f.last_error.as_deref().filter(|e| !e.trim().is_empty()),
        })).collect::<Vec<_>>(),
    }))
}

/// Ergebnis für Tests und andere Aufrufer ohne HTTP.
pub async fn call(pool: &SqlitePool, user: &User, name: &str, args: Value) -> AppResult<Value> {
    let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let res = handle(pool, user, msg).await.unwrap_or(Value::Null);
    if let Some(e) = res.get("error") {
        return Err(AppError::bad(e["message"].as_str().unwrap_or("MCP-Fehler")));
    }
    Ok(res["result"]["structuredContent"].clone())
}

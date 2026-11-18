//! Artikel über die API (`/api/v1/articles…`). Logik: [`crate::services::articles`].

use crate::auth::CurrentUser;
use crate::error::AppResult;
use crate::services::articles::{self, ListQuery};
use nrznews_types::{
    Article, ArticlePage, ArticleState, ArticleStateInput, MarkReadInput, MarkReadResult, View,
};
use rocket::serde::json::Json;
use rocket::{get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![list, get_one, set_state, mark_read]
}

/// Artikelliste: `view` (all, unread, starred, today), `feed`, `folder`, Suche `q`,
/// `cursor` aus `nextCursor` der vorigen Seite, `limit` (Vorgabe 40, höchstens 100).
#[allow(clippy::too_many_arguments)]
#[get("/articles?<view>&<feed>&<folder>&<q>&<cursor>&<limit>")]
async fn list(
    user: CurrentUser,
    view: Option<&str>,
    feed: Option<i64>,
    folder: Option<i64>,
    q: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<ArticlePage>> {
    let query = ListQuery {
        view: view.map(View::parse).unwrap_or_default(),
        feed,
        folder,
        q: q.map(String::from),
        cursor: cursor.map(String::from),
        limit,
    };
    Ok(Json(articles::list(pool, user.id(), &query).await?))
}

#[get("/articles/<id>")]
async fn get_one(user: CurrentUser, id: i64, pool: &State<SqlitePool>) -> AppResult<Json<Article>> {
    Ok(Json(articles::get(pool, user.id(), id).await?))
}

/// Gelesen/Gemerkt setzen (nur die mitgeschickten Felder).
#[put("/articles/<id>", data = "<body>")]
async fn set_state(
    user: CurrentUser,
    id: i64,
    body: Json<ArticleStateInput>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<ArticleState>> {
    Ok(Json(articles::set_state(pool, user.id(), id, &body).await?))
}

#[post("/articles/mark-read", data = "<body>")]
async fn mark_read(
    user: CurrentUser,
    body: Json<MarkReadInput>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<MarkReadResult>> {
    Ok(Json(articles::mark_read(pool, user.id(), &body).await?))
}

//! Feeds über die API (`/api/v1/feeds…`) und der Überblick für die Seitenleiste
//! (`/api/v1/overview`). Logik: [`crate::services::feeds`].

use crate::auth::CurrentUser;
use crate::config::Config;
use crate::error::AppResult;
use crate::feeds::fetch::Fetcher;
use crate::services::feeds;
use nrznews_types::{
    DiscoverInput, DiscoveredFeed, Feed, Overview, RefreshResult, SubscribeInput, UpdateFeedInput,
};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![
        overview,
        list,
        discover,
        subscribe,
        update,
        remove,
        refresh_one,
        refresh_all
    ]
}

/// Ordner, Feeds und Zähler für die Seitenleiste in einer Anfrage.
#[get("/overview")]
async fn overview(user: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Overview>> {
    Ok(Json(feeds::overview(pool, user.id()).await?))
}

#[get("/feeds")]
async fn list(user: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<Feed>>> {
    Ok(Json(feeds::list(pool, user.id()).await?))
}

/// Feeds zu einer Adresse finden (Webseite oder Feed), ohne zu abonnieren.
#[post("/feeds/discover", data = "<body>")]
async fn discover(
    _user: CurrentUser,
    body: Json<DiscoverInput>,
    fetcher: &State<Fetcher>,
) -> AppResult<Json<Vec<DiscoveredFeed>>> {
    Ok(Json(feeds::discover(fetcher, &body.url).await?))
}

/// Abonnieren (auch über die Adresse einer Webseite); der Feed wird sofort abgerufen.
#[post("/feeds", data = "<body>")]
async fn subscribe(
    user: CurrentUser,
    body: Json<SubscribeInput>,
    pool: &State<SqlitePool>,
    fetcher: &State<Fetcher>,
    config: &State<Config>,
) -> AppResult<(Status, Json<Feed>)> {
    let feed = feeds::subscribe(pool, fetcher, config, user.id(), &body).await?;
    Ok((Status::Created, Json(feed)))
}

#[put("/feeds/<id>", data = "<body>")]
async fn update(
    user: CurrentUser,
    id: i64,
    body: Json<UpdateFeedInput>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Feed>> {
    Ok(Json(feeds::update(pool, user.id(), id, &body).await?))
}

/// Abo beenden – mitsamt allen Artikeln des Feeds.
#[delete("/feeds/<id>")]
async fn remove(user: CurrentUser, id: i64, pool: &State<SqlitePool>) -> AppResult<Status> {
    feeds::delete(pool, user.id(), id).await?;
    Ok(Status::NoContent)
}

#[post("/feeds/<id>/refresh")]
async fn refresh_one(
    user: CurrentUser,
    id: i64,
    pool: &State<SqlitePool>,
    fetcher: &State<Fetcher>,
    config: &State<Config>,
) -> AppResult<Json<Feed>> {
    Ok(Json(
        feeds::refresh(pool, fetcher, config, user.id(), id).await?,
    ))
}

/// Alle eigenen Feeds jetzt abrufen.
#[post("/feeds/refresh")]
async fn refresh_all(
    user: CurrentUser,
    pool: &State<SqlitePool>,
    fetcher: &State<Fetcher>,
    config: &State<Config>,
) -> AppResult<Json<RefreshResult>> {
    Ok(Json(
        feeds::refresh_all(pool, fetcher, config, user.id()).await?,
    ))
}

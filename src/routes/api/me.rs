//! Eigenes Konto: Daten und persönliche Einstellungen, Passwort ändern, API-Tokens (MCP).

use crate::auth::{session, CurrentUser};
use crate::error::AppResult;
use crate::services::{tokens, users};
use nrznews_types::{
    ApiToken, ApiTokenCreated, ApiTokenInput, ChangePasswordInput, UpdateMeInput, User,
};
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::{delete, get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![
        me,
        update_me,
        change_password,
        list_tokens,
        create_token,
        delete_token
    ]
}

#[get("/me")]
fn me(user: CurrentUser) -> Json<User> {
    Json(user.0)
}

/// Anzeigename und Einstellungen (Sprache, Lesemodus, …); nur gesetzte Felder.
#[put("/me", data = "<body>")]
async fn update_me(
    user: CurrentUser,
    body: Json<UpdateMeInput>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<User>> {
    Ok(Json(users::update_me(pool, user.id(), &body).await?))
}

/// Passwort ändern (bisheriges nötig). Alle anderen Sitzungen enden – wer das Passwort
/// eines gestohlenen Geräts ändert, wirft es damit hinaus; dieses Gerät bleibt angemeldet.
#[put("/me/password", data = "<body>")]
async fn change_password(
    user: CurrentUser,
    body: Json<ChangePasswordInput>,
    cookies: &CookieJar<'_>,
    pool: &State<SqlitePool>,
) -> AppResult<Status> {
    users::change_password(pool, user.id(), &body.current, &body.new).await?;
    match session::token_from(cookies) {
        Some(token) => session::destroy_others(pool, user.id(), &token).await?,
        None => session::destroy_all(pool, user.id()).await?,
    };
    tracing::info!(user = %user.0.username, "Passwort geändert");
    Ok(Status::NoContent)
}

/// Eigene API-Tokens (ohne das Token selbst).
#[get("/me/tokens")]
async fn list_tokens(
    user: CurrentUser,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Vec<ApiToken>>> {
    Ok(Json(tokens::list(pool, user.id()).await?))
}

/// Neues Token – das Token steht nur in dieser Antwort.
#[post("/me/tokens", data = "<body>")]
async fn create_token(
    user: CurrentUser,
    body: Json<ApiTokenInput>,
    pool: &State<SqlitePool>,
) -> AppResult<(Status, Json<ApiTokenCreated>)> {
    let created = tokens::create(pool, user.id(), &body.name).await?;
    tracing::info!(user = %user.0.username, name = %created.token.name, "API-Token angelegt");
    Ok((Status::Created, Json(created)))
}

#[delete("/me/tokens/<id>")]
async fn delete_token(user: CurrentUser, id: i64, pool: &State<SqlitePool>) -> AppResult<Status> {
    tokens::delete(pool, user.id(), id).await?;
    Ok(Status::NoContent)
}

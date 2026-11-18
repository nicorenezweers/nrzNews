//! Ordner über die API (`/api/v1/folders…`). Logik: [`crate::services::folders`].

use crate::auth::CurrentUser;
use crate::error::AppResult;
use crate::services::folders;
use nrznews_types::{Folder, FolderInput};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![list, create, rename, remove]
}

#[get("/folders")]
async fn list(user: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<Folder>>> {
    Ok(Json(folders::list(pool, user.id()).await?))
}

#[post("/folders", data = "<body>")]
async fn create(
    user: CurrentUser,
    body: Json<FolderInput>,
    pool: &State<SqlitePool>,
) -> AppResult<(Status, Json<Folder>)> {
    let folder = folders::create(pool, user.id(), &body.name).await?;
    Ok((Status::Created, Json(folder)))
}

#[put("/folders/<id>", data = "<body>")]
async fn rename(
    user: CurrentUser,
    id: i64,
    body: Json<FolderInput>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Folder>> {
    Ok(Json(
        folders::rename(pool, user.id(), id, &body.name).await?,
    ))
}

/// Ordner löschen – seine Feeds bleiben und stehen danach „ohne Ordner“.
#[delete("/folders/<id>")]
async fn remove(user: CurrentUser, id: i64, pool: &State<SqlitePool>) -> AppResult<Status> {
    folders::delete(pool, user.id(), id).await?;
    Ok(Status::NoContent)
}

//! OPML über die API: `GET /api/v1/opml` lädt die Abos herunter, `POST /api/v1/opml` liest
//! eine OPML-Datei (als Text im Körper) ein. Logik: [`crate::services::feeds`].

use crate::auth::CurrentUser;
use crate::error::AppResult;
use crate::services::feeds;
use nrznews_types::OpmlImportResult;
use rocket::http::{ContentType, Header};
use rocket::serde::json::Json;
use rocket::{get, post, routes, Responder, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![export, import]
}

/// Datei zum Herunterladen (`Content-Disposition: attachment`).
#[derive(Responder)]
struct Download {
    body: (ContentType, String),
    disposition: Header<'static>,
}

#[get("/opml")]
async fn export(user: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Download> {
    let xml = feeds::export_opml(pool, user.id(), user.0.name()).await?;
    Ok(Download {
        body: (ContentType::new("text", "x-opml"), xml),
        disposition: Header::new(
            "Content-Disposition",
            "attachment; filename=\"nrznews.opml\"",
        ),
    })
}

/// OPML einlesen (bis 5 MiB, Grenze `string` in `Rocket.toml`). Abgerufen wird im Hintergrund.
#[post("/opml", data = "<body>")]
async fn import(
    user: CurrentUser,
    body: String,
    pool: &State<SqlitePool>,
) -> AppResult<Json<OpmlImportResult>> {
    Ok(Json(feeds::import_opml(pool, user.id(), &body).await?))
}

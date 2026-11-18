//! KI über die API: Einstellungen (nur Admins; Schlüssel verlassen den Server nie),
//! Zusammenfassung eines Artikels, Überblick über neue Artikel und Dieter, der Assistent.
//! Logik: [`crate::ai`] und [`crate::services::assistant`].

use crate::ai::{self, AiConfig};
use crate::auth::{AdminUser, CurrentUser};
use crate::config::Config;
use crate::error::AppResult;
use crate::services::assistant;
use nrznews_types::{
    AiModels, AiSettingsInput, AiSettingsView, AiSummary, AssistantAnswer, AssistantInput,
    Briefing, BriefingInput,
};
use rocket::serde::json::Json;
use rocket::{get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![get_settings, put_settings, models, summary, briefing, ask]
}

#[get("/settings/ai")]
async fn get_settings(
    _admin: AdminUser,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AiSettingsView>> {
    Ok(Json(ai::settings_view(pool, config).await?))
}

#[put("/settings/ai", data = "<body>")]
async fn put_settings(
    _admin: AdminUser,
    body: Json<AiSettingsInput>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AiSettingsView>> {
    Ok(Json(ai::save_settings(pool, config, &body).await?))
}

/// Modelle des gewählten Anbieters – oder eines anderen (`?provider=`) für die Auswahl.
#[get("/settings/ai/models?<provider>")]
async fn models(
    _admin: AdminUser,
    provider: Option<&str>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AiModels>> {
    Ok(Json(ai::list_models(pool, config, provider).await?))
}

/// Zusammenfassung eines Artikels (gespeichert; `?refresh=true` erzeugt eine neue).
#[post("/articles/<id>/summary?<refresh>")]
async fn summary(
    user: CurrentUser,
    id: i64,
    refresh: Option<bool>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AiSummary>> {
    let ai = AiConfig::load(pool, config).await;
    Ok(Json(
        assistant::summary(pool, &ai, user.id(), id, refresh.unwrap_or(false)).await?,
    ))
}

/// KI-Überblick über die neuen Artikel.
#[post("/briefing", data = "<body>")]
async fn briefing(
    user: CurrentUser,
    body: Json<BriefingInput>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<Briefing>> {
    let ai = AiConfig::load(pool, config).await;
    let language = user.0.preferences.language.clone();
    Ok(Json(
        assistant::briefing(pool, &ai, user.id(), &language, &body).await?,
    ))
}

/// Dieter beantwortet Fragen zu den eigenen Artikeln – mit Quellen.
#[post("/assistant", data = "<body>")]
async fn ask(
    user: CurrentUser,
    body: Json<AssistantInput>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AssistantAnswer>> {
    let ai = AiConfig::load(pool, config).await;
    Ok(Json(
        assistant::ask(pool, &ai, user.id(), &body.messages).await?,
    ))
}

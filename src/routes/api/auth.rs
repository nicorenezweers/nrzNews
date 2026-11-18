//! Anmeldung: Status für die Oberfläche, Ersteinrichtung bzw. Registrierung, Login, Logout.
//! Login und Registrierung prüfen die Herkunft der Anfrage ([`SameOrigin`]) – sonst könnte
//! eine fremde Seite den Browser unbemerkt in ein Konto des Angreifers einloggen.

use crate::auth::guards::{ClientIp, MaybeUser, SameOrigin, UserAgent};
use crate::auth::session;
use crate::auth::throttle::LoginThrottle;
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::services::users;
use nrznews_types::{AuthStatus, LoginInput, RegisterInput, Role, User};
use rocket::http::{CookieJar, Status};
use rocket::serde::json::Json;
use rocket::{get, post, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![status, register, login, logout]
}

/// Was die Oberfläche beim Start braucht: angemeldet? Ersteinrichtung? Registrierung offen?
#[get("/auth/status")]
async fn status(
    user: MaybeUser,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<AuthStatus>> {
    Ok(Json(AuthStatus {
        authenticated: user.0.is_some(),
        setup_required: users::count(pool).await? == 0,
        registration_open: users::registration_allowed(pool, config).await?,
        user: user.0,
        version: env!("CARGO_PKG_VERSION").into(),
    }))
}

/// Konto anlegen und gleich anmelden. Das erste Konto der Instanz wird Administrator.
#[post("/auth/register", data = "<body>")]
async fn register(
    _origin: SameOrigin,
    body: Json<RegisterInput>,
    cookies: &CookieJar<'_>,
    ua: UserAgent,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<(Status, Json<User>)> {
    if !users::registration_allowed(pool, config).await? {
        return Err(AppError::forbidden(
            "Die Registrierung ist geschlossen – ein Administrator muss dein Konto anlegen.",
        ));
    }
    let role = if users::count(pool).await? == 0 {
        Role::Admin
    } else {
        Role::Member
    };
    let user = users::create(
        pool,
        &body.username,
        &body.password,
        &body.display_name,
        role,
    )
    .await?;
    let token = session::create(pool, user.id, &ua.0).await?;
    session::set_cookie(cookies, token, config.cookie_secure);
    tracing::info!(user = %user.username, admin = user.is_admin(), "Konto angelegt");
    let user = users::record_login(pool, user.id).await?;
    Ok((Status::Created, Json(user)))
}

/// Anmelden. Nach zu vielen Fehlversuchen (je Name und IP) gibt es eine Pause (429).
#[allow(clippy::too_many_arguments)]
#[post("/auth/login", data = "<body>")]
async fn login(
    _origin: SameOrigin,
    body: Json<LoginInput>,
    cookies: &CookieJar<'_>,
    ua: UserAgent,
    ip: ClientIp,
    throttle: &State<LoginThrottle>,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Json<User>> {
    throttle.check(&body.username, &ip.0)?;
    match users::authenticate(pool, &body.username, &body.password).await {
        Ok(user) => {
            throttle.succeeded(&body.username, &ip.0);
            let token = session::create(pool, user.id, &ua.0).await?;
            session::set_cookie(cookies, token, config.cookie_secure);
            tracing::info!(user = %user.username, "Anmeldung");
            Ok(Json(users::record_login(pool, user.id).await?))
        }
        Err(AppError::InvalidCredentials) => {
            throttle.failed(&body.username, &ip.0);
            let name: String = body.username.chars().take(40).collect();
            tracing::warn!(username = %name, ip = %ip.0, "Anmeldung fehlgeschlagen");
            Err(AppError::InvalidCredentials)
        }
        Err(e) => Err(e),
    }
}

/// Abmelden: Sitzung in der Datenbank löschen und Cookie entfernen.
#[post("/auth/logout")]
async fn logout(
    _origin: SameOrigin,
    cookies: &CookieJar<'_>,
    pool: &State<SqlitePool>,
) -> AppResult<Status> {
    if let Some(token) = session::token_from(cookies) {
        session::destroy(pool, &token).await?;
    }
    session::clear_cookie(cookies);
    Ok(Status::NoContent)
}

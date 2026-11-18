//! Request Guards. Routen verlangen damit eine Anmeldung:
//!
//! * [`CurrentUser`] – angemeldet über das Sitzungs-Cookie.
//! * [`AdminUser`] – zusätzlich Administrator der Instanz.
//! * [`MaybeUser`] – angemeldet oder nicht (z. B. für `/auth/status`).
//! * [`SameOrigin`] – nur die Herkunftsprüfung, für Anmeldung und Registrierung.
//!
//! Schutz gegen CSRF: Das Cookie ist `SameSite=Lax` (fremde Seiten können kein POST mit
//! Cookie auslösen). Zusätzlich werden ändernde Anfragen abgewiesen, deren `Origin` nicht
//! zum eigenen Host passt.
//!
//! Lehnt ein Guard ab, merkt er sich die Meldung im Anfrage-Cache ([`GuardFailure`]); der
//! Catcher zeigt sie dann statt einer allgemeinen („Nur für Administratoren.“ statt „Kein Zugriff.“).

use crate::auth::session;
use crate::config::Config;
use crate::error::AppError;
use nrznews_types::User;
use rocket::http::{Method, Status};
use rocket::request::{FromRequest, Outcome, Request};
use sqlx::SqlitePool;

/// Benutzer der Sitzung als eigener Typ im Anfrage-Cache (der Cache unterscheidet nach Typ).
#[derive(Clone)]
struct SessionUser(Option<User>);

/// Benutzer der Sitzung – je Anfrage nur einmal aus der Datenbank geladen. Wurde die Sitzung
/// dabei verlängert, bekommt auch das Cookie ein neues Ablaufdatum.
async fn session_user(req: &Request<'_>) -> Option<User> {
    req.local_cache_async(async {
        let Some(pool) = req.rocket().state::<SqlitePool>() else {
            return SessionUser(None);
        };
        let Some(token) = session::token_from(req.cookies()) else {
            return SessionUser(None);
        };
        match session::check(pool, &token).await {
            Ok(Some(checked)) => {
                if checked.extended {
                    let secure = req
                        .rocket()
                        .state::<Config>()
                        .is_some_and(|c| c.cookie_secure);
                    session::set_cookie(req.cookies(), token, secure);
                }
                SessionUser(Some(checked.user))
            }
            Ok(None) => {
                // Abgelaufen, abgemeldet oder gesperrt: das nutzlose Cookie entfernen (wirkt
                // nur bei erfolgreichen Antworten wie `/auth/status` – bei Fehlern verwirft
                // Rocket Cookie-Änderungen)
                session::clear_cookie(req.cookies());
                SessionUser(None)
            }
            Err(e) => {
                tracing::warn!("Sitzung prüfen: {e}");
                SessionUser(None)
            }
        }
    })
    .await
    .0
    .clone()
}

/// Meldung des Guards, der die Anfrage abgewiesen hat – für den Catcher.
#[derive(Clone, Debug)]
pub struct GuardFailure(pub Option<(Status, String)>);

/// Abweisen und die Meldung für den Catcher merken (der erste abweisende Guard gewinnt).
fn reject<T>(req: &Request<'_>, error: AppError) -> Outcome<T, AppError> {
    let status = error.status();
    req.local_cache(|| GuardFailure(Some((status, error.public_message()))));
    Outcome::Error((status, error))
}

/// Ändernde Anfrage von einer fremden Seite?
pub fn is_cross_origin(req: &Request<'_>) -> bool {
    if matches!(req.method(), Method::Get | Method::Head | Method::Options) {
        return false;
    }
    let Some(origin) = req.headers().get_one("Origin") else {
        return false;
    };
    let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or("");
    // `dx serve` (Entwicklung der Oberfläche) leitet von :8080 weiter und schreibt Host um
    if cfg!(debug_assertions) && origin_host == "localhost:8080" {
        return false;
    }
    let own = [
        req.headers().get_one("X-Forwarded-Host"),
        req.headers().get_one("Host"),
    ];
    !own.iter()
        .flatten()
        .any(|h| h.eq_ignore_ascii_case(origin_host))
}

fn cross_origin_error() -> AppError {
    AppError::forbidden("Anfrage von einer fremden Seite abgewiesen.")
}

/// Angemeldeter Benutzer.
pub struct CurrentUser(pub User);

impl CurrentUser {
    pub fn id(&self) -> i64 {
        self.0.id
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for CurrentUser {
    type Error = AppError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if is_cross_origin(req) {
            return reject(req, cross_origin_error());
        }
        match session_user(req).await {
            Some(user) => Outcome::Success(CurrentUser(user)),
            None => reject(req, AppError::Unauthorized),
        }
    }
}

/// Angemeldeter Administrator.
pub struct AdminUser(pub User);

impl AdminUser {
    pub fn id(&self) -> i64 {
        self.0.id
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminUser {
    type Error = AppError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match CurrentUser::from_request(req).await {
            Outcome::Success(CurrentUser(u)) if u.is_admin() => Outcome::Success(AdminUser(u)),
            Outcome::Success(_) => reject(req, AppError::forbidden("Nur für Administratoren.")),
            Outcome::Error(e) => Outcome::Error(e),
            Outcome::Forward(s) => Outcome::Forward(s),
        }
    }
}

/// Angemeldet, aber optional – für den Anmeldestatus.
pub struct MaybeUser(pub Option<User>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for MaybeUser {
    type Error = std::convert::Infallible;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(MaybeUser(session_user(req).await))
    }
}

/// Nur die Herkunftsprüfung (für Anfragen ohne Anmeldung wie Login und Registrierung –
/// sonst könnte eine fremde Seite den Browser in ihr eigenes Konto einloggen).
pub struct SameOrigin;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SameOrigin {
    type Error = AppError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if is_cross_origin(req) {
            return reject(req, cross_origin_error());
        }
        Outcome::Success(SameOrigin)
    }
}

/// Browser-Kennung für die Sitzung.
pub struct UserAgent(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for UserAgent {
    type Error = std::convert::Infallible;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(UserAgent(
            req.headers()
                .get_one("User-Agent")
                .unwrap_or_default()
                .to_string(),
        ))
    }
}

/// IP-Adresse des Aufrufers (hinter einem Proxy aus `X-Real-IP`, siehe Rockets `ip_header`)
/// – für die Bremse gegen Passwort-Raten.
pub struct ClientIp(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ClientIp {
    type Error = std::convert::Infallible;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(ClientIp(
            req.client_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "unbekannt".into()),
        ))
    }
}

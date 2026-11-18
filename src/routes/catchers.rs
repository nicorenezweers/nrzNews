//! Fehler, die vor einem Handler entstehen (Guard abgewiesen, Route fehlt, JSON kaputt).
//! Die API bekommt JSON ([`nrznews_types::ApiError`]), alles andere eine kleine HTML-Seite.
//! Hat ein Guard eine eigene Meldung hinterlassen („Nur für Administratoren.“), steht sie
//! in der Antwort, sonst eine allgemeine zum Status.

use crate::auth::guards::GuardFailure;
use crate::error::error_response;
use rocket::http::Status;
use rocket::response::{self, Responder};
use rocket::{catch, catchers, Catcher, Request};

pub fn all() -> Vec<Catcher> {
    catchers![default]
}

pub struct Reply(Status, String);

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        error_response(req, self.0, &self.1)
    }
}

fn message(status: Status) -> &'static str {
    match status.code {
        400 => "Die Anfrage ist ungültig.",
        401 => "Bitte anmelden.",
        403 => "Kein Zugriff.",
        404 => "Nicht gefunden.",
        405 => "Diese Methode ist hier nicht erlaubt.",
        413 => "Die Anfrage ist zu gross.",
        415 => "Dieses Format wird hier nicht angenommen.",
        422 => "Die Eingaben sind unvollständig oder ungültig.",
        429 => "Zu viele Anfragen – bitte später erneut versuchen.",
        503 => "Der Dienst ist gerade nicht verfügbar.",
        _ => "Interner Fehler – Details stehen im Server-Protokoll.",
    }
}

#[catch(default)]
fn default(status: Status, req: &Request<'_>) -> Reply {
    let text = match req.local_cache(|| GuardFailure(None)) {
        GuardFailure(Some((s, text))) if *s == status => text.clone(),
        _ => message(status).to_string(),
    };
    Reply(status, text)
}

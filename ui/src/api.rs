//! Zugriff auf die JSON-API unter `/api/v1`. Anmeldung läuft über das Sitzungs-Cookie
//! (gleiche Herkunft); Fehler kommen als [`nrznews_types::ApiError`] (`{ "error": { code, message } }`).

use gloo_net::http::{Request, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const BASE: &str = "/api/v1";

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    /// Maschinenlesbarer Code des Servers (`bad_request`, `conflict`, …), leer bei Netzfehlern.
    pub code: String,
    /// Meldung, schon in die gewählte Sprache übersetzt.
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

fn network(e: impl std::fmt::Display) -> ApiError {
    ApiError { status: 0, code: String::new(), message: crate::i18n::trf("Keine Verbindung zum Server ({0}).", &[&e.to_string()]) }
}

async fn finish<T: DeserializeOwned>(res: Response) -> ApiResult<T> {
    let status = res.status();
    let fail = |message: String| ApiError { status, code: String::new(), message };
    if status == 204 {
        return serde_json::from_value(Value::Null).map_err(|e| fail(e.to_string()));
    }
    let text = res.text().await.map_err(network)?;
    if (200..300).contains(&status) {
        let value: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).map_err(|e| fail(e.to_string()))? };
        return serde_json::from_value(value).map_err(|e| fail(crate::i18n::trf("Unerwartete Antwort: {0}", &[&e.to_string()])));
    }
    let body = serde_json::from_str::<nrznews_types::ApiError>(&text).ok();
    let code = body.as_ref().map(|b| b.error.code.clone()).unwrap_or_default();
    let message = body
        // Meldungen des Servers sind deutsch – gibt es sie im Katalog, erscheinen sie übersetzt
        .map(|b| crate::i18n::tr(&b.error.message))
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| match status {
            401 => crate::i18n::tr("Bitte anmelden."),
            403 => crate::i18n::tr("Keine Berechtigung."),
            404 => crate::i18n::tr("Nicht gefunden."),
            _ => crate::i18n::trf("Fehler {0}", &[&status.to_string()]),
        });
    if status == 401 {
        crate::state::on_unauthorized();
    }
    Err(ApiError { status, code, message })
}

fn url(path: &str) -> String {
    if path.starts_with('/') && !path.starts_with(BASE) && !path.starts_with("/static") { format!("{BASE}{path}") } else { path.to_string() }
}

fn builder(method: &str, path: &str) -> RequestBuilder {
    let url = url(path);
    let b = match method {
        "POST" => Request::post(&url),
        "PUT" => Request::put(&url),
        "PATCH" => Request::patch(&url),
        "DELETE" => Request::delete(&url),
        _ => Request::get(&url),
    };
    b.header("Accept", "application/json")
}

pub async fn get<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    finish(builder("GET", path).send().await.map_err(network)?).await
}

pub async fn send<T: DeserializeOwned, B: Serialize>(method: &str, path: &str, body: &B) -> ApiResult<T> {
    let req = builder(method, path).json(body).map_err(network)?;
    finish(req.send().await.map_err(network)?).await
}

pub async fn post<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> ApiResult<T> {
    send("POST", path, body).await
}

pub async fn put<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> ApiResult<T> {
    send("PUT", path, body).await
}

/// POST ohne Inhalt (Aktionen wie „jetzt abrufen“).
pub async fn action<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    finish(builder("POST", path).send().await.map_err(network)?).await
}

pub async fn delete(path: &str) -> ApiResult<()> {
    let _: Value = finish(builder("DELETE", path).send().await.map_err(network)?).await?;
    Ok(())
}

/// POST mit reinem Text (OPML-Import: `Content-Type: text/xml`).
pub async fn post_text<T: DeserializeOwned>(path: &str, content_type: &str, body: String) -> ApiResult<T> {
    let req = builder("POST", path).header("Content-Type", content_type).body(body).map_err(network)?;
    finish(req.send().await.map_err(network)?).await
}

/// URL-Kodierung für Pfad- und Suchteile.
pub fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

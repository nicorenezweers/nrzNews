//! Abrufen über HTTP: ein gemeinsamer Client (Verbindungen und TLS-Sitzungen werden
//! wiederverwendet) für Feeds, Webseiten und Icons.
//!
//! Bedingtes Abrufen mit `If-None-Match`/`If-Modified-Since`: antwortet der Server mit
//! `304 Not Modified`, wird nichts geladen und nichts gelesen – das schont beide Seiten.
//! Antworten über 32 MiB werden abgebrochen, damit ein kaputter oder böswilliger Server den
//! Speicher nicht füllt. Nur `http` und `https`, höchstens 10 Weiterleitungen.

use crate::config::Config;
use reqwest::header::{self, HeaderMap, HeaderValue};
use std::time::Duration;

/// Grösste Antwort, die gelesen wird.
pub const MAX_BODY: usize = 32 * 1024 * 1024;

/// So meldet sich nrzNews bei den Servern.
pub const USER_AGENT: &str = concat!(
    "nrzNews/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/nicorenezweers/nrzNews)"
);

const ACCEPT: &str = "application/rss+xml, application/atom+xml, application/feed+json, \
     application/json;q=0.9, application/xml;q=0.9, text/xml;q=0.9, text/html;q=0.8, */*;q=0.5";

/// Gemeinsamer HTTP-Client (als Rocket-State verwaltet; `Clone` teilt den Verbindungspool).
#[derive(Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

/// Eine gelesene Antwort.
#[derive(Debug, Clone)]
pub struct Page {
    /// Adresse nach allen Weiterleitungen
    pub url: String,
    /// `Content-Type` ohne Parameter, klein geschrieben (leer = keiner)
    pub content_type: String,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Ergebnis eines (bedingten) Abrufs.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// `304 Not Modified`: seit dem letzten Abruf unverändert
    NotModified,
    Page(Page),
}

impl Fetcher {
    pub fn new(timeout_secs: u64) -> Self {
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("zu viele Weiterleitungen")
            } else if !matches!(attempt.url().scheme(), "http" | "https") {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(ACCEPT));
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .timeout(Duration::from_secs(timeout_secs.clamp(2, 300)))
            .connect_timeout(Duration::from_secs(timeout_secs.clamp(2, 30)))
            .redirect(redirects)
            .build()
            // Schlägt nur fehl, wenn das TLS-Backend fehlt – dann läuft gar nichts.
            .expect("HTTP-Client");
        Self { client }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::new(config.fetch_timeout_secs)
    }

    /// Lädt eine Adresse ohne Bedingungen.
    pub async fn get(&self, url: &str) -> Result<Page, String> {
        match self.fetch(url, None, None).await? {
            Fetched::Page(p) => Ok(p),
            // Ohne Bedingungen antwortet kein Server mit 304 – falls doch: wie ein Fehler
            Fetched::NotModified => Err("Der Server meldet „unverändert“ ohne Anfrage.".into()),
        }
    }

    /// Lädt eine Adresse; mit `etag`/`last_modified` vom letzten Abruf bedingt.
    pub async fn fetch(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<Fetched, String> {
        let parsed = url::Url::parse(url).map_err(|_| "Ungültige Adresse.".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("Nur Adressen mit http:// oder https:// gehen.".into());
        }
        let mut req = self.client.get(parsed);
        if let Some(e) = etag.filter(|e| !e.is_empty()) {
            req = req.header(header::IF_NONE_MATCH, e);
        }
        if let Some(m) = last_modified.filter(|m| !m.is_empty()) {
            req = req.header(header::IF_MODIFIED_SINCE, m);
        }
        let mut resp = req.send().await.map_err(describe)?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => format!("Zugriff verweigert ({}).", status.as_u16()),
                404 | 410 => format!("Nicht (mehr) vorhanden ({}).", status.as_u16()),
                429 => "Zu viele Anfragen – der Server bremst (429).".into(),
                code if code >= 500 => format!("Fehler beim Server ({code})."),
                code => format!("Unerwartete Antwort ({code})."),
            });
        }
        if resp.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err("Die Antwort ist zu gross (über 32 MiB).".into());
        }
        let text_header = |name: header::HeaderName| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let content_type = text_header(header::CONTENT_TYPE)
            .map(|c| {
                c.split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        let etag = text_header(header::ETAG);
        let last_modified = text_header(header::LAST_MODIFIED);
        let final_url = resp.url().to_string();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(describe)? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err("Die Antwort ist zu gross (über 32 MiB).".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Fetched::Page(Page {
            url: final_url,
            content_type,
            body,
            etag,
            last_modified,
        }))
    }

    /// Gibt es unter der Adresse ein Bild? (für das Favicon als letzte Möglichkeit)
    pub async fn is_image(&self, url: &str) -> bool {
        match self.get(url).await {
            Ok(p) => {
                !p.body.is_empty()
                    && (p.content_type.starts_with("image/")
                        || (p.content_type.is_empty()
                            || p.content_type == "application/octet-stream")
                            && url.ends_with(".ico"))
            }
            Err(_) => false,
        }
    }
}

/// Kurze deutsche Meldung zu einem Netzwerkfehler (für `feeds.last_error`).
fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Zeitüberschreitung – der Server antwortet nicht.".into()
    } else if e.is_redirect() {
        "Zu viele Weiterleitungen.".into()
    } else if e.is_connect() {
        "Keine Verbindung zum Server.".into()
    } else if e.is_body() || e.is_decode() {
        "Die Antwort kam unvollständig an.".into()
    } else {
        "Abruf fehlgeschlagen.".into()
    }
}

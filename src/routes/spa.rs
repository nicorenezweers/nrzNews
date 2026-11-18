//! Die Oberfläche: eine Dioxus-App (WebAssembly, Quelltext in `ui/`), beim Übersetzen
//! eingebettet (siehe build.rs). Ihre Dateien liegen unter `/assets/` (mit Prüfsumme im
//! Namen, darum für immer zwischenspeicherbar); jeder andere Pfad, der nicht zur API oder zu
//! `/static` gehört, bekommt `index.html` – den Rest erledigt der Router der App.

use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder, Response};
use rocket::{get, routes, Request, Route};
use std::io::Cursor;
use std::path::PathBuf;

pub struct SpaFile {
    pub path: &'static str,
    pub body: &'static [u8],
    /// gzip-Fassung (Text und WebAssembly), sonst `None`
    pub gzip: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/spa.rs"));

pub fn routes() -> Vec<Route> {
    routes![index, fallback]
}

/// Ist die Oberfläche eingebettet? (Ohne `scripts/ui.sh` vor dem Übersetzen nicht.)
pub fn available() -> bool {
    FILES.iter().any(|f| f.path == "index.html")
}

fn find(path: &str) -> Option<&'static SpaFile> {
    FILES.iter().find(|f| f.path == path)
}

/// Versteht der Browser gzip?
pub struct AcceptsGzip(pub bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptsGzip {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let gzip = req
            .headers()
            .get("Accept-Encoding")
            .any(|v| v.split(',').any(|e| e.trim().starts_with("gzip")));
        Outcome::Success(AcceptsGzip(gzip))
    }
}

/// Content-Type nach Dateiendung; ergänzt, was Rocket nicht kennt (Module, Manifest).
pub fn content_type(path: &str) -> ContentType {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "mjs" => ContentType::JavaScript,
        "webmanifest" => ContentType::new("application", "manifest+json"),
        _ => ContentType::from_extension(ext).unwrap_or(ContentType::Binary),
    }
}

/// Eingebettete Datei ausliefern – gzip-komprimiert, wenn der Browser das versteht.
pub fn embedded_response(
    body: &'static [u8],
    gzip: Option<&'static [u8]>,
    accepts_gzip: bool,
    content_type: ContentType,
    cache: &'static str,
) -> response::Result<'static> {
    let mut res = Response::build();
    res.header(content_type)
        .header(Header::new("Cache-Control", cache));
    if gzip.is_some() {
        res.header(Header::new("Vary", "Accept-Encoding"));
    }
    match (accepts_gzip, gzip) {
        (true, Some(gz)) => res
            .header(Header::new("Content-Encoding", "gzip"))
            .sized_body(gz.len(), Cursor::new(gz)),
        _ => res.sized_body(body.len(), Cursor::new(body)),
    };
    res.ok()
}

pub enum Reply {
    File(&'static SpaFile, bool),
    /// Oberfläche nicht eingebettet: Hinweis statt leerer Seite
    Missing,
}

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let (file, gzip) = match self {
            Reply::File(f, g) => (f, g),
            Reply::Missing => {
                let body = "<!doctype html><html lang=de><meta charset=utf-8>\
                    <meta name=viewport content=\"width=device-width,initial-scale=1\">\
                    <title>nrzNews</title>\
                    <body style=\"background:#171717;color:#e5e5e5;font-family:sans-serif;display:grid;place-items:center;min-height:90vh\">\
                    <main style=\"max-width:36rem;padding:1rem\"><h1>nrzNews</h1>\
                    <p>Die Oberfläche ist nicht eingebaut – vor dem Übersetzen \
                    <code>scripts/ui.sh</code> ausführen (baut <code>ui/</code> nach \
                    <code>ui/dist/public</code>) und den Server neu übersetzen.</p>\
                    <p>Die API unter <code>/api/v1</code> läuft.</p></main>";
                return Response::build()
                    .status(Status::ServiceUnavailable)
                    .header(ContentType::HTML)
                    .header(Header::new("Cache-Control", "no-cache"))
                    .sized_body(body.len(), Cursor::new(body))
                    .ok();
            }
        };
        // Dateien unter assets/ tragen eine Prüfsumme im Namen; alles andere (vor allem
        // index.html) immer neu prüfen, damit ein Update sofort ankommt.
        let cache = if file.path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        embedded_response(file.body, file.gzip, gzip, content_type(file.path), cache)
    }
}

fn page(gzip: AcceptsGzip) -> Reply {
    find("index.html").map_or(Reply::Missing, |f| Reply::File(f, gzip.0))
}

#[get("/")]
fn index(gzip: AcceptsGzip) -> Reply {
    page(gzip)
}

/// Dateien der App oder – für alle Seiten der App – index.html. Niedriger Rang, damit API
/// und `/static` zuerst greifen; unbekannte API-Pfade bleiben 404 (JSON).
#[get("/<path..>", rank = 50)]
fn fallback(path: PathBuf, gzip: AcceptsGzip) -> Option<Reply> {
    let path = path.to_str()?.replace('\\', "/");
    if path == "api" || path.starts_with("api/") {
        return None;
    }
    if let Some(f) = find(&path) {
        return Some(Reply::File(f, gzip.0));
    }
    // Fehlende Datei (z. B. alte Prüfsumme nach einem Update) nicht als Seite beantworten –
    // sonst bekäme der Browser HTML statt Skript.
    let file_like = [".js", ".mjs", ".css", ".wasm", ".map"]
        .iter()
        .any(|e| path.ends_with(e));
    if path.starts_with("assets/")
        || path.starts_with("wasm/")
        || path == "static"
        || path.starts_with("static/")
        || file_like
    {
        return None;
    }
    Some(page(gzip))
}

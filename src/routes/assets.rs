//! Statische Dateien, ins Programm eingebettet (siehe build.rs) – nrzNews bleibt ein
//! einzelnes Programm und braucht kein CDN:
//!
//! * `/static/fonts/…` – die Schrift (Inter), für immer zwischenspeicherbar
//! * `/static/icon.png`, `/static/favicon-32.png` (und weitere Icons aus `static/`)
//! * `/static/locales/<sprache>.json` – Übersetzungen der Oberfläche (deutsch → Sprache);
//!   die Oberfläche hängt `?v=<version>` an, ein Update lädt also neu
//! * `/favicon.ico` – Browser fragen ohne Link danach
//! * `/robots.txt` – ein persönlicher Reader gehört nicht in Suchmaschinen

use crate::routes::spa::{content_type, embedded_response, AcceptsGzip};
use rocket::http::ContentType;
use rocket::response::{self, Responder};
use rocket::{get, routes, Request, Route};

pub fn routes() -> Vec<Route> {
    routes![asset, favicon, robots]
}

/// Eine eingebettete Datei (Pfad relativ zu `static/` bzw. `locales/…`).
pub struct Embedded {
    pub path: &'static str,
    pub body: &'static [u8],
    pub gzip: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/locales.rs"));
include!(concat!(env!("OUT_DIR"), "/static_files.rs"));

/// Sprachen, für die eine Übersetzung eingebettet ist (ausser Deutsch, der Quellsprache).
pub fn locale_codes() -> Vec<&'static str> {
    LOCALES
        .iter()
        .filter_map(|l| l.path.strip_prefix("locales/")?.strip_suffix(".json"))
        .collect()
}

pub struct Asset {
    file: &'static Embedded,
    gzip: bool,
}

impl<'r> Responder<'r, 'static> for Asset {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        // Schriften ändern sich nie; Icons und Übersetzungen eine Stunde
        let cache = if self.file.path.starts_with("fonts/") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        };
        embedded_response(
            self.file.body,
            self.file.gzip,
            self.gzip,
            content_type(self.file.path),
            cache,
        )
    }
}

fn lookup(path: &str) -> Option<&'static Embedded> {
    STATIC_FILES
        .iter()
        .chain(LOCALES.iter())
        .find(|f| f.path == path)
}

#[get("/static/<path..>")]
fn asset(path: std::path::PathBuf, gzip: AcceptsGzip) -> Option<Asset> {
    let path = path.to_str()?.replace('\\', "/");
    lookup(&path).map(|file| Asset { file, gzip: gzip.0 })
}

/// Dasselbe Icon wie im Tab und auf Umbrel.
#[get("/favicon.ico")]
fn favicon(gzip: AcceptsGzip) -> Option<Asset> {
    lookup("favicon-32.png")
        .or_else(|| lookup("icon.png"))
        .map(|file| Asset { file, gzip: gzip.0 })
}

#[get("/robots.txt")]
fn robots() -> (ContentType, &'static str) {
    (ContentType::Plain, "User-agent: *\nDisallow: /\n")
}

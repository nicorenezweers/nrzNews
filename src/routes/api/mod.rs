//! REST-API unter `/api/v1`. Typen der Körper und Antworten: Crate `nrznews-types`.

mod admin;
mod ai;
mod articles;
mod auth;
mod feeds;
mod folders;
mod me;
mod opml;

use rocket::serde::json::{json, Value};
use rocket::{get, routes, Route};

pub fn routes() -> Vec<Route> {
    let mut r = routes![health];
    r.extend(admin::routes());
    r.extend(ai::routes());
    r.extend(articles::routes());
    r.extend(auth::routes());
    r.extend(feeds::routes());
    r.extend(folders::routes());
    r.extend(me::routes());
    r.extend(opml::routes());
    r
}

/// Lebenszeichen für Docker-Healthcheck und Monitoring.
#[get("/health")]
pub fn health() -> Value {
    json!({ "status": "ok", "name": "nrzNews", "version": env!("CARGO_PKG_VERSION") })
}

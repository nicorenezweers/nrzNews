//! nrzNews – selbst gehosteter RSS-Reader. Aufbau: siehe ARCHITECTURE.md.

pub mod ai;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod fairings;
pub mod feeds;
pub mod mcp;
pub mod routes;
pub mod services;

use rocket::figment::Figment;
use rocket::{Build, Rocket};

/// Baut die Rocket-Instanz mit der normalen Konfiguration (`Rocket.toml`, `NRZNEWS_*`).
pub async fn build() -> Result<Rocket<Build>, error::AppError> {
    build_with(config::figment()).await
}

/// Baut die Rocket-Instanz aus einer gegebenen Konfiguration (auch für Tests):
/// Datenbank öffnen und migrieren, Cookie-Schlüssel laden, Fairings, Routen, Catcher.
/// Mit `background = false` (Tests) startet nichts im Hintergrund.
pub async fn build_with(figment: Figment) -> Result<Rocket<Build>, error::AppError> {
    let config: config::Config = figment
        .extract()
        .map_err(|e| error::AppError::Config(e.to_string()))?;
    let pool = db::connect(&config).await?;
    // Ein selbst gesetzter secret_key hat Vorrang; sonst der aus der Datenbank.
    let figment = figment.join((
        "secret_key",
        services::settings::cookie_secret(&pool).await?,
    ));

    Ok(rocket::custom(figment)
        .manage(pool)
        // Ein HTTP-Client für alle Abrufe (Verbindungspool, Zeitlimit aus der Konfiguration)
        .manage(feeds::fetch::Fetcher::from_config(&config))
        .manage(config)
        .manage(auth::throttle::LoginThrottle::default())
        .attach(fairings::RequestLog)
        // Rockets Shield ohne eigene Header: die setzt alle SecurityHeaders (sonst gewönne
        // Shields X-Frame-Options: SAMEORIGIN über unser DENY)
        .attach(rocket::shield::Shield::new())
        .attach(fairings::SecurityHeaders)
        .attach(fairings::SessionCleanup)
        .attach(fairings::CloseDatabase)
        .attach(feeds::refresher::Refresher)
        .mount("/", routes::assets::routes())
        .mount("/", routes::spa::routes())
        .mount("/api/v1", routes::api::routes())
        .mount("/", routes::mcp::routes())
        .register("/", routes::catchers::all()))
}

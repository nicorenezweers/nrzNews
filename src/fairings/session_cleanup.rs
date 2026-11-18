//! Startet beim Hochfahren (Liftoff) das Aufräumen: abgelaufene Sitzungen werden stündlich
//! gelöscht (die Anmeldung weist sie ohnehin ab – das hält nur die Tabelle klein). Endet mit
//! dem Server; läuft nur mit `background = true` (Tests schalten es aus).

use crate::auth::session;
use crate::config::Config;
use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Orbit, Rocket};
use sqlx::SqlitePool;
use std::time::Duration;

pub struct SessionCleanup;

#[rocket::async_trait]
impl Fairing for SessionCleanup {
    fn info(&self) -> Info {
        Info {
            name: "Sitzungen aufräumen",
            kind: Kind::Liftoff,
        }
    }

    async fn on_liftoff(&self, rocket: &Rocket<Orbit>) {
        let (Some(pool), Some(config)) = (
            rocket.state::<SqlitePool>().cloned(),
            rocket.state::<Config>(),
        ) else {
            return;
        };
        if !config.background {
            return;
        }
        let mut shutdown = rocket.shutdown();
        tokio::spawn(async move {
            loop {
                match session::purge_expired(&pool).await {
                    Ok(n) if n > 0 => tracing::info!("{n} abgelaufene Sitzungen gelöscht"),
                    Err(e) => tracing::warn!("Sitzungen aufräumen: {e}"),
                    _ => {}
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
                    _ = &mut shutdown => break,
                }
            }
        });
    }
}

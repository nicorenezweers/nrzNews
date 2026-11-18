//! Einstellungen. Quelle ist Rockets Figment: `Rocket.toml` und Umgebungsvariablen mit
//! dem Präfix `NRZNEWS_` (z. B. `NRZNEWS_DATA_DIR`, `NRZNEWS_AI_PROVIDER`).

use rocket::figment::providers::{Env, Format, Toml};
use rocket::figment::Figment;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Datenordner: hier liegt `nrznews.db`.
    pub data_dir: PathBuf,
    /// Neue Benutzer dürfen sich selbst registrieren, solange der Admin nichts anderes
    /// einstellt (das erste Konto immer).
    pub registration_open: bool,
    /// Cookies nur über HTTPS senden – einschalten, sobald nrzNews hinter HTTPS läuft.
    pub cookie_secure: bool,
    /// Hintergrundarbeiten (Feeds abrufen, Aufräumen) starten. Tests schalten sie aus.
    pub background: bool,
    /// Höchstens so viele Feeds gleichzeitig abrufen.
    pub fetch_concurrency: usize,
    /// Zeitlimit je Abruf in Sekunden.
    pub fetch_timeout_secs: u64,
    /// KI-Anbieter aus der Umgebung (z. B. "openai", "anthropic", "ollama"; leer = OpenRouter).
    /// In den Einstellungen der Oberfläche lässt sich jeder Anbieter wählen.
    pub ai_provider: String,
    /// Schlüssel für `ai_provider`
    pub ai_key: Option<String>,
    /// Eigene Adresse der API (leer = Vorgabe des Anbieters)
    pub ai_url: String,
    /// Modell (leer = Vorgabe des Anbieters)
    pub ai_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            registration_open: false,
            cookie_secure: false,
            background: true,
            fetch_concurrency: 8,
            fetch_timeout_secs: 20,
            ai_provider: String::new(),
            ai_key: None,
            ai_url: String::new(),
            ai_model: String::new(),
        }
    }
}

impl Config {
    /// `nrznews.db` im Datenordner. Eine Datenbank aus der Zeit vor der Umbenennung
    /// (`nrzrss.db`) wird weiterverwendet, solange es noch keine `nrznews.db` gibt.
    pub fn database_path(&self) -> PathBuf {
        let path = self.data_dir.join("nrznews.db");
        let legacy = self.data_dir.join("nrzrss.db");
        if !path.exists() && legacy.exists() {
            return legacy;
        }
        path
    }
}

/// Rocket-Konfiguration plus `NRZNEWS_*`-Umgebung (statt `ROCKET_*`).
pub fn figment() -> Figment {
    Figment::from(rocket::Config::default())
        .merge(Toml::file(Env::var_or("NRZNEWS_CONFIG", "Rocket.toml")).nested())
        // NRZNEWS_LOG ist der tracing-Filter (main.rs), keine Rocket-Einstellung
        .merge(
            Env::prefixed("NRZNEWS_")
                .ignore(&["PROFILE", "LOG"])
                .global(),
        )
        .select(rocket::figment::Profile::from_env_or(
            "NRZNEWS_PROFILE",
            rocket::Config::DEFAULT_PROFILE,
        ))
}

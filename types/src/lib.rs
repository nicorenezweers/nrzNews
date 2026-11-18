//! nrzNews – Typen der JSON-API unter `/api/v1`. Server (Rocket) und Oberfläche (Dioxus,
//! WebAssembly) benutzen dieselben Structs, damit beide Seiten nie auseinanderlaufen.
//!
//! Konventionen:
//! * JSON-Felder in camelCase, Zeitpunkte als Unix-Sekunden (UTC, `i64`), IDs als `i64`.
//! * Anmeldung per Sitzungs-Cookie (`nrznews_session`, HttpOnly, SameSite=Lax).
//! * Fehler: HTTP-Status plus [`ApiError`] als Körper.
//!
//! Endpunkte (alle unter `/api/v1`):
//!
//! | Methode & Pfad | Körper → Antwort | Wer |
//! |---|---|---|
//! | `GET /health` | → `{status, name, version}` | alle |
//! | `GET /auth/status` | → [`AuthStatus`] | alle |
//! | `POST /auth/register` | [`RegisterInput`] → [`User`] (setzt Cookie; erstes Konto = Admin) | alle, solange Registrierung offen oder noch kein Konto |
//! | `POST /auth/login` | [`LoginInput`] → [`User`] (setzt Cookie) | alle |
//! | `POST /auth/logout` | → `204` | angemeldet |
//! | `GET /me` | → [`User`] | angemeldet |
//! | `PUT /me` | [`UpdateMeInput`] → [`User`] | angemeldet |
//! | `PUT /me/password` | [`ChangePasswordInput`] → `204` | angemeldet |
//! | `GET /admin/users` | → `Vec<`[`User`]`>` | Admin |
//! | `POST /admin/users` | [`CreateUserInput`] → [`User`] | Admin |
//! | `PUT /admin/users/<id>` | [`UpdateUserInput`] → [`User`] | Admin |
//! | `DELETE /admin/users/<id>` | → `204` | Admin (nicht sich selbst, nicht den letzten Admin) |
//! | `GET /settings/instance` | → [`InstanceSettings`] | Admin |
//! | `PUT /settings/instance` | [`InstanceSettings`] → [`InstanceSettings`] | Admin |
//! | `GET /settings/ai` | → [`AiSettingsView`] | Admin |
//! | `PUT /settings/ai` | [`AiSettingsInput`] → [`AiSettingsView`] | Admin |
//! | `GET /settings/ai/models?provider=` | → [`AiModels`] | Admin |
//! | `GET /overview` | → [`Overview`] (Ordner, Feeds, Zähler für die Seitenleiste) | angemeldet |
//! | `GET /folders` | → `Vec<`[`Folder`]`>` | angemeldet |
//! | `POST /folders` | [`FolderInput`] → [`Folder`] | angemeldet |
//! | `PUT /folders/<id>` | [`FolderInput`] → [`Folder`] | angemeldet |
//! | `DELETE /folders/<id>` | → `204` (Feeds wandern nach „ohne Ordner“) | angemeldet |
//! | `GET /feeds` | → `Vec<`[`Feed`]`>` | angemeldet |
//! | `POST /feeds/discover` | [`DiscoverInput`] → `Vec<`[`DiscoveredFeed`]`>` | angemeldet |
//! | `POST /feeds` | [`SubscribeInput`] → [`Feed`] (sofort abgerufen) | angemeldet |
//! | `PUT /feeds/<id>` | [`UpdateFeedInput`] → [`Feed`] | angemeldet |
//! | `DELETE /feeds/<id>` | → `204` (mit allen Artikeln) | angemeldet |
//! | `POST /feeds/<id>/refresh` | → [`Feed`] | angemeldet |
//! | `POST /feeds/refresh` | → [`RefreshResult`] (alle eigenen Feeds) | angemeldet |
//! | `GET /articles?view=&feed=&folder=&q=&cursor=&limit=` | → [`ArticlePage`] | angemeldet |
//! | `GET /articles/<id>` | → [`Article`] | angemeldet |
//! | `PUT /articles/<id>` | [`ArticleStateInput`] → [`ArticleState`] | angemeldet |
//! | `POST /articles/mark-read` | [`MarkReadInput`] → [`MarkReadResult`] | angemeldet |
//! | `POST /articles/<id>/summary` | → [`AiSummary`] (KI, wird gespeichert) | angemeldet |
//! | `POST /briefing` | [`BriefingInput`] → [`Briefing`] (KI-Überblick über neue Artikel) | angemeldet |
//! | `POST /assistant` | [`AssistantInput`] → [`AssistantAnswer`] (Dieter) | angemeldet |
//! | `GET /opml` | → OPML-Datei (`text/x-opml`, Download) | angemeldet |
//! | `POST /opml` | OPML als Text (`Content-Type: text/xml` o. ä.) → [`OpmlImportResult`] | angemeldet |

use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------ Fehler

/// Fehlerkörper jeder Antwort mit Status ≥ 400.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Maschinenlesbar: `bad_request`, `unauthorized`, `forbidden`, `not_found`, `conflict`, …
    pub code: String,
    /// Deutsche Meldung für den Benutzer (die Oberfläche übersetzt sie über den Katalog).
    pub message: String,
}

// ------------------------------------------------------------------ Konten

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    #[default]
    Member,
}

/// Persönliche Einstellungen, gespeichert beim Benutzer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// Sprache der Oberfläche: "de" oder "en".
    pub language: String,
    /// Listen zeigen anfangs nur Ungelesenes.
    pub unread_only: bool,
    /// Artikel beim Öffnen als gelesen markieren.
    pub mark_read_on_open: bool,
    /// Schriftgrösse im Lesemodus: "s", "m" oder "l".
    pub reader_size: String,
    /// Liste kompakt (nur Titel) statt mit Auszug und Bild.
    pub compact_list: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "de".into(),
            unread_only: true,
            mark_read_on_open: true,
            reader_size: "m".into(),
            compact_list: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub username: String,
    /// Anzeigename; leer = Benutzername anzeigen.
    pub display_name: String,
    pub role: Role,
    pub disabled: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub preferences: Preferences,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn name(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub authenticated: bool,
    /// Noch kein Konto: die Oberfläche zeigt die Ersteinrichtung (erstes Konto = Admin).
    pub setup_required: bool,
    /// Selbst registrieren erlaubt (Einstellung des Admins).
    pub registration_open: bool,
    pub user: Option<User>,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterInput {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateMeInput {
    pub display_name: Option<String>,
    pub preferences: Option<Preferences>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordInput {
    pub current: String,
    pub new: String,
}

/// Mindestlänge für Passwörter (Server prüft, Oberfläche zeigt sie an).
pub const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserInput {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub role: Role,
}

/// Nur gesetzte Felder werden geändert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateUserInput {
    pub display_name: Option<String>,
    pub role: Option<Role>,
    /// Neues Passwort setzen (meldet alle Sitzungen des Benutzers ab).
    pub password: Option<String>,
    /// Gesperrte Konten können sich nicht anmelden.
    pub disabled: Option<bool>,
}

/// Einstellungen der Instanz (Admin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstanceSettings {
    /// Jeder darf sich selbst ein Konto anlegen.
    pub registration_open: bool,
    /// Feeds alle N Minuten abrufen (5–1440).
    pub refresh_minutes: u32,
    /// Gelesene, nicht gemerkte Artikel nach N Tagen löschen (0 = nie).
    pub retention_days: u32,
}

impl Default for InstanceSettings {
    fn default() -> Self {
        Self {
            registration_open: false,
            refresh_minutes: 30,
            retention_days: 60,
        }
    }
}

// ------------------------------------------------------------------ Ordner & Feeds

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub position: i64,
    /// Ungelesene Artikel aller Feeds im Ordner.
    pub unread: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInput {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub id: i64,
    pub folder_id: Option<i64>,
    /// Angezeigter Name (vom Benutzer änderbar, sonst Titel des Feeds).
    pub title: String,
    pub feed_url: String,
    /// Webseite des Feeds (für Links und das Icon).
    pub site_url: String,
    /// Favicon der Webseite; leer = Anfangsbuchstabe anzeigen.
    pub icon_url: String,
    pub description: String,
    pub unread: i64,
    pub total: i64,
    pub last_fetched_at: Option<i64>,
    /// Letzter Fehler beim Abrufen (leer = ok).
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// Alles für die Seitenleiste in einer Anfrage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub folders: Vec<Folder>,
    pub feeds: Vec<Feed>,
    /// Ungelesen insgesamt
    pub unread: i64,
    /// Gemerkt (Stern)
    pub starred: i64,
    /// Ungelesen aus den letzten 24 Stunden
    pub today: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverInput {
    /// Adresse einer Webseite oder direkt eines Feeds.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredFeed {
    pub url: String,
    pub title: String,
    /// "rss", "atom" oder "json"
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeInput {
    /// Adresse des Feeds (oder einer Webseite – dann wird der erste gefundene Feed genommen).
    pub url: String,
    #[serde(default)]
    pub folder_id: Option<i64>,
    /// Eigener Name; leer = Titel des Feeds.
    #[serde(default)]
    pub title: String,
}

/// Ersetzt Name und Ordner (die Oberfläche schickt immer beide).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFeedInput {
    pub title: String,
    pub folder_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    pub feeds: u32,
    pub new_articles: u32,
    pub errors: u32,
}

// ------------------------------------------------------------------ Artikel

/// Welche Artikel `GET /articles` liefert (`?view=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    All,
    Unread,
    Starred,
    /// Die letzten 24 Stunden
    Today,
}

impl View {
    pub fn as_str(&self) -> &'static str {
        match self {
            View::All => "all",
            View::Unread => "unread",
            View::Starred => "starred",
            View::Today => "today",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "unread" => View::Unread,
            "starred" => View::Starred,
            "today" => View::Today,
            _ => View::All,
        }
    }
}

/// Eintrag in der Artikelliste.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleSummary {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub feed_icon: String,
    pub title: String,
    /// Link zum Original
    pub url: String,
    pub author: String,
    /// Reiner Text, höchstens ~300 Zeichen
    pub excerpt: String,
    /// Vorschaubild (erstes Bild oder Enclosure); leer = keins
    pub image_url: String,
    pub published_at: i64,
    pub read: bool,
    pub starred: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticlePage {
    pub items: Vec<ArticleSummary>,
    /// Für die nächste Seite als `?cursor=` mitschicken; `None` = Ende.
    pub next_cursor: Option<String>,
}

/// Ein Artikel zum Lesen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub feed_icon: String,
    pub feed_site_url: String,
    pub title: String,
    pub url: String,
    pub author: String,
    /// Gesäubertes HTML (ammonia): sicher für `dangerous_inner_html`, Links mit
    /// `target="_blank" rel="noopener noreferrer"`.
    pub content_html: String,
    pub image_url: String,
    pub published_at: i64,
    pub read: bool,
    pub starred: bool,
    /// Lesezeit in Minuten (mindestens 1)
    pub reading_minutes: u32,
    /// Gespeicherte KI-Zusammenfassung, falls schon erzeugt
    pub ai_summary: Option<String>,
}

/// Nur gesetzte Felder werden geändert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ArticleStateInput {
    pub read: Option<bool>,
    pub starred: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleState {
    pub id: i64,
    pub read: bool,
    pub starred: bool,
}

/// Alles als gelesen markieren – eingeschränkt auf Feed, Ordner oder Ansicht.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MarkReadInput {
    pub feed_id: Option<i64>,
    pub folder_id: Option<i64>,
    /// `View::Today` = nur die letzten 24 Stunden, `View::Starred` = nur gemerkte
    pub view: Option<View>,
    /// Nur Artikel bis zu diesem Zeitpunkt (published_at ≤ before) – damit beim Lesen
    /// eingetroffene neue Artikel nicht verloren gehen. Fehlt = jetzt.
    pub before: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkReadResult {
    pub updated: i64,
}

// ------------------------------------------------------------------ OPML

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpmlImportResult {
    pub added: u32,
    /// Schon abonniert
    pub skipped: u32,
    pub folders: u32,
    /// Meldungen zu Einträgen, die nicht gingen
    pub errors: Vec<String>,
}

// ------------------------------------------------------------------ KI

/// Zusammenfassung eines Artikels (Markdown: kurze Absätze und Aufzählungen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSummary {
    pub summary: String,
    pub model: String,
    /// Schon vorher gespeichert (keine neue KI-Anfrage)
    pub cached: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BriefingInput {
    /// Zeitraum in Stunden (Vorgabe 24, höchstens 168)
    pub hours: Option<u32>,
    pub feed_id: Option<i64>,
    pub folder_id: Option<i64>,
    /// Nur ungelesene Artikel (Vorgabe: ja)
    pub unread_only: Option<bool>,
}

/// Quelle in einer KI-Antwort (verlinkt in der Oberfläche).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub article_id: i64,
    pub title: String,
    pub feed_title: String,
    pub published_at: i64,
}

/// KI-Überblick: die wichtigsten Themen der neuen Artikel, nach Thema gruppiert (Markdown).
/// Verweise im Text als `[1]`, `[2]` … auf `sources` (1-basiert).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Briefing {
    pub text: String,
    pub sources: Vec<Source>,
    pub model: String,
    pub generated_at: i64,
    pub article_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// "user" oder "assistant"
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantInput {
    /// Bisheriges Gespräch, die letzte Nachricht ist die Frage.
    pub messages: Vec<ChatMessage>,
}

/// Antwort von Dieter. Verweise im Text als `[1]`, `[2]` … auf `sources` (1-basiert).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAnswer {
    pub answer: String,
    pub sources: Vec<Source>,
    pub model: String,
}

/// Ein KI-Anbieter in den Einstellungen (Schlüssel selbst verlassen den Server nie).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderInfo {
    pub id: String,
    pub name: String,
    /// Vorgabe-Adresse der API
    pub url: String,
    /// Vorgabe-Modell
    pub model: String,
    pub needs_key: bool,
    /// Läuft lokal (Ollama, LM Studio, …)
    pub local: bool,
    /// Wo es einen Schlüssel gibt (leer = nirgends nötig)
    pub key_url: String,
    pub has_key: bool,
    /// Anfang/Ende des Schlüssels zur Wiedererkennung, z. B. "sk-or-v1-0…cdef"
    pub key_hint: String,
    /// Eigene Adresse (leer = Vorgabe)
    pub custom_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsView {
    pub disabled: bool,
    /// Gewählter Anbieter (ID)
    pub provider: String,
    /// Gewähltes Modell (leer = Vorgabe des Anbieters)
    pub model: String,
    pub fallbacks: Vec<String>,
    /// Sprache von Zusammenfassungen und Überblick: Code ("de", "en", …) oder "auto"
    pub language: String,
    pub effective_model: String,
    pub effective_url: String,
    /// "settings", "environment", "builtin" oder "none"
    pub key_source: String,
    pub key_hint: String,
    /// KI ist nutzbar (eingeschaltet, Schlüssel vorhanden wo nötig)
    pub available: bool,
    /// Was fehlt, falls nicht nutzbar (leer = nichts)
    pub missing: String,
    pub providers: Vec<AiProviderInfo>,
    /// Wählbare Sprachen: (Code, Name)
    pub languages: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettingsInput {
    pub disabled: bool,
    /// Anbieter-ID (leer = bisheriger)
    pub provider: String,
    pub model: String,
    pub fallbacks: Vec<String>,
    /// Neuer Schlüssel für den Anbieter; leer = bisherigen behalten
    pub key: String,
    /// Eigenen Schlüssel des Anbieters entfernen
    pub clear_key: bool,
    /// Eigene Adresse der API (`Some("")` = Vorgabe des Anbieters, `None` = unverändert)
    pub url: Option<String>,
    /// Sprache (Code oder "auto"); `None` = unverändert
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModels {
    pub models: Vec<AiModel>,
}

// ------------------------------------------------------------------ API-Tokens (MCP)

/// Persönliches API-Token (für den MCP-Server unter `/mcp`). Das Token selbst gibt es nur
/// einmal beim Anlegen ([`ApiTokenCreated`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    /// Anfang des Tokens zum Wiedererkennen
    pub prefix: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiTokenInput {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenCreated {
    pub token: ApiToken,
    /// Das Token (`nrznews_…`) – wird nur jetzt angezeigt
    pub secret: String,
}

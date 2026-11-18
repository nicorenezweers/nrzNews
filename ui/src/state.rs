//! Globaler Zustand: angemeldeter Benutzer, Überblick der Seitenleiste (Ordner, Feeds, Zähler)
//! und die Zähler-Pflege bei gelesen/gemerkt (sofort in der Oberfläche, danach vom Server).

use crate::api;
use dioxus::prelude::*;
use nrznews_types::{Overview, Preferences, UpdateMeInput, User};

/// Angemeldeter Benutzer (None = noch nicht geladen / abgemeldet).
pub static ME: GlobalSignal<Option<User>> = Signal::global(|| None);

/// Ordner, Feeds und Zähler für die Seitenleiste.
pub static OVERVIEW: GlobalSignal<Option<Overview>> = Signal::global(|| None);

/// Erhöhen, damit die Artikelliste neu lädt (nach Abruf, Import, neuem Feed …).
pub static REFRESH: GlobalSignal<u64> = Signal::global(|| 0);

/// „Nur ungelesene“ in der Liste (None = Vorgabe aus den Einstellungen).
pub static UNREAD_ONLY: GlobalSignal<Option<bool>> = Signal::global(|| None);

/// Seitenleiste auf dem Handy geöffnet.
pub static SIDEBAR_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Dialog „Feed hinzufügen“ geöffnet (aus Seitenleiste, Tastatur und leeren Listen).
pub static ADD_FEED_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Ungelesene der gerade gezeigten Auswahl beim letzten Laden der Liste – steigt der Zähler
/// danach (Abruf im Hintergrund), zeigt die Liste „Neue Artikel“.
pub static LIST_BASELINE: GlobalSignal<Option<(String, i64)>> = Signal::global(|| None);

static SYNC_GEN: GlobalSignal<u64> = Signal::global(|| 0);

pub fn refresh() {
    *REFRESH.write() += 1;
}

/// Einstellungen des angemeldeten Benutzers (Vorgaben, solange niemand angemeldet ist).
pub fn prefs() -> Preferences {
    ME.read().as_ref().map(|u| u.preferences.clone()).unwrap_or_default()
}

/// Gilt „nur ungelesene“ gerade?
pub fn unread_only() -> bool {
    UNREAD_ONLY().unwrap_or_else(|| prefs().unread_only)
}

pub fn is_admin() -> bool {
    ME.read().as_ref().is_some_and(|u| u.is_admin())
}

/// Nach 401: zur Anmeldung (ausser auf den Anmeldeseiten selbst).
pub fn on_unauthorized() {
    let Some(window) = web_sys::window() else { return };
    let path = window.location().pathname().unwrap_or_default();
    if path.starts_with("/login") || path.starts_with("/setup") || path.starts_with("/register") {
        return;
    }
    *ME.write() = None;
    let next = api::enc(&crate::util::current_path());
    let _ = window.location().assign(&format!("/login?next={next}"));
}

/// Überblick neu laden.
pub async fn load_overview() {
    if let Ok(o) = api::get::<Overview>("/overview").await {
        *OVERVIEW.write() = Some(o);
    }
}

/// Überblick bald neu laden – mehrere Änderungen kurz hintereinander ergeben eine Anfrage.
/// Danach gilt der neue Stand als gesehen (kein „Neue Artikel“ wegen eigener Änderungen).
pub fn sync_soon() {
    let generation = {
        let mut g = SYNC_GEN.write();
        *g += 1;
        *g
    };
    dioxus::core::spawn_forever(async move {
        gloo_timers::future::TimeoutFuture::new(700).await;
        if *SYNC_GEN.peek() != generation {
            return;
        }
        load_overview().await;
        rebase();
    });
}

/// Aktuellen Zähler der gezeigten Auswahl als gesehen merken.
pub fn rebase() {
    let key = LIST_BASELINE.peek().as_ref().map(|(k, _)| k.clone());
    if let Some(key) = key {
        if let Some(count) = OVERVIEW.peek().as_ref().and_then(|o| unread_for_key(o, &key)) {
            *LIST_BASELINE.write() = Some((key, count));
        }
    }
}

/// Ungelesene für einen Auswahl-Schlüssel (`all`, `today`, `feed:3`, `folder:2`).
pub fn unread_for_key(o: &Overview, key: &str) -> Option<i64> {
    match key.split_once(':') {
        Some(("feed", id)) => o.feeds.iter().find(|f| f.id.to_string() == id).map(|f| f.unread),
        Some(("folder", id)) => o.folders.iter().find(|f| f.id.to_string() == id).map(|f| f.unread),
        _ => match key {
            "all" => Some(o.unread),
            "today" => Some(o.today),
            _ => None,
        },
    }
}

/// Zähler der Seitenleiste sofort anpassen (gelesen/ungelesen, gemerkt), bevor der Server antwortet.
pub fn adjust_counts(feed_id: i64, published_at: i64, unread_delta: i64, starred_delta: i64) {
    let recent = crate::util::now() - published_at < 86_400;
    {
        let mut ov = OVERVIEW.write();
        let Some(o) = ov.as_mut() else { return };
        o.unread = (o.unread + unread_delta).max(0);
        if recent {
            o.today = (o.today + unread_delta).max(0);
        }
        o.starred = (o.starred + starred_delta).max(0);
        let mut folder = None;
        if let Some(f) = o.feeds.iter_mut().find(|f| f.id == feed_id) {
            f.unread = (f.unread + unread_delta).max(0);
            folder = f.folder_id;
        }
        if let Some(fid) = folder {
            if let Some(f) = o.folders.iter_mut().find(|f| f.id == fid) {
                f.unread = (f.unread + unread_delta).max(0);
            }
        }
    }
    // Eigene Änderung: die gemerkte Zahl der Liste mitziehen
    if unread_delta != 0 {
        if let Some((_, n)) = LIST_BASELINE.write().as_mut() {
            *n = (*n + unread_delta).max(0);
        }
    }
}

/// Persönliche Einstellungen speichern (ganzer Satz) und übernehmen.
pub async fn save_prefs(p: Preferences) -> api::ApiResult<User> {
    let user = api::put::<User, _>("/me", &UpdateMeInput { display_name: None, preferences: Some(p) }).await?;
    *ME.write() = Some(user.clone());
    Ok(user)
}

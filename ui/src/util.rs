//! Kleine Helfer: Datum, relative Zeit, Browser-Speicher, Fokus.

use crate::i18n::{tr, trf};

fn js_date(ts: i64) -> js_sys::Date {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0))
}

/// Unix-Sekunden → "24.10.2026, 14:05" in der Zeitzone des Browsers.
pub fn datetime(ts: i64) -> String {
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
    js_date(ts).to_locale_string(crate::i18n::locale(), &opts).into()
}

/// Unix-Sekunden → "24.10.2026".
pub fn date(ts: i64) -> String {
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    js_date(ts).to_locale_date_string(crate::i18n::locale(), &opts).into()
}

/// Unix-Sekunden → "Freitag, 31. Oktober 2026, 14:05" (Kopf des Lesemodus).
pub fn long_datetime(ts: i64) -> String {
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"full".into());
    let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
    js_date(ts).to_locale_string(crate::i18n::locale(), &opts).into()
}

pub fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Wie lange her: "gerade eben", "vor 5 Min.", "vor 3 Std.", "gestern", "vor 4 Tagen", sonst das Datum.
pub fn relative(ts: i64) -> String {
    let diff = now() - ts;
    if diff < 60 {
        tr("gerade eben")
    } else if diff < 3600 {
        trf("vor {0} Min.", &[&(diff / 60)])
    } else if diff < 86_400 {
        trf("vor {0} Std.", &[&(diff / 3600)])
    } else if diff < 2 * 86_400 {
        tr("gestern")
    } else if diff < 7 * 86_400 {
        trf("vor {0} Tagen", &[&(diff / 86_400)])
    } else {
        date(ts)
    }
}

pub fn storage_get(key: &str) -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
}

pub fn storage_set(key: &str, value: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(key, value);
    }
}

/// Aktuelle Adresse samt Suchteil (für `?next=` bei der Anmeldung).
pub fn current_path() -> String {
    let Some(w) = web_sys::window() else { return "/".into() };
    let loc = w.location();
    format!("{}{}", loc.pathname().unwrap_or_default(), loc.search().unwrap_or_default())
}

/// Erste Datei aus einem <input type=file> (onchange); leert das Feld danach.
pub fn input_file(e: &dioxus::prelude::FormEvent) -> Option<web_sys::File> {
    use wasm_bindgen::JsCast;
    let input = e
        .data()
        .downcast::<web_sys::Event>()
        .and_then(|ev| ev.target())
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())?;
    let file = input.files().and_then(|l| l.get(0));
    input.set_value("");
    file
}

/// Inhalt einer Datei als Text (UTF-8).
pub async fn file_text(file: &web_sys::File) -> Option<String> {
    wasm_bindgen_futures::JsFuture::from(file.text()).await.ok()?.as_string()
}

/// Anfangsbuchstabe für Platzhalter-Icons.
pub fn initial(name: &str) -> String {
    name.trim().chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "·".into())
}

/// Host einer Adresse ohne "www." (z. B. für Links zur Webseite).
pub fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// Element mit dieser ID in den sichtbaren Bereich holen (Liste beim Blättern mit j/k).
pub fn scroll_into_view(id: &str) {
    dioxus::prelude::document::eval(&format!(
        "requestAnimationFrame(() => document.getElementById({}).scrollIntoView({{ block: 'nearest' }}))",
        serde_json::to_string(id).unwrap_or_default()
    ));
}

/// Element mit dieser ID fokussieren (und Text markieren).
pub fn focus(id: &str) {
    dioxus::prelude::document::eval(&format!(
        "requestAnimationFrame(() => {{ const e = document.getElementById({}); if (e) {{ e.focus(); e.select && e.select(); }} }})",
        serde_json::to_string(id).unwrap_or_default()
    ));
}

/// Link in einem neuen Tab öffnen.
pub fn open_tab(url: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_and_initial() {
        assert_eq!(super::host("https://www.example.com/feed.xml"), "example.com");
        assert_eq!(super::host("http://blog.rust-lang.org"), "blog.rust-lang.org");
        assert_eq!(super::initial("  ärger"), "Ä");
        assert_eq!(super::initial("…"), "·");
    }
}

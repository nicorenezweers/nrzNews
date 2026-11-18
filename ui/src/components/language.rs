//! Sprachwahl als kleines Auswahlfeld (Anmeldeseiten). Angemeldet gilt die Sprache aus den
//! eigenen Einstellungen (Einstellungen → Lesen).

use crate::i18n::{self, LANG, LANGUAGES, tr};
use dioxus::prelude::*;

/// Kleines Auswahlfeld mit Symbol.
#[component]
pub fn LanguageSelect() -> Element {
    rsx! {
        label { class: "inline-flex items-center gap-1.5 text-xs text-muted-foreground",
            icons::Languages { class: "size-3.5" }
            span { class: "sr-only", {tr("Sprache")} }
            select {
                class: "cursor-pointer rounded-md border-0 bg-transparent py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground [&>option]:bg-popover",
                onchange: move |e| {
                    let code = e.value();
                    spawn(async move { i18n::set_language(&code, true).await; });
                },
                for (code , name) in LANGUAGES.iter().copied() {
                    option { value: code, selected: *LANG.read() == code, "{name}" }
                }
            }
        }
    }
}

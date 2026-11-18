//! Unbekannte Adresse.

use crate::components::common::*;
use crate::i18n::tr;
use crate::Route;
use dioxus::prelude::*;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let _ = segments;
    rsx! {
        div { class: "grid min-h-dvh place-items-center p-6",
            EmptyState { title: tr("Seite nicht gefunden"), description: tr("Diese Adresse gibt es in nrzNews nicht."), icon: rsx! { icons::Rss {} },
                Link { class: btn(BTN_OUTLINE), to: Route::All { article: None }, {tr("Zu den Artikeln")} }
            }
        }
    }
}

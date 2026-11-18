//! Lesen: Sprache der Oberfläche, „nur ungelesene“, beim Öffnen als gelesen markieren,
//! Schriftgrösse im Lesemodus, kompakte Liste. Jede Änderung wird sofort gespeichert.

use crate::components::common::*;
use crate::components::overlay::*;
use crate::i18n::{self, LANGUAGES, tr};
use crate::state::{self, UNREAD_ONLY};
use dioxus::prelude::*;
use nrznews_types::Preferences;

fn save(p: Preferences) {
    spawn(async move {
        match state::save_prefs(p).await {
            Ok(_) => toast(tr("Gespeichert.")),
            Err(e) => toast_error(e),
        }
    });
}

#[component]
pub fn Reading() -> Element {
    let p = state::prefs();
    let (p1, p2, p3, p4, p5) = (p.clone(), p.clone(), p.clone(), p.clone(), p.clone());
    rsx! {
        Section { title: tr("Sprache"), description: tr("Für die Oberfläche – auf allen Geräten, auf denen du angemeldet bist."),
            Segmented {
                options: LANGUAGES.iter().map(|(c, n)| (c.to_string(), n.to_string())).collect::<Vec<_>>(),
                value: p.language.clone(),
                onchange: move |code: String| {
                    let mut p = p1.clone();
                    p.language = code.clone();
                    spawn(async move {
                        i18n::set_language(&code, true).await;
                    });
                    save(p);
                },
            }
        }
        Section { title: tr("Liste"),
            div { class: "grid gap-5",
                Toggle {
                    checked: p.unread_only,
                    label: tr("Nur ungelesene Artikel zeigen"),
                    hint: tr("Vorgabe für die Listen; oben in der Liste lässt es sich jederzeit umschalten."),
                    onchange: move |on: bool| {
                        let mut p = p2.clone();
                        p.unread_only = on;
                        *UNREAD_ONLY.write() = None;
                        save(p);
                    },
                }
                Toggle {
                    checked: p.mark_read_on_open,
                    label: tr("Beim Öffnen als gelesen markieren"),
                    hint: tr("Sonst mit m oder dem Knopf im Lesemodus."),
                    onchange: move |on: bool| {
                        let mut p = p3.clone();
                        p.mark_read_on_open = on;
                        save(p);
                    },
                }
                Toggle {
                    checked: p.compact_list,
                    label: tr("Kompakte Liste"),
                    hint: tr("Nur Titel – ohne Auszug und Vorschaubild."),
                    onchange: move |on: bool| {
                        let mut p = p4.clone();
                        p.compact_list = on;
                        save(p);
                    },
                }
            }
        }
        Section { title: tr("Schriftgrösse im Lesemodus"),
            div { class: "grid gap-5",
                Segmented {
                    options: vec![("s".to_string(), tr("Klein")), ("m".to_string(), tr("Mittel")), ("l".to_string(), tr("Gross"))],
                    value: p.reader_size.clone(),
                    onchange: move |size: String| {
                        let mut p = p5.clone();
                        p.reader_size = size;
                        save(p);
                    },
                }
                div { class: "rounded-lg border bg-background/40 px-5 py-4",
                    div { class: "prose-reader", "data-size": "{p.reader_size}",
                        p { {tr("Gute Typografie macht das Lesen leicht: angenehme Zeilenlänge, ruhiger Zeilenabstand und genug Weissraum – damit nur der Text zählt.")} }
                    }
                }
            }
        }
    }
}

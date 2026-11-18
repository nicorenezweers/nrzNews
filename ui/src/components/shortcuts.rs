//! Übersicht der Tastenkürzel (mit `?` oder über das Profilmenü) – rust-ui Kbd.

use crate::components::overlay::Modal;
use crate::i18n::tr;
use crate::ui::kbd::{Kbd, KbdGroup};
use dioxus::prelude::*;

/// Hilfe zu den Tastenkürzeln geöffnet.
pub static SHORTCUTS_OPEN: GlobalSignal<bool> = Signal::global(|| false);

#[component]
pub fn ShortcutsHelp() -> Element {
    let rows: [(&[&str], String); 9] = [
        (&["j", "k"], tr("Nächster / vorheriger Artikel")),
        (&["o", "Enter"], tr("Original im neuen Tab öffnen")),
        (&["s"], tr("Merken oder nicht mehr merken")),
        (&["m"], tr("Gelesen / ungelesen")),
        (&["r"], tr("Feeds jetzt abrufen")),
        (&["/"], tr("Suchen")),
        (&["Esc"], tr("Artikel schliessen")),
        (&["?"], tr("Diese Übersicht")),
        (&["Shift", "A"], tr("Alle als gelesen markieren")),
    ];
    rsx! {
        Modal { open: SHORTCUTS_OPEN.signal(), title: tr("Tastenkürzel"), description: tr("Funktionieren überall in der Artikelliste, solange kein Eingabefeld aktiv ist."), class: "max-w-md",
            div { class: "divide-y rounded-lg border",
                for (keys , label) in rows {
                    div { class: "flex items-center justify-between gap-4 px-3 py-2 text-sm",
                        span { class: "text-muted-foreground", "{label}" }
                        KbdGroup {
                            for k in keys.iter().copied() {
                                Kbd { class: "min-w-6 px-1.5", "{k}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

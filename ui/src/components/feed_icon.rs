//! Icon eines Feeds: das Favicon der Webseite, sonst (leer oder nicht ladbar) der
//! Anfangsbuchstabe auf grauem Grund.

use crate::util;
use dioxus::prelude::*;
use tw_merge::tw_merge;

#[component]
pub fn FeedIcon(#[props(into)] src: String, #[props(into)] title: String, #[props(into, optional)] class: Option<String>) -> Element {
    let mut failed = use_signal(|| false);
    // Anderes Icon (z. B. neu geladener Feed) → neuer Versuch
    let key = src.clone();
    use_effect(use_reactive!(|key| {
        let _ = key;
        failed.set(false);
    }));
    let c = tw_merge!("relative grid size-4 shrink-0 place-items-center overflow-hidden rounded-[4px] bg-muted text-[9px] font-semibold text-muted-foreground", class.as_deref().unwrap_or(""));
    rsx! {
        span { class: "{c}", "aria-hidden": "true",
            if src.is_empty() || failed() {
                "{util::initial(&title)}"
            } else {
                img {
                    class: "size-full object-cover",
                    src: "{src}",
                    alt: "",
                    loading: "lazy",
                    decoding: "async",
                    referrerpolicy: "no-referrer",
                    onerror: move |_| failed.set(true),
                }
            }
        }
    }
}

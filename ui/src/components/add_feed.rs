//! „Feed hinzufügen“: Adresse einer Webseite oder eines Feeds → Suchen → Feed wählen →
//! Name und Ordner (auch neuer Ordner) → abonnieren → zum Feed.

use crate::api;
use crate::components::common::*;
use crate::components::feed_dialog::{FolderSelect, resolve_folder};
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use crate::state::{self, ADD_FEED_OPEN};
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{DiscoverInput, DiscoveredFeed, Feed, SubscribeInput};

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "atom" => "Atom",
        "json" => "JSON Feed",
        _ => "RSS",
    }
}

#[component]
pub fn AddFeedDialog() -> Element {
    let open = ADD_FEED_OPEN.signal();
    let mut url = use_signal(String::new);
    let mut found = use_signal(|| None::<Vec<DiscoveredFeed>>);
    let mut chosen = use_signal(String::new);
    let mut title = use_signal(String::new);
    let folder = use_signal(String::new);
    let new_folder = use_signal(String::new);
    let mut searching = use_signal(|| false);
    let mut subscribing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let nav = use_navigator();

    // Beim Öffnen leeren
    use_effect(move || {
        if open() {
            let (mut folder, mut new_folder) = (folder, new_folder);
            url.set(String::new());
            found.set(None);
            chosen.set(String::new());
            title.set(String::new());
            folder.set(String::new());
            new_folder.set(String::new());
            error.set(None);
        }
    });

    let mut search = move || {
        let u = url().trim().to_string();
        if u.is_empty() || searching() {
            return;
        }
        searching.set(true);
        error.set(None);
        spawn(async move {
            match api::post::<Vec<DiscoveredFeed>, _>("/feeds/discover", &DiscoverInput { url: u }).await {
                Ok(list) => {
                    if let Some(first) = list.first() {
                        chosen.set(first.url.clone());
                        title.set(first.title.clone());
                    }
                    found.set(Some(list));
                }
                Err(e) => {
                    found.set(None);
                    error.set(Some(e.message));
                }
            }
            searching.set(false);
        });
    };

    let subscribe = move |_| {
        let feed_url = chosen();
        if feed_url.is_empty() || subscribing() {
            return;
        }
        subscribing.set(true);
        error.set(None);
        spawn(async move {
            let res = async {
                let folder_id = resolve_folder(&folder(), &new_folder()).await?;
                api::post::<Feed, _>("/feeds", &SubscribeInput { url: feed_url, folder_id, title: title().trim().to_string() }).await
            }
            .await;
            subscribing.set(false);
            match res {
                Ok(feed) => {
                    let mut open = open;
                    open.set(false);
                    toast(trf("„{0}“ abonniert.", &[&feed.title]));
                    state::load_overview().await;
                    state::refresh();
                    nav.push(Route::FeedView { id: feed.id, article: None });
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };

    let results = found();
    rsx! {
        Modal { open, title: tr("Feed hinzufügen"), description: tr("Adresse einer Webseite oder direkt eines Feeds (RSS, Atom, JSON Feed)."),
            div { class: "grid gap-5",
                form { class: "flex gap-2",
                    onsubmit: move |e| {
                        e.prevent_default();
                        search();
                    },
                    input {
                        class: INPUT,
                        r#type: "url",
                        inputmode: "url",
                        autocomplete: "off",
                        placeholder: "https://example.com",
                        required: true,
                        value: "{url}",
                        oninput: move |e| url.set(e.value()),
                    }
                    button { class: btn(BTN_OUTLINE), r#type: "submit", disabled: searching() || url().trim().is_empty(),
                        if searching() {
                            icons::LoaderCircle { class: "size-4 animate-spin" }
                        } else {
                            icons::Search { class: "size-4" }
                        }
                        {tr("Suchen")}
                    }
                }
                if let Some(e) = error() {
                    Callout { text: e, error: true }
                }
                if let Some(list) = results {
                    if list.is_empty() {
                        Callout { text: tr("Auf dieser Seite wurde kein Feed gefunden. Probier die Adresse des Feeds selbst (z. B. …/feed oder …/rss.xml).") }
                    } else {
                        div { class: "grid gap-2 animate-fade-in",
                            span { class: "text-sm font-medium",
                                if list.len() == 1 { {tr("Gefundener Feed")} } else { {trf("{0} Feeds gefunden", &[&list.len()])} }
                            }
                            div { class: "grid max-h-56 gap-1.5 overflow-y-auto",
                                for f in list.iter().cloned() {
                                    button {
                                        key: "{f.url}",
                                        r#type: "button",
                                        class: if chosen() == f.url { "flex items-start gap-3 rounded-lg border border-primary/60 bg-accent/60 px-3 py-2.5 text-left" } else { "flex items-start gap-3 rounded-lg border px-3 py-2.5 text-left hover:bg-accent/40" },
                                        onclick: {
                                            let f = f.clone();
                                            move |_| {
                                                chosen.set(f.url.clone());
                                                title.set(f.title.clone());
                                            }
                                        },
                                        span { class: if chosen() == f.url { "mt-1 grid size-3.5 shrink-0 place-items-center rounded-full border-4 border-primary" } else { "mt-1 size-3.5 shrink-0 rounded-full border border-input" } }
                                        span { class: "min-w-0 flex-1",
                                            span { class: "block truncate text-sm font-medium",
                                                if f.title.is_empty() { {tr("(ohne Titel)")} } else { "{f.title}" }
                                            }
                                            span { class: "block truncate font-mono text-xs text-muted-foreground", "{f.url}" }
                                        }
                                        span { class: "shrink-0 rounded-full border px-2 py-0.5 text-[10px] uppercase tracking-wider text-muted-foreground", "{kind_label(&f.kind)}" }
                                    }
                                }
                            }
                        }
                        div { class: "grid gap-4 sm:grid-cols-2 animate-fade-in",
                            Field { label: tr("Name"), hint: tr("Leer = Titel des Feeds"),
                                input { class: INPUT, maxlength: 200, value: "{title}", oninput: move |e| title.set(e.value()) }
                            }
                            Field { label: tr("Ordner"),
                                FolderSelect { value: folder, new_name: new_folder }
                            }
                        }
                    }
                }
                div { class: "flex justify-end gap-2",
                    button { class: btn(BTN_OUTLINE), r#type: "button", onclick: move |_| { let mut open = open; open.set(false); }, {tr("Abbrechen")} }
                    button { class: btn(BTN_PRIMARY), r#type: "button", disabled: chosen().is_empty() || subscribing(), onclick: subscribe,
                        if subscribing() {
                            icons::LoaderCircle { class: "size-4 animate-spin" }
                        } else {
                            icons::Plus { class: "size-4" }
                        }
                        {tr("Abonnieren")}
                    }
                }
            }
        }
    }
}

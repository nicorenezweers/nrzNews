//! Feeds & Ordner: alle Abos (umbenennen, verschieben, abrufen, abbestellen, letzter Fehler),
//! Ordner (anlegen, umbenennen, löschen) und OPML (Import aus Datei, Export als Download).

use crate::api;
use crate::components::common::*;
use crate::components::feed_dialog::{FeedDialog, FolderDialog, delete_folder, unsubscribe};
use crate::components::feed_icon::FeedIcon;
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use crate::state::{self, ADD_FEED_OPEN, OVERVIEW};
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{Feed, Folder, OpmlImportResult, RefreshResult, UpdateFeedInput};

#[component]
pub fn FeedsSettings() -> Element {
    let ov = OVERVIEW.read().clone();
    let mut feed_open = use_signal(|| false);
    let mut editing_feed = use_signal(|| None::<Feed>);
    let mut folder_open = use_signal(|| false);
    let mut editing_folder = use_signal(|| None::<Folder>);
    let mut refreshing = use_signal(|| None::<i64>);
    let mut refreshing_all = use_signal(|| false);
    let mut filter = use_signal(String::new);

    let Some(ov) = ov else { return rsx! { Loading {} } };
    let folders = ov.folders.clone();
    let needle = filter().trim().to_lowercase();
    let feeds: Vec<Feed> = ov
        .feeds
        .iter()
        .filter(|f| needle.is_empty() || f.title.to_lowercase().contains(&needle) || f.feed_url.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    let broken = ov.feeds.iter().filter(|f| f.last_error.as_ref().is_some_and(|e| !e.trim().is_empty())).count();

    rsx! {
        Section {
            title: tr("Alle Feeds"),
            description: if broken > 0 { trf("{0} abonniert · {1} mit Problemen beim Abrufen", &[&ov.feeds.len(), &broken]) } else { trf("{0} abonniert", &[&ov.feeds.len()]) },
            actions: rsx! {
                button { class: btn(BTN_OUTLINE), disabled: refreshing_all() || ov.feeds.is_empty(),
                    onclick: move |_| {
                        refreshing_all.set(true);
                        spawn(async move {
                            match api::action::<RefreshResult>("/feeds/refresh").await {
                                Ok(r) => toast(trf("{0} Feeds abgerufen, {1} neue Artikel.", &[&r.feeds, &r.new_articles])),
                                Err(e) => toast_error(e),
                            }
                            refreshing_all.set(false);
                            state::load_overview().await;
                            state::refresh();
                        });
                    },
                    span { class: if refreshing_all() { "grid animate-spin" } else { "grid" }, icons::RefreshCw { class: "size-4" } }
                    {tr("Alle abrufen")}
                }
                button { class: btn(BTN_PRIMARY), onclick: move |_| *ADD_FEED_OPEN.write() = true,
                    icons::Plus { class: "size-4" }
                    {tr("Feed hinzufügen")}
                }
            },
            if ov.feeds.is_empty() {
                p { class: "text-sm text-muted-foreground", {tr("Noch keine Feeds – füge einen hinzu oder importiere eine OPML-Datei.")} }
            } else {
                if ov.feeds.len() > 8 {
                    div { class: "relative mb-4 max-w-sm",
                        icons::Search { class: "pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" }
                        input { class: "{INPUT} pl-9", r#type: "search", placeholder: tr("Feeds filtern …"), value: "{filter}", oninput: move |e| filter.set(e.value()) }
                    }
                }
                div { class: "divide-y rounded-lg border",
                    for f in feeds {
                        div { key: "{f.id}", class: "flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3",
                            div { class: "flex min-w-0 flex-1 basis-64 items-start gap-3",
                                FeedIcon { src: f.icon_url.clone(), title: f.title.clone(), class: "mt-0.5 size-5 text-[10px]" }
                                div { class: "min-w-0 flex-1",
                                    Link { to: Route::FeedView { id: f.id, article: None }, class: "block truncate text-sm font-medium hover:underline", "{f.title}" }
                                    p { class: "truncate font-mono text-xs text-muted-foreground", title: "{f.feed_url}", "{f.feed_url}" }
                                    p { class: "mt-0.5 text-xs text-muted-foreground",
                                        {trf("{0} ungelesen · {1} insgesamt", &[&f.unread, &f.total])}
                                        if let Some(t) = f.last_fetched_at {
                                            " · "
                                            span { title: "{util::datetime(t)}", {trf("abgerufen {0}", &[&util::relative(t)])} }
                                        }
                                    }
                                    if let Some(e) = f.last_error.clone().filter(|e| !e.trim().is_empty()) {
                                        p { class: "mt-1 flex items-start gap-1.5 text-xs text-destructive",
                                            icons::CircleAlert { class: "mt-px size-3.5 shrink-0" }
                                            span { class: "break-words", "{e}" }
                                        }
                                    }
                                }
                            }
                            div { class: "flex items-center gap-1",
                                select {
                                    class: select_class("h-8 w-40 text-xs"),
                                    title: tr("Ordner"),
                                    onchange: {
                                        let f = f.clone();
                                        move |e: FormEvent| {
                                            let folder_id = e.value().parse::<i64>().ok();
                                            let f = f.clone();
                                            spawn(async move {
                                                match api::put::<Feed, _>(&format!("/feeds/{}", f.id), &UpdateFeedInput { title: f.title.clone(), folder_id }).await {
                                                    Ok(_) => {
                                                        toast(tr("Feed verschoben."));
                                                        state::load_overview().await;
                                                    }
                                                    Err(e) => toast_error(e),
                                                }
                                            });
                                        }
                                    },
                                    option { value: "", selected: f.folder_id.is_none(), {tr("— ohne Ordner —")} }
                                    for x in folders.iter() {
                                        option { key: "{x.id}", value: "{x.id}", selected: f.folder_id == Some(x.id), "{x.name}" }
                                    }
                                }
                                button { class: ICON_BTN, title: tr("Umbenennen"),
                                    onclick: {
                                        let f = f.clone();
                                        move |_| {
                                            editing_feed.set(Some(f.clone()));
                                            feed_open.set(true);
                                        }
                                    },
                                    icons::Pencil {}
                                }
                                button { class: ICON_BTN, title: tr("Jetzt abrufen"), disabled: refreshing() == Some(f.id),
                                    onclick: {
                                        let id = f.id;
                                        move |_| {
                                            refreshing.set(Some(id));
                                            spawn(async move {
                                                match api::action::<Feed>(&format!("/feeds/{id}/refresh")).await {
                                                    Ok(f) => match f.last_error.filter(|e| !e.trim().is_empty()) {
                                                        Some(e) => toast_error(trf("Abruf fehlgeschlagen: {0}", &[&e])),
                                                        None => toast(trf("„{0}“ abgerufen.", &[&f.title])),
                                                    },
                                                    Err(e) => toast_error(e),
                                                }
                                                refreshing.set(None);
                                                state::load_overview().await;
                                                state::refresh();
                                            });
                                        }
                                    },
                                    span { class: if refreshing() == Some(f.id) { "grid animate-spin" } else { "grid" }, icons::RefreshCw { class: "size-4" } }
                                }
                                button { class: icon_btn("hover:text-destructive"), title: tr("Abbestellen"),
                                    onclick: {
                                        let f = f.clone();
                                        move |_| {
                                            let f = f.clone();
                                            spawn(async move {
                                                if unsubscribe(&f).await {
                                                    state::refresh();
                                                }
                                            });
                                        }
                                    },
                                    icons::Trash2 {}
                                }
                            }
                        }
                    }
                }
            }
        }

        Section {
            title: tr("Alle Ordner"),
            description: tr("Ordner gruppieren Feeds in der Seitenleiste. Gelöschte Ordner geben ihre Feeds frei."),
            actions: rsx! {
                button { class: btn(BTN_OUTLINE),
                    onclick: move |_| {
                        editing_folder.set(None);
                        folder_open.set(true);
                    },
                    icons::FolderPlus { class: "size-4" }
                    {tr("Neuer Ordner")}
                }
            },
            if folders.is_empty() {
                p { class: "text-sm text-muted-foreground", {tr("Noch keine Ordner.")} }
            } else {
                div { class: "divide-y rounded-lg border",
                    for x in folders.iter().cloned() {
                        div { key: "{x.id}", class: "flex items-center gap-3 px-4 py-2.5",
                            icons::Folder { class: "size-4 shrink-0 text-muted-foreground" }
                            Link { to: Route::FolderView { id: x.id, article: None }, class: "min-w-0 flex-1 truncate text-sm font-medium hover:underline", "{x.name}" }
                            span { class: "hidden text-xs text-muted-foreground sm:inline",
                                {
                                    let n = ov.feeds.iter().filter(|f| f.folder_id == Some(x.id)).count();
                                    if n == 1 { tr("1 Feed") } else { trf("{0} Feeds", &[&n]) }
                                }
                            }
                            button { class: ICON_BTN, title: tr("Umbenennen"),
                                onclick: {
                                    let x = x.clone();
                                    move |_| {
                                        editing_folder.set(Some(x.clone()));
                                        folder_open.set(true);
                                    }
                                },
                                icons::Pencil {}
                            }
                            button { class: icon_btn("hover:text-destructive"), title: tr("Löschen"),
                                onclick: {
                                    let x = x.clone();
                                    move |_| {
                                        let x = x.clone();
                                        spawn(async move {
                                            delete_folder(&x).await;
                                        });
                                    }
                                },
                                icons::Trash2 {}
                            }
                        }
                    }
                }
            }
        }

        Opml {}

        FeedDialog { open: feed_open, feed: editing_feed() }
        FolderDialog { open: folder_open, folder: editing_folder() }
    }
}

#[component]
fn Opml() -> Element {
    let mut importing = use_signal(|| false);
    let mut result = use_signal(|| None::<OpmlImportResult>);
    rsx! {
        Section { title: tr("OPML"), description: tr("Abos aus einem anderen Reader übernehmen oder hier sichern – Ordner bleiben erhalten."),
            div { class: "flex flex-wrap items-center gap-2",
                label { class: "{btn(BTN_OUTLINE)} cursor-pointer",
                    if importing() {
                        icons::LoaderCircle { class: "size-4 animate-spin" }
                    } else {
                        icons::Upload { class: "size-4" }
                    }
                    {tr("OPML importieren …")}
                    input {
                        r#type: "file",
                        class: "hidden",
                        accept: ".opml,.xml,text/xml,text/x-opml,application/xml",
                        disabled: importing(),
                        onchange: move |e| {
                            let Some(file) = util::input_file(&e) else { return };
                            importing.set(true);
                            result.set(None);
                            spawn(async move {
                                let Some(text) = util::file_text(&file).await else {
                                    importing.set(false);
                                    toast_error(tr("Die Datei liess sich nicht lesen."));
                                    return;
                                };
                                match api::post_text::<OpmlImportResult>("/opml", "text/xml; charset=utf-8", text).await {
                                    Ok(r) => {
                                        toast(trf("{0} Feeds importiert.", &[&r.added]));
                                        result.set(Some(r));
                                        state::load_overview().await;
                                        state::refresh();
                                    }
                                    Err(e) => toast_error(e),
                                }
                                importing.set(false);
                            });
                        },
                    }
                }
                a { class: btn(BTN_OUTLINE), href: "/api/v1/opml", download: "nrznews.opml",
                    icons::Download { class: "size-4" }
                    {tr("OPML exportieren")}
                }
            }
            if let Some(r) = result() {
                div { class: "mt-5 grid gap-3 rounded-lg border bg-muted/30 p-4 text-sm animate-fade-in",
                    div { class: "flex flex-wrap gap-x-6 gap-y-1",
                        span { {trf("{0} neu abonniert", &[&r.added])} }
                        span { class: "text-muted-foreground", {trf("{0} schon vorhanden", &[&r.skipped])} }
                        span { class: "text-muted-foreground", {trf("{0} Ordner angelegt", &[&r.folders])} }
                    }
                    if !r.errors.is_empty() {
                        div { class: "grid gap-1",
                            p { class: "font-medium text-destructive", {trf("{0} Einträge gingen nicht:", &[&r.errors.len()])} }
                            ul { class: "max-h-40 list-disc space-y-0.5 overflow-y-auto pl-5 text-xs text-muted-foreground",
                                for (i , e) in r.errors.iter().enumerate() {
                                    li { key: "{i}", "{e}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

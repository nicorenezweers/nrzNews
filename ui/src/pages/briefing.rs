//! Überblick: die KI fasst die neuen Artikel der letzten 24 Stunden, 3 oder 7 Tage nach Themen
//! zusammen (optional nur ein Ordner). Verweise [1], [2] führen zu den Artikeln.

use crate::api;
use crate::components::common::*;
use crate::components::feed_icon::FeedIcon;
use crate::i18n::{tr, trf};
use crate::markdown;
use crate::state::{self, OVERVIEW};
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{AiSettingsView, Briefing as BriefingData, BriefingInput, Feed};
use std::collections::HashMap;

/// Erstellte Überblicke dieser Sitzung (Zeitraum|Ordner → Überblick) – kein erneuter KI-Aufruf
/// beim Zurückkehren.
static CACHE: GlobalSignal<HashMap<String, BriefingData>> = Signal::global(HashMap::new);

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Loading,
    Failed(String),
}

#[component]
pub fn Briefing() -> Element {
    let mut hours = use_signal(|| 24u32);
    let mut folder = use_signal(String::new);
    let mut status = use_signal(|| Status::Idle);
    let mut generation = use_signal(|| 0u64);
    let admin = state::is_admin();

    // Admins sehen gleich, ob die KI eingerichtet ist
    let ai = use_resource(move || async move {
        if admin {
            api::get::<AiSettingsView>("/settings/ai").await.ok()
        } else {
            None
        }
    });
    let ai_missing = ai.read().as_ref().and_then(|a| a.as_ref()).filter(|a| !a.available).map(|a| if a.missing.is_empty() { tr("Die KI ist nicht eingerichtet.") } else { tr(&a.missing) });

    let key = move || format!("{}|{}", hours(), folder());
    let mut create = move |force: bool| {
        let k = key();
        if !force && CACHE.peek().contains_key(&k) {
            // Schon erstellt: zeigen, eine laufende Anfrage für einen anderen Zeitraum verwerfen
            generation += 1;
            status.set(Status::Idle);
            return;
        }
        let g = *generation.peek() + 1;
        generation.set(g);
        status.set(Status::Loading);
        let input = BriefingInput { hours: Some(hours()), feed_id: None, folder_id: folder().parse().ok(), unread_only: None };
        spawn(async move {
            let res = api::post::<BriefingData, _>("/briefing", &input).await;
            if *generation.peek() != g {
                return;
            }
            match res {
                Ok(b) => {
                    CACHE.write().insert(k, b);
                    status.set(Status::Idle);
                }
                Err(e) => status.set(Status::Failed(e.message)),
            }
        });
    };

    // Beim Öffnen und bei anderem Zeitraum/Ordner (sofern noch nicht erstellt)
    let dep = (key(), ai_missing.is_some());
    use_effect(use_reactive!(|dep| {
        let (_, blocked) = dep;
        if !blocked {
            create(false);
        }
    }));

    let ov = OVERVIEW.read().clone();
    let folders = ov.as_ref().map(|o| o.folders.clone()).unwrap_or_default();
    let feeds: Vec<Feed> = ov.as_ref().map(|o| o.feeds.clone()).unwrap_or_default();
    let current = CACHE.read().get(&key()).cloned();
    let loading = status() == Status::Loading;

    rsx! {
        div { id: "briefing-scroll", class: "h-full overflow-y-auto",
            div { class: "mx-auto w-full max-w-3xl px-4 py-6 sm:px-8 sm:py-10",
                PageHeader {
                    title: tr("Überblick"),
                    description: tr("Die wichtigsten Themen deiner neuen Artikel – zusammengefasst von der KI."),
                }
                div { class: "mb-6 flex flex-wrap items-center gap-2",
                    Segmented {
                        options: vec![("24".to_string(), tr("24 Stunden")), ("72".to_string(), tr("3 Tage")), ("168".to_string(), tr("7 Tage"))],
                        value: hours().to_string(),
                        onchange: move |v: String| hours.set(v.parse().unwrap_or(24)),
                    }
                    if !folders.is_empty() {
                        select { class: select_class("w-auto min-w-40"), onchange: move |e| folder.set(e.value()),
                            option { value: "", selected: folder().is_empty(), {tr("Alle Feeds")} }
                            for f in folders.iter() {
                                option { key: "{f.id}", value: "{f.id}", selected: folder() == f.id.to_string(), "{f.name}" }
                            }
                        }
                    }
                    div { class: "flex-1" }
                    button { class: btn(BTN_OUTLINE), disabled: loading || ai_missing.is_some(), onclick: move |_| create(true),
                        span { class: if loading { "grid animate-spin" } else { "grid" }, icons::RefreshCw { class: "size-4" } }
                        {tr("Neu erstellen")}
                    }
                }

                if let Some(m) = ai_missing.clone() {
                    NotConfigured { message: m, admin }
                } else if loading {
                    Thinking { hours: hours() }
                } else if let Status::Failed(e) = status() {
                    NotConfigured { message: e, admin }
                } else if let Some(b) = current {
                    BriefingView { briefing: b, feeds }
                } else {
                    Thinking { hours: hours() }
                }
            }
        }
    }
}

#[component]
fn Thinking(hours: u32) -> Element {
    let span = match hours {
        24 => tr("der letzten 24 Stunden"),
        72 => tr("der letzten 3 Tage"),
        _ => tr("der letzten 7 Tage"),
    };
    rsx! {
        div { class: "rounded-xl border bg-card p-6 shadow-sm animate-fade-in",
            div { class: "flex items-center gap-3",
                span { class: "grid size-10 place-items-center rounded-full bg-muted",
                    icons::Sparkles { class: "size-5 animate-pulse text-muted-foreground" }
                }
                div {
                    p { class: "font-medium", {tr("Einen Moment – die KI liest mit.")} }
                    p { class: "text-sm text-muted-foreground", {trf("Sie sortiert die Artikel {0} nach Themen. Das dauert meist 10–30 Sekunden.", &[&span])} }
                }
            }
            div { class: "mt-6 space-y-3",
                div { class: "h-4 w-1/3 animate-pulse rounded bg-muted" }
                div { class: "h-3 w-full animate-pulse rounded bg-muted" }
                div { class: "h-3 w-11/12 animate-pulse rounded bg-muted" }
                div { class: "h-3 w-4/5 animate-pulse rounded bg-muted" }
                div { class: "mt-5 h-4 w-1/4 animate-pulse rounded bg-muted" }
                div { class: "h-3 w-full animate-pulse rounded bg-muted" }
                div { class: "h-3 w-3/4 animate-pulse rounded bg-muted" }
            }
        }
    }
}

#[component]
fn NotConfigured(message: String, admin: bool) -> Element {
    rsx! {
        div { class: "rounded-xl border bg-card p-6 shadow-sm animate-fade-in",
            div { class: "flex items-start gap-3",
                span { class: "grid size-10 shrink-0 place-items-center rounded-full bg-muted",
                    icons::Sparkles { class: "size-5 text-muted-foreground" }
                }
                div { class: "grid gap-1",
                    p { class: "font-medium", {tr("Kein Überblick möglich")} }
                    p { class: "text-sm text-muted-foreground", "{message}" }
                    if admin {
                        p { class: "text-sm text-muted-foreground", {tr("Richte unter KI & Modelle einen Anbieter ein – lokal (z. B. Ollama) oder in der Cloud.")} }
                        Link { class: "{btn(BTN_PRIMARY)} mt-3 justify-self-start", to: Route::Settings { section: "ai".into() },
                            icons::Settings2 { class: "size-4" }
                            {tr("KI einrichten")}
                        }
                    } else {
                        p { class: "text-sm text-muted-foreground", {tr("Ein Administrator richtet die KI unter Einstellungen → KI & Modelle ein.")} }
                    }
                }
            }
        }
    }
}

#[component]
fn BriefingView(briefing: BriefingData, feeds: Vec<Feed>) -> Element {
    if briefing.article_count == 0 || briefing.text.trim().is_empty() {
        return rsx! {
            div { class: "rounded-xl border bg-card shadow-sm",
                EmptyState { title: tr("Nichts Neues"), description: tr("In diesem Zeitraum sind keine ungelesenen Artikel eingetroffen."), icon: rsx! { icons::Sun {} } }
            }
        };
    }
    let ids: Vec<i64> = briefing.sources.iter().map(|s| s.article_id).collect();
    let icon = |title: &str| feeds.iter().find(|f| f.title == title).map(|f| f.icon_url.clone()).unwrap_or_default();
    rsx! {
        article { class: "rounded-xl border bg-card shadow-sm animate-fade-in",
            div { class: "flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-5 py-3 text-xs text-muted-foreground sm:px-7",
                span { class: "inline-flex items-center gap-1.5", icons::Newspaper { class: "size-3.5" }
                    if briefing.article_count == 1 { {tr("1 Artikel")} } else { {trf("{0} Artikel", &[&briefing.article_count])} }
                }
                span { class: "inline-flex items-center gap-1.5", title: "{util::datetime(briefing.generated_at)}", icons::Clock { class: "size-3.5" }
                    {trf("erstellt {0}", &[&util::relative(briefing.generated_at)])}
                }
                if !briefing.model.is_empty() {
                    span { class: "truncate font-mono", "{briefing.model}" }
                }
            }
            div { class: "prose-ai px-5 py-6 text-[0.97rem] text-foreground/90 sm:px-7", {markdown::render(&briefing.text, &ids)} }
            if !briefing.sources.is_empty() {
                div { class: "border-t px-5 py-5 sm:px-7",
                    p { class: "mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground", {tr("Quellen")} }
                    ol { class: "grid gap-1",
                        for (n , s) in briefing.sources.iter().cloned().enumerate() {
                            li { key: "{s.article_id}",
                                Link { to: Route::All { article: Some(s.article_id) }, class: "group flex items-start gap-3 rounded-lg px-2 py-1.5 -mx-2 hover:bg-accent/50",
                                    span { class: "mt-0.5 grid h-5 min-w-5 place-items-center rounded-md bg-muted px-1 text-[11px] font-semibold tabular-nums text-muted-foreground", "{n + 1}" }
                                    span { class: "min-w-0 flex-1",
                                        span { class: "block text-sm leading-snug group-hover:underline group-hover:decoration-foreground/30 group-hover:underline-offset-4", "{s.title}" }
                                        span { class: "mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground",
                                            FeedIcon { src: icon(&s.feed_title), title: s.feed_title.clone(), class: "size-3.5 text-[8px]" }
                                            span { class: "truncate", "{s.feed_title}" }
                                            span { class: "opacity-60", "·" }
                                            span { class: "shrink-0", "{util::relative(s.published_at)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

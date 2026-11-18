//! Einstellungen (Zahnrad unten in der Seitenleiste oder Profilmenü): das eigene Konto,
//! Lesen, Feeds & Ordner und – für Administratoren – Benutzer, Registrierung & Abruf, KI.

mod account;
mod admin;
mod ai;
mod feeds;
mod reading;

use crate::components::common::*;
use crate::i18n::tr;
use crate::state;
use crate::Route;
use dioxus::prelude::*;

struct Item {
    key: &'static str,
    label: &'static str,
    group: &'static str,
}

const ITEMS: &[Item] = &[
    Item { key: "account", label: "Konto", group: "Persönlich" },
    Item { key: "reading", label: "Lesen", group: "Persönlich" },
    Item { key: "feeds", label: "Feeds & Ordner", group: "Persönlich" },
    Item { key: "users", label: "Benutzer", group: "Instanz" },
    Item { key: "instance", label: "Registrierung & Abruf", group: "Instanz" },
    Item { key: "ai", label: "KI & Modelle", group: "Instanz" },
];

fn icon(key: &str) -> Element {
    match key {
        "account" => rsx! { icons::User {} },
        "reading" => rsx! { icons::BookOpen {} },
        "feeds" => rsx! { icons::Rss {} },
        "users" => rsx! { icons::Users {} },
        "ai" => rsx! { icons::Sparkles {} },
        _ => rsx! { icons::ShieldCheck {} },
    }
}

#[component]
pub fn Settings(section: String) -> Element {
    let admin = state::is_admin();
    let visible: Vec<&Item> = ITEMS.iter().filter(|i| i.group != "Instanz" || admin).collect();
    let mut groups: Vec<&str> = Vec::new();
    for i in &visible {
        if !groups.contains(&i.group) {
            groups.push(i.group);
        }
    }
    let title = visible.iter().find(|i| i.key == section).map(|i| tr(i.label)).unwrap_or_else(|| tr("Einstellungen"));
    rsx! {
        div { class: "h-full overflow-y-auto",
            div { class: "mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 md:py-8",
                PageHeader { title: tr("Einstellungen"), description: title }
                div { class: "grid gap-6 lg:grid-cols-[13rem_minmax(0,1fr)]",
                    nav { class: "-mx-1 flex gap-1 overflow-x-auto px-1 pb-1 lg:mx-0 lg:flex-col lg:overflow-visible lg:px-0",
                        for g in groups.iter().copied() {
                            p { class: "hidden px-2.5 pt-3 pb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground first:pt-0 lg:block", {tr(g)} }
                            for i in visible.iter().filter(|i| i.group == g) {
                                Link {
                                    key: "{i.key}",
                                    to: Route::Settings { section: i.key.to_string() },
                                    class: if i.key == section { "flex shrink-0 items-center gap-2.5 rounded-md bg-accent px-2.5 py-1.5 text-sm font-medium [&_svg]:size-4" } else { "flex shrink-0 items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground hover:bg-accent/50 hover:text-foreground [&_svg]:size-4" },
                                    {icon(i.key)}
                                    {tr(i.label)}
                                }
                            }
                        }
                    }
                    div { class: "min-w-0 space-y-6",
                        match section.as_str() {
                            "account" => rsx! { account::Account {} },
                            "reading" => rsx! { reading::Reading {} },
                            "feeds" => rsx! { feeds::FeedsSettings {} },
                            "users" if admin => rsx! { admin::Users {} },
                            "instance" if admin => rsx! { admin::Instance {} },
                            "ai" if admin => rsx! { ai::Ai {} },
                            _ => rsx! { Callout { text: tr("Diesen Bereich gibt es nicht oder er ist nur für Administratoren.") } },
                        }
                    }
                }
            }
        }
    }
}

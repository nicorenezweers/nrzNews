//! Gemeinsame Bausteine im Stil von rust-ui: Feld mit Beschriftung, Knöpfe, Seitenkopf,
//! leerer Zustand, Ladeanzeige, Hinweis, Abschnitt, Schalter, Auswahl in Segmenten.

use crate::i18n::tr;
use dioxus::prelude::*;
use tw_merge::tw_merge;

pub const INPUT: &str = "flex h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-base shadow-xs outline-none transition-[color,box-shadow] placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 md:text-sm dark:bg-input/30";
pub const SELECT: &str = "flex h-9 w-full items-center rounded-md border border-input bg-transparent px-3 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 dark:bg-input/30 [&>option]:bg-popover";
pub const BTN: &str = "inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";
pub const BTN_PRIMARY: &str = "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 h-9 px-4 py-2";
pub const BTN_OUTLINE: &str = "border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground dark:bg-input/30 dark:border-input dark:hover:bg-input/50 h-9 px-4 py-2";
pub const BTN_GHOST: &str = "hover:bg-accent hover:text-accent-foreground dark:hover:bg-accent/50 h-9 px-3";
pub const BTN_SM: &str = "h-8 px-3 text-[13px]";
/// Kleiner runder Symbolknopf (Werkzeugleisten in Liste und Lesemodus).
/// Die Symbole selbst nehmen keine Maus an – sonst zeigt der Browser ihren SVG-Titel statt `title`.
pub const ICON_BTN: &str = "inline-grid size-8 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4";

/// Klassen für einen Knopf, z. B. `btn(BTN_PRIMARY)`.
pub fn btn(variant: &str) -> String {
    format!("{BTN} {variant}")
}

/// Kleiner Knopf (Höhe 8), z. B. `btn_sm(BTN_OUTLINE)`.
pub fn btn_sm(variant: &str) -> String {
    tw_merge!(BTN, variant, BTN_SM)
}

/// Eingabefeld mit eigenen Klassen (Konflikte wie `h-8` statt `h-9` werden aufgelöst).
pub fn input_class(extra: &str) -> String {
    tw_merge!(INPUT, extra)
}

/// Auswahlfeld mit eigenen Klassen.
pub fn select_class(extra: &str) -> String {
    tw_merge!(SELECT, extra)
}

/// Symbolknopf mit eigenen Klassen.
pub fn icon_btn(extra: &str) -> String {
    tw_merge!(ICON_BTN, extra)
}

#[component]
pub fn Field(#[props(into)] label: String, #[props(into, optional)] hint: Option<String>, #[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let c = tw_merge!("grid gap-1.5", class.as_deref().unwrap_or(""));
    rsx! {
        label { class: "{c}",
            span { class: "text-sm font-medium leading-none", "{label}" }
            {children}
            if let Some(h) = hint {
                span { class: "text-xs text-muted-foreground", "{h}" }
            }
        }
    }
}

#[component]
pub fn PageHeader(#[props(into)] title: String, #[props(into, optional)] description: Option<String>, #[props(optional)] actions: Option<Element>, #[props(optional)] eyebrow: Option<String>) -> Element {
    rsx! {
        div { class: "mb-6 flex flex-wrap items-end justify-between gap-4",
            div { class: "flex min-w-0 items-center gap-2",
                MenuButton {}
                div { class: "min-w-0",
                    if let Some(e) = eyebrow {
                        p { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", "{e}" }
                    }
                    h1 { class: "truncate text-2xl font-semibold tracking-tight", "{title}" }
                    if let Some(d) = description {
                        p { class: "mt-1 text-sm text-muted-foreground", "{d}" }
                    }
                }
            }
            if let Some(a) = actions {
                div { class: "flex flex-wrap items-center gap-2", {a} }
            }
        }
    }
}

/// Öffnet auf schmalen Bildschirmen die Seitenleiste.
#[component]
pub fn MenuButton() -> Element {
    rsx! {
        button { class: "{ICON_BTN} -ml-1 lg:hidden", title: tr("Menü"), "aria-label": tr("Menü"),
            onclick: move |_| *crate::state::SIDEBAR_OPEN.write() = true,
            icons::Menu { class: "size-5" }
        }
    }
}

#[component]
pub fn EmptyState(#[props(into)] title: String, #[props(into, optional)] description: Option<String>, #[props(optional)] icon: Option<Element>, #[props(optional)] children: Option<Element>, #[props(into, optional)] class: Option<String>) -> Element {
    let c = tw_merge!("flex flex-col items-center justify-center gap-3 p-10 text-center animate-fade-in", class.as_deref().unwrap_or(""));
    rsx! {
        div { class: "{c}",
            if let Some(i) = icon {
                div { class: "grid size-12 place-items-center rounded-xl bg-muted text-muted-foreground [&_svg]:size-6", {i} }
            }
            div {
                p { class: "font-medium", "{title}" }
                if let Some(d) = description {
                    p { class: "mx-auto mt-1 max-w-sm text-sm text-muted-foreground", "{d}" }
                }
            }
            if let Some(c) = children {
                {c}
            }
        }
    }
}

#[component]
pub fn Loading(#[props(into, optional)] text: Option<String>) -> Element {
    let text = text.unwrap_or_else(|| tr("Lade …"));
    rsx! {
        div { class: "flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground",
            icons::LoaderCircle { class: "size-4 animate-spin" }
            "{text}"
        }
    }
}

#[component]
pub fn Callout(#[props(into)] text: String, #[props(default)] error: bool, #[props(optional)] children: Option<Element>) -> Element {
    let c = if error { "border-destructive/40 bg-destructive/10" } else { "bg-muted/50" };
    rsx! {
        div { class: "flex items-start gap-2 rounded-lg border px-3 py-2.5 text-sm {c}",
            if error {
                icons::CircleAlert { class: "mt-0.5 size-4 shrink-0 text-destructive" }
            } else {
                icons::Info { class: "mt-0.5 size-4 shrink-0 text-muted-foreground" }
            }
            div { class: "grid gap-2",
                span { "{text}" }
                if let Some(c) = children {
                    {c}
                }
            }
        }
    }
}

/// Abschnitt einer Einstellungsseite (rust-ui Card).
#[component]
pub fn Section(#[props(into)] title: String, #[props(into, optional)] description: Option<String>, #[props(optional)] actions: Option<Element>, children: Element) -> Element {
    rsx! {
        section { class: "rounded-xl border bg-card text-card-foreground shadow-sm animate-fade-in",
            div { class: "flex flex-wrap items-start justify-between gap-3 border-b px-5 py-4 sm:px-6",
                div { class: "min-w-0",
                    h2 { class: "font-semibold leading-none", "{title}" }
                    if let Some(d) = description {
                        p { class: "mt-1.5 text-sm text-muted-foreground", "{d}" }
                    }
                }
                if let Some(a) = actions {
                    div { class: "flex flex-wrap items-center gap-2", {a} }
                }
            }
            div { class: "px-5 py-5 sm:px-6", {children} }
        }
    }
}

/// rust-ui Switch als kontrollierte Komponente.
#[component]
pub fn Toggle(checked: bool, onchange: EventHandler<bool>, #[props(into, optional)] label: Option<String>, #[props(into, optional)] hint: Option<String>, #[props(default)] disabled: bool) -> Element {
    let state = if checked { "bg-primary" } else { "bg-input dark:bg-input/80" };
    let thumb = if checked { "translate-x-[calc(100%-2px)] dark:bg-primary-foreground" } else { "translate-x-0 dark:bg-foreground" };
    rsx! {
        label { class: "inline-flex cursor-pointer items-start gap-3 text-sm",
            button {
                r#type: "button",
                role: "switch",
                disabled,
                "aria-checked": "{checked}",
                class: "peer mt-0.5 inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 {state}",
                onclick: move |_| onchange.call(!checked),
                span { class: "pointer-events-none block size-4 rounded-full bg-background ring-0 transition-transform {thumb}" }
            }
            if label.is_some() || hint.is_some() {
                span { class: "grid gap-0.5",
                    if let Some(l) = label {
                        span { "{l}" }
                    }
                    if let Some(h) = hint {
                        span { class: "text-xs text-muted-foreground", "{h}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Checkbox(checked: bool, onchange: EventHandler<bool>, #[props(into)] label: String, #[props(into, optional)] hint: Option<String>) -> Element {
    let state = if checked { "bg-primary text-primary-foreground border-primary" } else { "dark:bg-input/30" };
    rsx! {
        label { class: "flex cursor-pointer items-start gap-2.5 text-sm",
            button {
                r#type: "button",
                role: "checkbox",
                "aria-checked": "{checked}",
                class: "mt-0.5 grid size-4 shrink-0 place-items-center rounded-[4px] border border-input shadow-xs outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 {state}",
                onclick: move |_| onchange.call(!checked),
                if checked {
                    icons::Check { class: "size-3.5" }
                }
            }
            span { class: "grid gap-0.5",
                span { "{label}" }
                if let Some(h) = hint {
                    span { class: "text-xs text-muted-foreground", "{h}" }
                }
            }
        }
    }
}

/// Auswahl in Segmenten (rust-ui Tabs-Optik), z. B. Schriftgrösse oder Zeitraum.
#[component]
pub fn Segmented(options: Vec<(String, String)>, value: String, onchange: EventHandler<String>, #[props(into, optional)] class: Option<String>) -> Element {
    let c = tw_merge!("inline-flex h-9 w-fit items-center rounded-lg bg-muted p-[3px] text-muted-foreground", class.as_deref().unwrap_or(""));
    rsx! {
        div { class: "{c}", role: "radiogroup",
            for (v , label) in options {
                button {
                    key: "{v}",
                    r#type: "button",
                    role: "radio",
                    "aria-checked": "{v == value}",
                    class: if v == value { "inline-flex h-full items-center justify-center rounded-md border border-input bg-input/30 px-3 text-sm font-medium text-foreground shadow-sm transition-all" } else { "inline-flex h-full items-center justify-center rounded-md border border-transparent px-3 text-sm font-medium text-foreground/60 transition-all hover:text-foreground" },
                    onclick: {
                        let v = v.clone();
                        move |_| onchange.call(v.clone())
                    },
                    "{label}"
                }
            }
        }
    }
}

/// Zähler-Abzeichen (ungelesene Artikel) in der Seitenleiste.
#[component]
pub fn Count(n: i64, #[props(default)] strong: bool) -> Element {
    if n <= 0 {
        return rsx! {};
    }
    let shown = if n > 9999 { "9999+".to_string() } else { n.to_string() };
    let c = if strong { "text-foreground" } else { "text-muted-foreground" };
    rsx! {
        span { class: "shrink-0 text-[11px] font-medium tabular-nums {c}", "{shown}" }
    }
}

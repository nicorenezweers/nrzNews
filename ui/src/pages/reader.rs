//! Lesemodus (rechte Spalte): Feed, Autor, Datum, Lesezeit, Titel, Merken, Gelesen,
//! Original, KI-Zusammenfassung über dem Inhalt. Der Inhalt kommt vom Server schon gesäubert
//! (ammonia) und wird darum direkt eingesetzt.

use crate::api;
use crate::components::common::*;
use crate::components::feed_icon::FeedIcon;
use crate::components::overlay::{toast, toast_error};
use crate::i18n::{tr, trf};
use crate::markdown;
use crate::pages::articles::{ArticleRef, Pane, Sel, close_reader, set_state};
use crate::state;
use crate::ui::kbd::Kbd;
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{AiSummary, Article, ArticleSummary};

/// Leerer Lesemodus (Desktop, kein Artikel gewählt).
#[component]
pub fn ReaderEmpty() -> Element {
    rsx! {
        div { class: "flex h-full w-full flex-col items-center justify-center gap-5 p-10 text-center animate-fade-in",
            img { src: asset!("/assets/icon.png"), class: "size-12 rounded-2xl opacity-40 grayscale", alt: "" }
            div {
                p { class: "font-medium text-foreground/80", {tr("Wähle links einen Artikel.")} }
                p { class: "mt-1 text-sm text-muted-foreground", {tr("Lies in Ruhe – ohne Werbung, ohne Ablenkung.")} }
            }
            div { class: "flex flex-wrap items-center justify-center gap-x-4 gap-y-2 text-xs text-muted-foreground",
                span { class: "inline-flex items-center gap-1.5", Kbd { "j" } Kbd { "k" } {tr("blättern")} }
                span { class: "inline-flex items-center gap-1.5", Kbd { "?" } {tr("alle Tastenkürzel")} }
            }
        }
    }
}

/// Ein Artikel zum Lesen. `preview` (aus der Liste) zeigt den Kopf sofort, der Inhalt lädt nach.
#[component]
pub fn Reader(id: i64, sel: Sel, preview: Option<ArticleSummary>) -> Element {
    let pane = use_context::<Pane>();
    let mut error = use_signal(|| None::<String>);
    // Zusammenfassung: Text (oder Fehler) und Modell
    let mut summary = use_signal(|| None::<Result<(String, String), String>>);
    let mut summarizing = use_signal(|| false);
    let mut show_summary = use_signal(|| false);
    // Gerade gezeigter Artikel – spät eintreffende Antworten für einen anderen werden verworfen
    let mut current = use_signal(|| id);
    let mut open = pane.open;

    // Laden (auch wenn derselbe Lesemodus einen anderen Artikel zeigen soll)
    use_effect(use_reactive!(|id| {
        current.set(id);
        error.set(None);
        summary.set(None);
        summarizing.set(false);
        show_summary.set(false);
        if open.peek().as_ref().is_some_and(|a| a.id != id) {
            open.set(None);
        }
        document::eval("document.getElementById('reader-scroll')?.scrollTo(0, 0)");
        spawn(async move {
            let res = api::get::<Article>(&format!("/articles/{id}")).await;
            if *current.peek() != id {
                return;
            }
            match res {
                Ok(a) => {
                    if let Some(s) = a.ai_summary.clone().filter(|s| !s.trim().is_empty()) {
                        summary.set(Some(Ok((s, String::new()))));
                        show_summary.set(true);
                    }
                    let from_server = ArticleRef { id, feed_id: a.feed_id, published_at: a.published_at, read: a.read, starred: a.starred };
                    open.set(Some(a));
                    if state::prefs().mark_read_on_open {
                        let r = pane.get(id).unwrap_or(from_server);
                        if !r.read {
                            set_state(Some(pane), r, Some(true), None);
                        }
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    }));

    let mut summarize = move |force: bool| {
        show_summary.set(true);
        if summarizing() {
            return;
        }
        summarizing.set(true);
        let id = *current.peek();
        spawn(async move {
            let path = if force { format!("/articles/{id}/summary?refresh=true") } else { format!("/articles/{id}/summary") };
            let res = api::action::<AiSummary>(&path).await;
            if *current.peek() != id {
                return;
            }
            match res {
                Ok(s) => summary.set(Some(Ok((s.summary, s.model)))),
                Err(e) => summary.set(Some(Err(e.message))),
            }
            summarizing.set(false);
        });
    };

    let article = pane.open.read().clone().filter(|a| a.id == id);
    let item = pane.items.read().iter().find(|a| a.id == id).cloned();
    let (read, starred) = item
        .as_ref()
        .map(|i| (i.read, i.starred))
        .or_else(|| article.as_ref().map(|a| (a.read, a.starred)))
        .or_else(|| preview.as_ref().map(|p| (p.read, p.starred)))
        .unwrap_or((false, false));
    let reference = article
        .as_ref()
        .map(|a| ArticleRef { id, feed_id: a.feed_id, published_at: a.published_at, read, starred })
        .or_else(|| preview.as_ref().map(|p| ArticleRef { id, feed_id: p.feed_id, published_at: p.published_at, read, starred }));

    // Kopf: aus dem geladenen Artikel, sonst aus der Liste
    let head = match (&article, &preview) {
        (Some(a), _) => Some((a.feed_id, a.feed_title.clone(), a.feed_icon.clone(), a.title.clone(), a.url.clone(), a.author.clone(), a.published_at)),
        (None, Some(p)) => Some((p.feed_id, p.feed_title.clone(), p.feed_icon.clone(), p.title.clone(), p.url.clone(), p.author.clone(), p.published_at)),
        _ => None,
    };
    let size = state::prefs().reader_size;
    let (prev_id, next_id) = {
        let items = pane.items.read();
        let pos = items.iter().position(|a| a.id == id);
        (
            pos.and_then(|i| i.checked_sub(1)).and_then(|i| items.get(i)).map(|a| a.id),
            pos.and_then(|i| items.get(i + 1)).map(|a| a.id),
        )
    };
    let go = {
        let sel = sel.clone();
        move |target: i64| navigator().replace(sel.route(Some(target)))
    };
    let go2 = go.clone();
    let mut toggle_summary = move || {
        if show_summary() {
            show_summary.set(false);
        } else if matches!(summary(), Some(Ok(_))) {
            show_summary.set(true);
        } else {
            summarize(false);
        }
    };
    let toggle_star = move || {
        if let Some(r) = reference {
            set_state(Some(pane), r, None, Some(!r.starred));
        }
    };
    let toggle_read = move || {
        if let Some(r) = reference {
            set_state(Some(pane), r, Some(!r.read), None);
        }
    };
    let share_data = head.as_ref().map(|h| (h.3.clone(), h.4.clone()));
    let original = head.as_ref().map(|h| h.4.clone());
    let close = {
        let sel = sel.clone();
        move |_| close_reader(&sel)
    };
    let close2 = close.clone();

    rsx! {
        div { class: "flex h-full min-h-0 w-full flex-col",
            // Werkzeugleiste
            div { class: "relative flex h-14 shrink-0 items-center gap-1 border-b px-2 sm:px-3",
                button { class: "{ICON_BTN} size-10 md:hidden", title: tr("Zurück zur Liste"), onclick: close, icons::ArrowLeft { class: "size-5" } }
                button { class: icon_btn("hidden md:inline-grid"), title: tr("Schliessen (Esc)"), onclick: close2, icons::X {} }
                // Handy: Feed als Titel, daneben vorheriger/nächster Artikel
                if let Some((_, feed_title, feed_icon, ..)) = head.clone() {
                    div { class: "flex min-w-0 flex-1 items-center gap-2 px-1 text-sm font-medium text-foreground/80 md:hidden",
                        FeedIcon { src: feed_icon, title: feed_title.clone() }
                        span { class: "truncate", "{feed_title}" }
                    }
                } else {
                    div { class: "flex-1 md:hidden" }
                }
                div { class: "flex items-center md:hidden",
                    button { class: "{ICON_BTN} size-10", title: tr("Vorheriger Artikel"), disabled: prev_id.is_none(),
                        onclick: move |_| if let Some(p) = prev_id { go(p); },
                        icons::ChevronUp { class: "size-5" }
                    }
                    button { class: "{ICON_BTN} size-10", title: tr("Nächster Artikel"), disabled: next_id.is_none(),
                        onclick: move |_| if let Some(n) = next_id { go2(n); },
                        icons::ChevronDown { class: "size-5" }
                    }
                }
                // Desktop: alle Aktionen oben
                div { class: "hidden flex-1 items-center justify-end gap-1 md:flex",
                    button {
                        class: if show_summary() { "{BTN} {BTN_SM} bg-accent text-foreground" } else { "{BTN} {BTN_SM} text-muted-foreground hover:bg-accent hover:text-foreground" },
                        title: tr("KI-Zusammenfassung"),
                        disabled: article.is_none(),
                        onclick: move |_| toggle_summary(),
                        if summarizing() {
                            icons::LoaderCircle { class: "size-4 animate-spin" }
                        } else {
                            icons::Sparkles { class: "size-4" }
                        }
                        span { class: "hidden sm:inline", {tr("Zusammenfassen")} }
                    }
                    button {
                        class: ICON_BTN,
                        title: if starred { tr("Nicht mehr merken (s)") } else { tr("Merken (s)") },
                        disabled: reference.is_none(),
                        onclick: move |_| toggle_star(),
                        span { class: if starred { "grid text-foreground [&_svg]:fill-current" } else { "grid" }, icons::Star { class: "size-4" } }
                    }
                    button {
                        class: ICON_BTN,
                        title: if read { tr("Als ungelesen markieren (m)") } else { tr("Als gelesen markieren (m)") },
                        disabled: reference.is_none(),
                        onclick: move |_| toggle_read(),
                        if read {
                            icons::Circle { class: "size-4" }
                        } else {
                            icons::CircleDot { class: "size-4 text-foreground" }
                        }
                    }
                    if let Some((_, _, _, _, url, _, _)) = head.clone() {
                        a { class: ICON_BTN, href: "{url}", target: "_blank", rel: "noopener noreferrer", title: tr("Original öffnen (o)"),
                            icons::ExternalLink {}
                        }
                    }
                }
                // Lesefortschritt (gestures.rs)
                div { id: "reader-progress", class: "pointer-events-none absolute bottom-[-1px] left-0 h-0.5 w-0 bg-foreground/60" }
            }

            // Inhalt
            div { id: "reader-scroll", class: "min-h-0 flex-1 overflow-y-auto overscroll-contain",
                if let Some(e) = error() {
                    div { class: "mx-auto max-w-[68ch] px-5 py-10",
                        Callout { text: e, error: true }
                    }
                } else if let Some((feed_id, feed_title, feed_icon, title, url, author, published)) = head {
                    article { id: "reader-swipe", class: "mx-auto w-full max-w-[68ch] px-5 pt-6 pb-16 sm:px-8 sm:pt-12 md:pb-24 animate-fade-in",
                        // Feed und Autor
                        div { class: "flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground",
                            Link { to: Route::FeedView { id: feed_id, article: None }, class: "inline-flex min-w-0 items-center gap-2 font-medium text-foreground/80 hover:text-foreground",
                                FeedIcon { src: feed_icon, title: feed_title.clone() }
                                span { class: "truncate", "{feed_title}" }
                            }
                            if !author.trim().is_empty() {
                                span { class: "opacity-50", "·" }
                                span { class: "truncate", "{author}" }
                            }
                        }
                        // Titel
                        h1 { class: "mt-4 text-[1.65rem] leading-tight font-semibold tracking-tight text-balance sm:text-[2rem]",
                            a { href: "{url}", target: "_blank", rel: "noopener noreferrer", class: "hover:underline hover:decoration-foreground/30 hover:underline-offset-4",
                                if title.trim().is_empty() { {tr("(ohne Titel)")} } else { "{title}" }
                            }
                        }
                        // Datum und Lesezeit
                        div { class: "mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground",
                            span { class: "inline-flex items-center gap-1.5", title: "{util::datetime(published)}",
                                icons::Calendar { class: "size-3.5" }
                                "{util::long_datetime(published)}"
                            }
                            if let Some(a) = article.as_ref() {
                                span { class: "inline-flex items-center gap-1.5",
                                    icons::Clock { class: "size-3.5" }
                                    {trf("{0} Min. Lesezeit", &[&a.reading_minutes.max(1)])}
                                }
                            }
                        }

                        // KI-Zusammenfassung
                        if show_summary() {
                            SummaryBox { summary: summary(), busy: summarizing(),
                                on_refresh: move |_| summarize(true),
                                on_close: move |_| show_summary.set(false),
                            }
                        }

                        div { class: "my-8 h-px bg-border" }

                        match article.as_ref() {
                            None => rsx! { ContentSkeleton {} },
                            Some(a) if a.content_html.trim().is_empty() => rsx! {
                                div { class: "prose-reader", "data-size": "{size}",
                                    if let Some(p) = preview.as_ref().filter(|p| !p.excerpt.trim().is_empty()) {
                                        p { "{p.excerpt}" }
                                    }
                                    p { class: "text-muted-foreground", {tr("Dieser Feed liefert keinen Inhalt – der ganze Artikel steht auf der Webseite.")} }
                                }
                            },
                            Some(a) => rsx! {
                                if !a.image_url.is_empty() && !a.content_html.contains("<img") {
                                    img { class: "mb-8 w-full rounded-xl bg-muted object-cover", src: "{a.image_url}", alt: "", referrerpolicy: "no-referrer" }
                                }
                                div { class: "prose-reader", "data-size": "{size}", dangerous_inner_html: "{a.content_html}" }
                            },
                        }

                        // Fuss: Original und nächster Artikel
                        if article.is_some() {
                            div { class: "mt-12 flex flex-wrap items-center gap-3 border-t pt-6",
                                a { class: btn(BTN_OUTLINE), href: "{url}", target: "_blank", rel: "noopener noreferrer",
                                    icons::ExternalLink { class: "size-4" }
                                    {tr("Original lesen")}
                                    span { class: "text-muted-foreground", "{util::host(&url)}" }
                                }
                                if let Some(next) = next_id {
                                    Link { class: "{btn(BTN_GHOST)} ml-auto", to: sel.route(Some(next)),
                                        {tr("Nächster Artikel")}
                                        icons::ChevronRight { class: "size-4" }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    div { class: "mx-auto w-full max-w-[68ch] px-5 pt-12 sm:px-8",
                        div { class: "h-4 w-40 animate-pulse rounded bg-muted" }
                        div { class: "mt-5 h-8 w-4/5 animate-pulse rounded bg-muted" }
                        div { class: "mt-3 h-3.5 w-56 animate-pulse rounded bg-muted" }
                        div { class: "my-8 h-px bg-border" }
                        ContentSkeleton {}
                    }
                }
            }

            // Handy: Aktionen unten, mit dem Daumen erreichbar
            nav { class: "safe-bottom shrink-0 border-t bg-background md:hidden",
                div { class: "grid h-14 grid-cols-5",
                    BarButton { label: tr("Kurzfassung"), active: show_summary(), disabled: article.is_none(), onclick: move |_| toggle_summary(),
                        if summarizing() { icons::LoaderCircle { class: "animate-spin" } } else { icons::Sparkles {} }
                    }
                    BarButton { label: if starred { tr("Gemerkt") } else { tr("Merken") }, active: starred, disabled: reference.is_none(), onclick: move |_| toggle_star(),
                        span { class: if starred { "grid [&_svg]:fill-current" } else { "grid" }, icons::Star {} }
                    }
                    BarButton { label: if read { tr("Gelesen") } else { tr("Ungelesen") }, active: !read, disabled: reference.is_none(), onclick: move |_| toggle_read(),
                        if read { icons::Circle {} } else { icons::CircleDot {} }
                    }
                    BarButton { label: tr("Teilen"), disabled: share_data.is_none(),
                        onclick: move |_| if let Some((title, url)) = share_data.clone() { share(&title, &url) },
                        icons::Share {}
                    }
                    if let Some(url) = original {
                        a { class: BAR_BTN, href: "{url}", target: "_blank", rel: "noopener noreferrer",
                            icons::ExternalLink {}
                            span { {tr("Original")} }
                        }
                    } else {
                        span { class: "{BAR_BTN} opacity-50", icons::ExternalLink {} span { {tr("Original")} } }
                    }
                }
            }
            // Hinweise beim Wischen (gestures.rs)
            if prev_id.is_some() {
                div { id: "swipe-hint-prev", class: "swipe-hint swipe-hint-prev md:hidden", icons::ChevronLeft { class: "size-5" } }
            }
            if next_id.is_some() {
                div { id: "swipe-hint-next", class: "swipe-hint swipe-hint-next md:hidden", icons::ChevronRight { class: "size-5" } }
            }
        }
    }
}

const BAR_BTN: &str = "flex flex-col items-center justify-center gap-1 text-[10.5px] font-medium text-muted-foreground transition-colors active:scale-95 disabled:opacity-50 [&_svg]:size-[1.35rem]";

/// Knopf der unteren Leiste (Handy).
#[component]
fn BarButton(#[props(into)] label: String, #[props(default)] active: bool, #[props(default)] disabled: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    let c = if active { "text-foreground" } else { "" };
    rsx! {
        button { class: "{BAR_BTN} {c}", title: "{label}", disabled, onclick: move |e| onclick.call(e),
            {children}
            span { class: "max-w-full truncate px-0.5", "{label}" }
        }
    }
}

/// Teilen-Menü des Handys; ohne (z. B. am Desktop) den Link kopieren. Läuft direkt im Klick,
/// sonst verweigert der Browser das Teilen.
fn share(title: &str, url: &str) {
    let js = format!(
        "const title = {}, url = {};\
         try {{\
           if (navigator.share) {{ await navigator.share({{ title, url }}); return 'shared'; }}\
           await navigator.clipboard.writeText(url); return 'copied';\
         }} catch (e) {{ return e && e.name === 'AbortError' ? 'aborted' : 'error'; }}",
        serde_json::to_string(title).unwrap_or_default(),
        serde_json::to_string(url).unwrap_or_default(),
    );
    let eval = document::eval(&js);
    spawn(async move {
        match eval.join::<String>().await.as_deref() {
            Ok("copied") => toast(tr("Link kopiert.")),
            Ok("error") | Err(_) => toast_error(tr("Teilen nicht möglich.")),
            _ => {}
        }
    });
}

#[component]
fn ContentSkeleton() -> Element {
    rsx! {
        div { class: "space-y-3",
            for w in ["w-full", "w-11/12", "w-full", "w-4/5", "w-0", "w-full", "w-10/12", "w-3/5"] {
                div { class: if w == "w-0" { "h-4" } else { "h-3.5 animate-pulse rounded bg-muted {w}" } }
            }
        }
    }
}

/// Kasten mit der KI-Zusammenfassung (Markdown, nie als HTML).
#[component]
fn SummaryBox(summary: Option<Result<(String, String), String>>, busy: bool, on_refresh: EventHandler<()>, on_close: EventHandler<()>) -> Element {
    let admin = state::is_admin();
    rsx! {
        section { class: "mt-7 rounded-xl border bg-card/60 shadow-sm animate-fade-in",
            div { class: "flex items-center gap-2 border-b px-4 py-2.5",
                icons::Sparkles { class: "size-4 text-muted-foreground" }
                span { class: "text-sm font-medium", {tr("Zusammenfassung")} }
                if let Some(Ok((_, model))) = summary.as_ref() {
                    if !model.is_empty() {
                        span { class: "hidden truncate font-mono text-[11px] text-muted-foreground sm:inline", "{model}" }
                    }
                }
                div { class: "flex-1" }
                button { class: icon_btn("size-7"), title: tr("Neu zusammenfassen"), disabled: busy, onclick: move |_| on_refresh.call(()),
                    span { class: if busy { "grid animate-spin" } else { "grid" }, icons::RefreshCw { class: "size-3.5" } }
                }
                button { class: icon_btn("size-7"), title: tr("Ausblenden"), onclick: move |_| on_close.call(()),
                    icons::X { class: "size-3.5" }
                }
            }
            div { class: "px-4 py-4 text-[0.94rem]",
                match (summary, busy) {
                    (_, true) => rsx! {
                        div { class: "space-y-2.5",
                            p { class: "flex items-center gap-2 text-sm text-muted-foreground",
                                icons::LoaderCircle { class: "size-4 animate-spin" }
                                {tr("Die KI liest den Artikel …")}
                            }
                            div { class: "h-3 w-full animate-pulse rounded bg-muted" }
                            div { class: "h-3 w-5/6 animate-pulse rounded bg-muted" }
                            div { class: "h-3 w-2/3 animate-pulse rounded bg-muted" }
                        }
                    },
                    (Some(Ok((text, _))), false) => rsx! {
                        div { class: "prose-ai text-foreground/90", {markdown::render(&text, &[])} }
                    },
                    (Some(Err(e)), false) => rsx! {
                        div { class: "grid gap-2 text-sm",
                            p { class: "flex items-start gap-2", icons::CircleAlert { class: "mt-0.5 size-4 shrink-0 text-destructive" } "{e}" }
                            if admin {
                                Link { class: "justify-self-start text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline", to: Route::Settings { section: "ai".into() }, {tr("KI-Einstellungen öffnen")} }
                            } else {
                                p { class: "text-xs text-muted-foreground", {tr("Ein Administrator richtet die KI unter Einstellungen → KI & Modelle ein.")} }
                            }
                        }
                    },
                    (None, false) => rsx! {
                        p { class: "text-sm text-muted-foreground", {tr("Noch keine Zusammenfassung.")} }
                    },
                }
            }
        }
    }
}

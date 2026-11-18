//! Dieter, der Assistent: Chat unten rechts. Fragen zu den eigenen Feeds („Was gibt es Neues
//! zu Rust?“, „Fasse die Woche zusammen“) – Dieter sucht in den Artikeln und antwortet mit
//! Verweisen [1], [2] auf die Artikel, auf die er sich stützt.

use crate::api;
use crate::i18n::{tr, trf};
use crate::markdown;
use crate::state;
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{AssistantAnswer, AssistantInput, ChatMessage, Source};

#[derive(Debug, Clone, PartialEq, Default)]
struct Msg {
    role: String,
    content: String,
    sources: Vec<Source>,
    error: bool,
}

/// Gespräch – bleibt beim Seitenwechsel erhalten.
static CHAT: GlobalSignal<Vec<Msg>> = Signal::global(Vec::new);
static OPEN: GlobalSignal<bool> = Signal::global(|| false);

#[component]
pub fn Assistant() -> Element {
    let mut input = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let messages = CHAT.read().clone();
    let admin = state::is_admin();

    let send = use_callback(move |text: String| {
        let text = text.trim().to_string();
        if text.is_empty() || busy() {
            return;
        }
        CHAT.write().push(Msg { role: "user".into(), content: text, ..Default::default() });
        input.set(String::new());
        busy.set(true);
        document::eval("requestAnimationFrame(() => { const c = document.getElementById('dieter-log'); if (c) c.scrollTop = c.scrollHeight; })");
        spawn(async move {
            let history: Vec<ChatMessage> = CHAT
                .read()
                .iter()
                .filter(|m| !m.error)
                .map(|m| ChatMessage { role: m.role.clone(), content: m.content.clone() })
                .collect();
            let msg = match api::post::<AssistantAnswer, _>("/assistant", &AssistantInput { messages: history }).await {
                Ok(a) => Msg { role: "assistant".into(), content: a.answer, sources: a.sources, error: false },
                Err(e) => Msg { role: "assistant".into(), content: e.message, sources: Vec::new(), error: true },
            };
            CHAT.write().push(msg);
            busy.set(false);
            // ans Ende scrollen
            document::eval("requestAnimationFrame(() => { const c = document.getElementById('dieter-log'); if (c) c.scrollTop = c.scrollHeight; })");
        });
    });
    let suggestions = [tr("Was gibt es heute Neues?"), tr("Fasse die wichtigsten Themen der Woche zusammen."), tr("Welche Artikel habe ich mir gemerkt?")];

    // Auf dem Handy beim Lesen kein schwebender Knopf über dem Text
    let reading = crate::pages::layout::open_article(&use_route::<crate::Route>()).is_some();
    let fab_vis = if reading { "hidden md:flex" } else { "flex" };

    rsx! {
        if !OPEN() {
            button {
                class: "{fab_vis} above-tabbar fixed right-4 z-40 items-center gap-2 rounded-full border bg-primary px-3.5 py-3 text-sm font-medium text-primary-foreground shadow-lg transition hover:scale-[1.03] active:scale-95 animate-in sm:right-5 sm:px-4",
                title: tr("Dieter fragen"),
                onclick: move |_| {
                    *OPEN.write() = true;
                    document::eval("setTimeout(() => document.getElementById('dieter-input')?.focus(), 50)");
                },
                icons::MessageCircle { class: "size-5" }
                span { class: "hidden sm:inline", {tr("Dieter fragen")} }
            }
        } else {
            div {
                class: "fixed inset-x-2 bottom-[calc(0.5rem+env(safe-area-inset-bottom))] z-40 flex h-[min(36rem,85dvh)] flex-col overflow-hidden rounded-2xl border bg-popover text-popover-foreground shadow-2xl animate-toast-in sm:inset-x-auto sm:right-5 sm:bottom-5 sm:w-[25rem]",
                role: "dialog",
                onkeydown: move |e| {
                    if e.key() == Key::Escape {
                        *OPEN.write() = false;
                    }
                },
                // Kopf
                div { class: "flex items-center gap-3 border-b px-4 py-3",
                    span { class: "grid size-9 place-items-center rounded-full bg-primary text-sm font-bold text-primary-foreground", "D" }
                    div { class: "min-w-0 flex-1",
                        p { class: "font-semibold leading-tight", "Dieter" }
                        p { class: "text-xs text-muted-foreground", {tr("Dein Assistent für die Feeds")} }
                    }
                    if !messages.is_empty() {
                        button {
                            class: "grid size-8 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground",
                            title: tr("Neues Gespräch"),
                            onclick: move |_| CHAT.write().clear(),
                            icons::SquarePen { class: "size-4" }
                        }
                    }
                    button {
                        class: "grid size-8 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground",
                        title: tr("Schliessen"),
                        onclick: move |_| *OPEN.write() = false,
                        icons::X { class: "size-4" }
                    }
                }
                // Verlauf
                div { id: "dieter-log", class: "min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4 text-sm",
                    if messages.is_empty() {
                        div { class: "rounded-xl bg-accent/50 p-3",
                            p { {tr("Hallo, ich bin Dieter. Frag mich nach deinen Feeds – ich suche in den Artikeln und zeige dir die Quellen.")} }
                        }
                        div { class: "grid gap-1.5",
                            for s in suggestions.iter().cloned() {
                                button {
                                    class: "rounded-lg border px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent hover:text-foreground",
                                    onclick: move |_| send.call(s.clone()),
                                    "{s}"
                                }
                            }
                        }
                    }
                    for (i , m) in messages.iter().enumerate() {
                        if m.role == "user" {
                            div { key: "{i}", class: "ml-8 rounded-2xl rounded-br-md bg-primary px-3 py-2 whitespace-pre-wrap text-primary-foreground", "{m.content}" }
                        } else {
                            div { key: "{i}", class: "mr-4 grid gap-2",
                                div { class: if m.error { "rounded-2xl rounded-bl-md border border-destructive/50 bg-destructive/10 px-3 py-2" } else { "prose-ai rounded-2xl rounded-bl-md bg-accent px-3 py-2" },
                                    if m.error {
                                        p { "{m.content}" }
                                        if admin {
                                            Link {
                                                class: "mt-1 inline-flex items-center gap-1 text-xs underline-offset-4 hover:underline",
                                                to: Route::Settings { section: "ai".into() },
                                                onclick: move |_| *OPEN.write() = false,
                                                {tr("KI-Einstellungen öffnen")}
                                            }
                                        } else {
                                            p { class: "mt-1 text-xs text-muted-foreground", {tr("Für Dieter braucht es eine KI – ein Administrator richtet sie unter Einstellungen → KI & Modelle ein.")} }
                                        }
                                    } else {
                                        {markdown::render(&m.content, &m.sources.iter().map(|s| s.article_id).collect::<Vec<_>>())}
                                    }
                                }
                                if !m.sources.is_empty() {
                                    div { class: "grid gap-1",
                                        p { class: "text-[11px] uppercase tracking-wider text-muted-foreground",
                                            if m.sources.len() == 1 { {tr("Quelle")} } else { {trf("{0} Quellen", &[&m.sources.len()])} }
                                        }
                                        for (n , s) in m.sources.iter().cloned().enumerate() {
                                            Link {
                                                key: "{s.article_id}",
                                                to: Route::All { article: Some(s.article_id) },
                                                class: "flex items-center gap-2 rounded-lg border bg-card px-2.5 py-1.5 text-xs hover:bg-accent",
                                                span { class: "grid h-4 min-w-4 place-items-center rounded bg-muted px-1 text-[10px] font-semibold tabular-nums text-muted-foreground", "{n + 1}" }
                                                span { class: "min-w-0 flex-1",
                                                    span { class: "block truncate", "{s.title}" }
                                                    span { class: "block truncate text-muted-foreground", "{s.feed_title} · {util::relative(s.published_at)}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if busy() {
                        div { class: "flex items-center gap-2 text-xs text-muted-foreground",
                            icons::LoaderCircle { class: "size-4 animate-spin" }
                            {tr("Dieter liest deine Artikel …")}
                        }
                    }
                }
                // Eingabe
                form {
                    class: "flex items-end gap-2 border-t p-3",
                    onsubmit: move |e| {
                        e.prevent_default();
                        send.call(input());
                    },
                    textarea {
                        id: "dieter-input",
                        class: "max-h-32 min-h-10 flex-1 resize-none rounded-lg border border-input bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30",
                        rows: 1,
                        placeholder: tr("Frag Dieter …"),
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !e.modifiers().shift() {
                                e.prevent_default();
                                send.call(input());
                            }
                        },
                    }
                    button {
                        class: "grid size-10 shrink-0 place-items-center rounded-lg bg-primary text-primary-foreground disabled:opacity-50",
                        r#type: "submit",
                        disabled: busy() || input().trim().is_empty(),
                        title: tr("Senden"),
                        icons::SendHorizontal { class: "size-4" }
                    }
                }
            }
        }
    }
}

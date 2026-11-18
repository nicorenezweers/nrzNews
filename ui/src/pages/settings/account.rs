//! Eigenes Konto: Anzeigename, Passwort und API-Tokens für den MCP-Server (z. B. nrzDashboard).

use crate::api;
use crate::components::common::*;
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use crate::state::ME;
use crate::util;
use dioxus::prelude::*;
use nrznews_types::{ApiToken, ApiTokenCreated, ApiTokenInput, ChangePasswordInput, MIN_PASSWORD_LEN, UpdateMeInput, User};

#[component]
pub fn Account() -> Element {
    let me = ME.read().clone();
    let mut name = use_signal(|| me.as_ref().map(|m| m.display_name.clone()).unwrap_or_default());
    let mut saving = use_signal(|| false);
    let mut current = use_signal(String::new);
    let mut new = use_signal(String::new);
    let mut repeat = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let Some(me) = me else { return rsx! { Loading {} } };
    let changed = name().trim() != me.display_name.trim();
    rsx! {
        Section { title: tr("Profil"), description: tr("Dein Konto in dieser nrzNews-Instanz."),
            div { class: "grid max-w-xl gap-5",
                dl { class: "grid grid-cols-[8rem_1fr] gap-y-2 text-sm",
                    dt { class: "text-muted-foreground", {tr("Benutzername")} }
                    dd { class: "font-medium", "{me.username}" }
                    dt { class: "text-muted-foreground", {tr("Rolle")} }
                    dd { if me.is_admin() { {tr("Administrator")} } else { {tr("Mitglied")} } }
                    dt { class: "text-muted-foreground", {tr("Dabei seit")} }
                    dd { "{util::date(me.created_at)}" }
                }
                form { class: "grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end",
                    onsubmit: move |e| {
                        e.prevent_default();
                        spawn(async move {
                            saving.set(true);
                            let res = api::put::<User, _>("/me", &UpdateMeInput { display_name: Some(name().trim().to_string()), preferences: None }).await;
                            saving.set(false);
                            match res {
                                Ok(u) => {
                                    *ME.write() = Some(u);
                                    toast(tr("Anzeigename gespeichert."));
                                }
                                Err(e) => toast_error(e),
                            }
                        });
                    },
                    Field { label: tr("Anzeigename"), hint: tr("Leer = Benutzername"),
                        input { class: INPUT, maxlength: 100, autocomplete: "name", value: "{name}", oninput: move |e| name.set(e.value()) }
                    }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: saving() || !changed, {tr("Speichern")} }
                }
            }
        }
        Section { title: tr("Passwort ändern"), description: tr("Danach sind alle anderen Geräte abgemeldet."),
            form { class: "grid max-w-3xl gap-4 sm:grid-cols-3 sm:items-end",
                onsubmit: move |e| {
                    e.prevent_default();
                    if new() != repeat() {
                        toast_error(tr("Die Passwörter stimmen nicht überein."));
                        return;
                    }
                    spawn(async move {
                        busy.set(true);
                        let res = api::put::<serde_json::Value, _>("/me/password", &ChangePasswordInput { current: current(), new: new() }).await;
                        busy.set(false);
                        match res {
                            Ok(_) => {
                                toast(tr("Passwort geändert. Andere Geräte sind abgemeldet."));
                                current.set(String::new());
                                new.set(String::new());
                                repeat.set(String::new());
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Aktuelles Passwort"),
                    input { class: INPUT, r#type: "password", autocomplete: "current-password", required: true, value: "{current}", oninput: move |e| current.set(e.value()) }
                }
                Field { label: tr("Neues Passwort"), hint: trf("Mindestens {0} Zeichen", &[&MIN_PASSWORD_LEN]),
                    input { class: INPUT, r#type: "password", autocomplete: "new-password", minlength: MIN_PASSWORD_LEN as i64, required: true, value: "{new}", oninput: move |e| new.set(e.value()) }
                }
                Field { label: tr("Wiederholen"),
                    input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, value: "{repeat}", oninput: move |e| repeat.set(e.value()) }
                }
                div { class: "sm:col-span-3",
                    button { class: btn(BTN_PRIMARY), disabled: busy(), r#type: "submit", {tr("Passwort ändern")} }
                }
            }
        }
        ApiTokens {}
    }
}

/// Adresse des MCP-Servers, wie der Browser die Instanz sieht.
fn mcp_url() -> String {
    let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
    format!("{origin}/mcp")
}

fn copy(text: &str) {
    let js = format!("navigator.clipboard && navigator.clipboard.writeText({})", serde_json::to_string(text).unwrap_or_default());
    document::eval(&js);
    toast(tr("Kopiert."));
}

/// Persönliche API-Tokens: Zugang zum MCP-Server für nrzDashboard und KI-Agenten.
#[component]
fn ApiTokens() -> Element {
    let mut tokens = use_resource(|| async { api::get::<Vec<ApiToken>>("/me/tokens").await });
    let mut name = use_signal(|| "nrzDashboard".to_string());
    let mut creating = use_signal(|| false);
    let mut created = use_signal(|| None::<ApiTokenCreated>);
    let url = mcp_url();
    let url_copy = url.clone();
    rsx! {
        Section { title: tr("API-Tokens (MCP)"),
            description: tr("Für nrzDashboard und KI-Agenten: Der MCP-Server liefert Kennzahlen für den Überblick und liest deine Artikel – ändern kann er nichts. Jedes Token sieht nur dein Konto."),
            div { class: "grid max-w-3xl gap-5",
                div { class: "grid gap-1.5",
                    span { class: "text-sm font-medium", {tr("Adresse des MCP-Servers")} }
                    div { class: "flex items-center gap-2",
                        code { class: "min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-3 py-2 font-mono text-[13px]", "{url}" }
                        button { class: btn_sm(BTN_OUTLINE), r#type: "button", onclick: move |_| copy(&url_copy), icons::Copy { class: "size-4" } {tr("Kopieren")} }
                    }
                    span { class: "text-xs text-muted-foreground", {tr("Anmeldung mit dem Header „Authorization: Bearer <Token>“. In nrzDashboard: System → nrzNews.")} }
                }

                if let Some(c) = created() {
                    div { class: "grid gap-2 rounded-lg border border-success/40 bg-success/10 p-4",
                        p { class: "text-sm font-medium", {trf("Token „{0}“ angelegt – jetzt kopieren, es wird nur einmal angezeigt.", &[&c.token.name])} }
                        div { class: "flex items-center gap-2",
                            code { class: "min-w-0 flex-1 break-all rounded-md border bg-background px-3 py-2 font-mono text-[13px] select-all", "{c.secret}" }
                            button { class: btn_sm(BTN_PRIMARY), r#type: "button", onclick: { let s = c.secret.clone(); move |_| copy(&s) }, icons::Copy { class: "size-4" } {tr("Kopieren")} }
                        }
                        button { class: "{btn_sm(BTN_GHOST)} justify-self-start", r#type: "button", onclick: move |_| created.set(None), {tr("Fertig")} }
                    }
                }

                form { class: "grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end",
                    onsubmit: move |e| {
                        e.prevent_default();
                        spawn(async move {
                            creating.set(true);
                            let res = api::post::<ApiTokenCreated, _>("/me/tokens", &ApiTokenInput { name: name().trim().to_string() }).await;
                            creating.set(false);
                            match res {
                                Ok(c) => {
                                    created.set(Some(c));
                                    tokens.restart();
                                }
                                Err(e) => toast_error(e),
                            }
                        });
                    },
                    Field { label: tr("Name des neuen Tokens"), hint: tr("Wofür ist es? z. B. nrzDashboard"),
                        input { class: INPUT, maxlength: 60, required: true, value: "{name}", oninput: move |e| name.set(e.value()) }
                    }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: creating() || name().trim().is_empty(),
                        icons::Plus { class: "size-4" }
                        {tr("Token anlegen")}
                    }
                }

                match &*tokens.read() {
                    None => rsx! { Loading {} },
                    Some(Err(e)) => rsx! { Callout { text: e.message.clone(), error: true } },
                    Some(Ok(list)) if list.is_empty() => rsx! {
                        p { class: "text-sm text-muted-foreground", {tr("Noch keine Tokens.")} }
                    },
                    Some(Ok(list)) => rsx! {
                        ul { class: "divide-y rounded-lg border",
                            for t in list.iter().cloned() {
                                li { key: "{t.id}", class: "flex items-center gap-3 px-4 py-3",
                                    icons::KeyRound { class: "size-4 shrink-0 text-muted-foreground" }
                                    div { class: "min-w-0 flex-1",
                                        p { class: "truncate text-sm font-medium", "{t.name}" }
                                        p { class: "truncate text-xs text-muted-foreground",
                                            span { class: "font-mono", "{t.prefix}…" }
                                            " · "
                                            {trf("angelegt {0}", &[&util::date(t.created_at)])}
                                            " · "
                                            match t.last_used_at {
                                                Some(ts) => trf("zuletzt benutzt {0}", &[&util::relative(ts)]),
                                                None => tr("noch nie benutzt"),
                                            }
                                        }
                                    }
                                    button { class: ICON_BTN, title: tr("Token löschen"),
                                        onclick: {
                                            let t = t.clone();
                                            move |_| {
                                                let t = t.clone();
                                                spawn(async move {
                                                    let ask = Ask::new(tr("Token löschen?"), trf("Programme, die „{0}“ benutzen, verlieren den Zugang sofort.", &[&t.name]))
                                                        .confirm(tr("Löschen"))
                                                        .destructive();
                                                    if !confirm(ask).await {
                                                        return;
                                                    }
                                                    match api::delete(&format!("/me/tokens/{}", t.id)).await {
                                                        Ok(()) => {
                                                            toast(tr("Token gelöscht."));
                                                            tokens.restart();
                                                        }
                                                        Err(e) => toast_error(e),
                                                    }
                                                });
                                            }
                                        },
                                        icons::Trash2 {}
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

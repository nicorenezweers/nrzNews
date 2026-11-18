//! Anmelden, Ersteinrichtung (erstes Konto wird Administrator) und Registrierung.

use crate::api;
use crate::components::common::*;
use crate::i18n::{tr, trf};
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{AuthStatus, LoginInput, MIN_PASSWORD_LEN, RegisterInput, User};

/// Gemeinsamer Rahmen der Anmeldeseiten: Logo, Karte, Fusszeile.
#[component]
fn AuthCard(#[props(into)] title: String, #[props(into)] subtitle: String, children: Element) -> Element {
    rsx! {
        div { class: "flex min-h-dvh items-center justify-center bg-background p-4",
            div { class: "w-full max-w-sm animate-fade-in",
                div { class: "mb-6 flex flex-col items-center gap-3 text-center",
                    img { src: asset!("/assets/icon.png"), class: "size-12 rounded-xl", alt: "" }
                    div {
                        h1 { class: "text-xl font-semibold tracking-tight", "{title}" }
                        p { class: "mt-1 text-sm text-muted-foreground", "{subtitle}" }
                    }
                }
                div { class: "rounded-xl border bg-card p-6 shadow-sm", {children} }
                p { class: "mt-6 text-center text-xs text-muted-foreground", {tr("nrzNews – deine Feeds, ruhig gelesen")} }
                div { class: "mt-3 flex justify-center", crate::components::language::LanguageSelect {} }
            }
        }
    }
}

/// Nur Ziele innerhalb der App (keine fremden Adressen über `?next=`).
fn safe_next(next: &str) -> String {
    if next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') && !next.starts_with("/login") { next.to_string() } else { "/".into() }
}

/// Ganze Seite neu laden, damit alles frisch startet.
fn go(target: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.location().assign(target);
    }
}

#[component]
pub fn Login(next: String) -> Element {
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let status = use_resource(|| api::get::<AuthStatus>("/auth/status"));
    let nav = use_navigator();
    let target = safe_next(&next);
    if let Some(Ok(s)) = &*status.read() {
        if s.setup_required {
            nav.replace(Route::Setup {});
        } else if s.authenticated {
            go(&target);
        }
    }
    let can_register = status.read().as_ref().and_then(|r| r.as_ref().ok()).is_some_and(|s| s.registration_open);
    let submit = move |e: FormEvent| {
        e.prevent_default();
        let target = target.clone();
        spawn(async move {
            busy.set(true);
            error.set(None);
            let res = api::post::<User, _>("/auth/login", &LoginInput { username: username().trim().to_string(), password: password() }).await;
            match res {
                Ok(user) => {
                    crate::i18n::set_language(&user.preferences.language, true).await;
                    go(&target);
                }
                Err(e) => {
                    busy.set(false);
                    error.set(Some(if e.status == 401 { tr("Benutzername oder Passwort stimmt nicht.") } else { e.message }));
                }
            }
        });
    };
    rsx! {
        AuthCard { title: tr("Anmelden"), subtitle: tr("Willkommen zurück bei nrzNews"),
            form { class: "grid gap-4", onsubmit: submit,
                if let Some(e) = error() {
                    Callout { text: e, error: true }
                }
                Field { label: tr("Benutzername"),
                    input { class: INPUT, autocomplete: "username", autocapitalize: "none", autofocus: true, required: true, value: "{username}", oninput: move |e| username.set(e.value()) }
                }
                Field { label: tr("Passwort"),
                    input { class: INPUT, r#type: "password", autocomplete: "current-password", required: true, value: "{password}", oninput: move |e| password.set(e.value()) }
                }
                button { class: "{btn(BTN_PRIMARY)} w-full", disabled: busy(), r#type: "submit",
                    if busy() {
                        icons::LoaderCircle { class: "size-4 animate-spin" }
                    }
                    {tr("Anmelden")}
                }
                if can_register {
                    p { class: "text-center text-sm text-muted-foreground",
                        {tr("Noch kein Konto? ")}
                        Link { class: "font-medium text-foreground underline-offset-4 hover:underline", to: Route::Register {}, {tr("Registrieren")} }
                    }
                }
            }
        }
    }
}

#[component]
fn RegisterForm(first: bool) -> Element {
    let mut username = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let submit = move |e: FormEvent| {
        e.prevent_default();
        if password() != confirm() {
            error.set(Some(tr("Die Passwörter stimmen nicht überein.")));
            return;
        }
        if password().chars().count() < MIN_PASSWORD_LEN {
            error.set(Some(trf("Das Passwort braucht mindestens {0} Zeichen.", &[&MIN_PASSWORD_LEN])));
            return;
        }
        spawn(async move {
            busy.set(true);
            error.set(None);
            let body = RegisterInput { username: username().trim().to_string(), password: password(), display_name: display_name().trim().to_string() };
            match api::post::<User, _>("/auth/register", &body).await {
                Ok(user) => {
                    crate::i18n::set_language(&user.preferences.language, true).await;
                    go("/");
                }
                Err(e) => {
                    busy.set(false);
                    error.set(Some(e.message));
                }
            }
        });
    };
    rsx! {
        form { class: "grid gap-4", onsubmit: submit,
            if first {
                Callout { text: tr("Das erste Konto wird Administrator dieser Instanz.") }
            }
            if let Some(e) = error() {
                Callout { text: e, error: true }
            }
            Field { label: tr("Benutzername"), hint: tr("Damit meldest du dich an."),
                input { class: INPUT, autocomplete: "username", autocapitalize: "none", autofocus: true, required: true, maxlength: 64, value: "{username}", oninput: move |e| username.set(e.value()) }
            }
            Field { label: tr("Anzeigename (optional)"),
                input { class: INPUT, autocomplete: "name", maxlength: 100, value: "{display_name}", oninput: move |e| display_name.set(e.value()) }
            }
            Field { label: tr("Passwort"), hint: trf("Mindestens {0} Zeichen", &[&MIN_PASSWORD_LEN]),
                input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, minlength: MIN_PASSWORD_LEN as i64, value: "{password}", oninput: move |e| password.set(e.value()) }
            }
            Field { label: tr("Passwort wiederholen"),
                input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, value: "{confirm}", oninput: move |e| confirm.set(e.value()) }
            }
            button { class: "{btn(BTN_PRIMARY)} w-full", disabled: busy(), r#type: "submit",
                if busy() {
                    icons::LoaderCircle { class: "size-4 animate-spin" }
                }
                if first { {tr("Einrichten")} } else { {tr("Konto anlegen")} }
            }
            if !first {
                p { class: "text-center text-sm text-muted-foreground",
                    {tr("Schon ein Konto? ")}
                    Link { class: "font-medium text-foreground underline-offset-4 hover:underline", to: Route::Login { next: String::new() }, {tr("Anmelden")} }
                }
            }
        }
    }
}

#[component]
pub fn Setup() -> Element {
    let status = use_resource(|| api::get::<AuthStatus>("/auth/status"));
    let nav = use_navigator();
    if let Some(Ok(s)) = &*status.read() {
        if !s.setup_required {
            nav.replace(Route::Login { next: String::new() });
        }
    }
    rsx! {
        AuthCard { title: tr("nrzNews einrichten"), subtitle: tr("Lege das erste Konto an."),
            RegisterForm { first: true }
        }
    }
}

#[component]
pub fn Register() -> Element {
    let status = use_resource(|| api::get::<AuthStatus>("/auth/status"));
    let open = status.read().as_ref().and_then(|r| r.as_ref().ok()).map(|s| s.registration_open || s.setup_required);
    rsx! {
        AuthCard { title: tr("Registrieren"), subtitle: tr("Neues Konto bei nrzNews"),
            match open {
                None => rsx! { Loading {} },
                Some(true) => rsx! { RegisterForm { first: false } },
                Some(false) => rsx! {
                    div { class: "grid gap-4",
                        Callout { text: tr("Die Registrierung ist geschlossen. Ein Administrator kann dir ein Konto anlegen.") }
                        Link { class: "{btn(BTN_OUTLINE)} w-full", to: Route::Login { next: String::new() }, {tr("Zur Anmeldung")} }
                    }
                },
            }
        }
    }
}

//! Instanz-Einstellungen (nur Administratoren): Benutzer verwalten, Registrierung und Abruf.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use crate::state::ME;
use crate::util;
use dioxus::prelude::*;
use nrznews_types::{CreateUserInput, InstanceSettings, MIN_PASSWORD_LEN, Role, UpdateUserInput, User};

fn role_label(r: Role) -> String {
    match r {
        Role::Admin => tr("Administrator"),
        Role::Member => tr("Mitglied"),
    }
}

// ------------------------------------------------------------------ Benutzer

#[component]
pub fn Users() -> Element {
    let mut version = use_signal(|| 0u64);
    let users = use_resource(move || async move {
        let _ = version();
        api::get::<Vec<User>>("/admin/users").await
    });
    let mut create_open = use_signal(|| false);
    let mut edit_open = use_signal(|| false);
    let mut editing = use_signal(|| None::<User>);
    let me = ME.read().as_ref().map(|u| u.id).unwrap_or_default();
    rsx! {
        Section {
            title: tr("Benutzer"),
            description: tr("Konten dieser Instanz. Jedes Konto hat seine eigenen Feeds, Ordner und Einstellungen."),
            actions: rsx! {
                button { class: btn(BTN_PRIMARY), onclick: move |_| create_open.set(true),
                    icons::UserPlus { class: "size-4" }
                    {tr("Konto anlegen")}
                }
            },
            match &*users.read() {
                None => rsx! { Loading {} },
                Some(Err(e)) => rsx! { Callout { text: e.message.clone(), error: true } },
                Some(Ok(list)) => rsx! {
                    div { class: "divide-y rounded-lg border",
                        for u in list.iter().cloned() {
                            div { key: "{u.id}", class: "flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3 text-sm",
                                span { class: "grid size-8 shrink-0 place-items-center rounded-full bg-secondary text-xs font-semibold", "{util::initial(u.name())}" }
                                div { class: "min-w-0 flex-1 basis-48",
                                    p { class: "flex flex-wrap items-center gap-2",
                                        span { class: if u.disabled { "truncate font-medium text-muted-foreground line-through" } else { "truncate font-medium" }, "{u.name()}" }
                                        if u.is_admin() {
                                            span { class: "rounded-full border px-2 py-0.5 text-[11px] text-muted-foreground", {tr("Admin")} }
                                        }
                                        if u.disabled {
                                            span { class: "rounded-full border border-destructive/40 px-2 py-0.5 text-[11px] text-destructive", {tr("gesperrt")} }
                                        }
                                        if u.id == me {
                                            span { class: "text-muted-foreground", {tr("(du)")} }
                                        }
                                    }
                                    p { class: "truncate text-xs text-muted-foreground",
                                        "@{u.username} · "
                                        match u.last_login_at {
                                            Some(t) => trf("zuletzt angemeldet {0}", &[&util::relative(t)]),
                                            None => tr("noch nie angemeldet"),
                                        }
                                    }
                                }
                                span { class: "hidden text-xs text-muted-foreground md:inline", {trf("seit {0}", &[&util::date(u.created_at)])} }
                                div { class: "flex items-center gap-1",
                                    button { class: ICON_BTN, title: tr("Bearbeiten"),
                                        onclick: {
                                            let u = u.clone();
                                            move |_| {
                                                editing.set(Some(u.clone()));
                                                edit_open.set(true);
                                            }
                                        },
                                        icons::Pencil {}
                                    }
                                    if u.id != me {
                                        button { class: icon_btn("hover:text-destructive"), title: tr("Konto löschen"),
                                            onclick: {
                                                let u = u.clone();
                                                move |_| {
                                                    let u = u.clone();
                                                    spawn(async move {
                                                        let ask = Ask::new(trf("Konto „{0}“ löschen?", &[&u.username]), tr("Das Konto wird mit allen Feeds, Ordnern und gemerkten Artikeln gelöscht.")).confirm(tr("Konto löschen")).destructive();
                                                        if confirm(ask).await {
                                                            match api::delete(&format!("/admin/users/{}", u.id)).await {
                                                                Ok(()) => {
                                                                    toast(tr("Konto gelöscht."));
                                                                    version += 1;
                                                                }
                                                                Err(e) => toast_error(e),
                                                            }
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
                },
            }
        }
        CreateUser { open: create_open, on_done: move |_| version += 1 }
        EditUser { open: edit_open, user: editing(), me, on_done: move |_| version += 1 }
    }
}

#[component]
fn CreateUser(open: Signal<bool>, on_done: EventHandler<()>) -> Element {
    let mut username = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut admin = use_signal(|| false);
    let mut busy = use_signal(|| false);
    use_effect(move || {
        if open() {
            username.set(String::new());
            display_name.set(String::new());
            password.set(String::new());
            admin.set(false);
        }
    });
    rsx! {
        Modal { open, title: tr("Konto anlegen"), description: tr("Gib der Person Benutzername und Passwort weiter; sie kann das Passwort danach unter Einstellungen → Konto ändern."),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    spawn(async move {
                        busy.set(true);
                        let body = CreateUserInput {
                            username: username().trim().to_string(),
                            password: password(),
                            display_name: display_name().trim().to_string(),
                            role: if admin() { Role::Admin } else { Role::Member },
                        };
                        let res = api::post::<User, _>("/admin/users", &body).await;
                        busy.set(false);
                        match res {
                            Ok(u) => {
                                toast(trf("Konto „{0}“ angelegt.", &[&u.username]));
                                open.set(false);
                                on_done.call(());
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Benutzername"),
                    input { class: INPUT, required: true, autocomplete: "off", autocapitalize: "none", maxlength: 64, value: "{username}", oninput: move |e| username.set(e.value()) }
                }
                Field { label: tr("Anzeigename (optional)"),
                    input { class: INPUT, autocomplete: "off", maxlength: 100, value: "{display_name}", oninput: move |e| display_name.set(e.value()) }
                }
                Field { label: tr("Passwort"), hint: trf("Mindestens {0} Zeichen", &[&MIN_PASSWORD_LEN]),
                    input { class: INPUT, r#type: "password", required: true, minlength: MIN_PASSWORD_LEN as i64, autocomplete: "new-password", value: "{password}", oninput: move |e| password.set(e.value()) }
                }
                Checkbox { checked: admin(), onchange: move |v| admin.set(v), label: tr("Administrator der Instanz"), hint: tr("Darf Benutzer, Registrierung, Abruf und KI verwalten.") }
                div { class: "flex justify-end gap-2",
                    button { class: btn(BTN_OUTLINE), r#type: "button", onclick: move |_| open.set(false), {tr("Abbrechen")} }
                    button { class: btn(BTN_PRIMARY), r#type: "submit",
                        disabled: busy() || username().trim().is_empty() || password().chars().count() < MIN_PASSWORD_LEN,
                        {tr("Anlegen")}
                    }
                }
            }
        }
    }
}

#[component]
fn EditUser(open: Signal<bool>, user: Option<User>, me: i64, on_done: EventHandler<()>) -> Element {
    let mut display_name = use_signal(String::new);
    let mut role = use_signal(Role::default);
    let mut password = use_signal(String::new);
    let mut disabled = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let init = user.clone();
    use_effect(use_reactive!(|init| {
        if open() {
            if let Some(u) = &init {
                display_name.set(u.display_name.clone());
                role.set(u.role);
                disabled.set(u.disabled);
            }
            password.set(String::new());
        }
    }));
    let Some(u) = user else { return rsx! {} };
    let id = u.id;
    let is_me = u.id == me;
    let original = u.clone();
    rsx! {
        Modal { open, title: trf("„{0}“ bearbeiten", &[&u.username]),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    let o = original.clone();
                    spawn(async move {
                        let name = display_name().trim().to_string();
                        let pw = password();
                        if !pw.is_empty() && pw.chars().count() < MIN_PASSWORD_LEN {
                            toast_error(trf("Das Passwort braucht mindestens {0} Zeichen.", &[&MIN_PASSWORD_LEN]));
                            return;
                        }
                        let body = UpdateUserInput {
                            display_name: (name != o.display_name).then_some(name),
                            role: (role() != o.role).then_some(role()),
                            password: (!pw.is_empty()).then_some(pw),
                            disabled: (disabled() != o.disabled).then_some(disabled()),
                        };
                        busy.set(true);
                        let res = api::put::<User, _>(&format!("/admin/users/{id}"), &body).await;
                        busy.set(false);
                        match res {
                            Ok(u) => {
                                if u.id == me {
                                    *ME.write() = Some(u);
                                }
                                toast(tr("Konto gespeichert."));
                                open.set(false);
                                on_done.call(());
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Anzeigename"), hint: tr("Leer = Benutzername"),
                    input { class: INPUT, maxlength: 100, value: "{display_name}", oninput: move |e| display_name.set(e.value()) }
                }
                Field { label: tr("Rolle"),
                    select { class: SELECT, disabled: is_me, onchange: move |e| role.set(if e.value() == "admin" { Role::Admin } else { Role::Member }),
                        option { value: "member", selected: role() == Role::Member, {role_label(Role::Member)} }
                        option { value: "admin", selected: role() == Role::Admin, {role_label(Role::Admin)} }
                    }
                }
                if !is_me {
                    Field { label: tr("Neues Passwort"), hint: tr("Leer lassen = unverändert. Ein neues Passwort meldet alle Sitzungen ab."),
                        input { class: INPUT, r#type: "password", autocomplete: "new-password", value: "{password}", oninput: move |e| password.set(e.value()) }
                    }
                    Toggle { checked: disabled(), onchange: move |v| disabled.set(v), label: tr("Konto sperren"), hint: tr("Gesperrte Konten können sich nicht mehr anmelden.") }
                }
                div { class: "flex justify-end gap-2",
                    button { class: btn(BTN_OUTLINE), r#type: "button", onclick: move |_| open.set(false), {tr("Abbrechen")} }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: busy(), {tr("Speichern")} }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Registrierung & Abruf

#[component]
pub fn Instance() -> Element {
    let mut current = use_signal(|| None::<InstanceSettings>);
    let mut registration = use_signal(|| false);
    let mut refresh = use_signal(String::new);
    let mut retention = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut apply = move |s: InstanceSettings| {
        registration.set(s.registration_open);
        refresh.set(s.refresh_minutes.to_string());
        retention.set(s.retention_days.to_string());
        current.set(Some(s));
    };
    use_future(move || async move {
        match api::get::<InstanceSettings>("/settings/instance").await {
            Ok(s) => apply(s),
            Err(e) => toast_error(e),
        }
    });
    let Some(_) = current() else { return rsx! { Loading {} } };
    let refresh_ok = refresh().trim().parse::<u32>().is_ok_and(|m| (5..=1440).contains(&m));
    let retention_ok = retention().trim().parse::<u32>().is_ok();
    rsx! {
        Section { title: tr("Registrierung & Abruf"), description: tr("Gilt für alle Konten dieser Instanz."),
            form { class: "grid max-w-xl gap-6",
                onsubmit: move |e| {
                    e.prevent_default();
                    let body = InstanceSettings {
                        registration_open: registration(),
                        refresh_minutes: refresh().trim().parse().unwrap_or(30),
                        retention_days: retention().trim().parse().unwrap_or(60),
                    };
                    spawn(async move {
                        busy.set(true);
                        let res = api::put::<InstanceSettings, _>("/settings/instance", &body).await;
                        busy.set(false);
                        match res {
                            Ok(s) => {
                                apply(s);
                                toast(tr("Einstellungen gespeichert."));
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Toggle {
                    checked: registration(),
                    onchange: move |v| registration.set(v),
                    label: tr("Neue Benutzer dürfen sich selbst registrieren"),
                    hint: tr("Sonst legen Administratoren die Konten unter „Benutzer“ an."),
                }
                Field { label: tr("Feeds abrufen alle … Minuten"), hint: tr("5 bis 1440 Minuten (ein Tag)."),
                    input { class: "{INPUT} max-w-40", r#type: "number", min: 5, max: 1440, step: 1, required: true, value: "{refresh}", oninput: move |e| refresh.set(e.value()) }
                }
                Field { label: tr("Gelesene Artikel löschen nach … Tagen"), hint: tr("Gemerkte Artikel bleiben immer. 0 = nie löschen."),
                    input { class: "{INPUT} max-w-40", r#type: "number", min: 0, step: 1, required: true, value: "{retention}", oninput: move |e| retention.set(e.value()) }
                }
                div {
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: busy() || !refresh_ok || !retention_ok, {tr("Speichern")} }
                }
            }
        }
    }
}

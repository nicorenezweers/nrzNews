//! Dialoge für Feeds und Ordner: Feed umbenennen/verschieben, Ordner anlegen/umbenennen,
//! dazu die Ordnerauswahl (mit „Neuer Ordner …“), die auch „Feed hinzufügen“ benutzt.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use crate::state::{self, OVERVIEW};
use dioxus::prelude::*;
use nrznews_types::{Feed, Folder, FolderInput, UpdateFeedInput};

/// Wert der Ordnerauswahl für „neuer Ordner“.
pub const NEW_FOLDER: &str = "new";

/// Auswahl des Ordners: leer = ohne Ordner, `NEW_FOLDER` = neuer Ordner (Name darunter).
#[component]
pub fn FolderSelect(value: Signal<String>, new_name: Signal<String>) -> Element {
    let folders: Vec<Folder> = OVERVIEW.read().as_ref().map(|o| o.folders.clone()).unwrap_or_default();
    rsx! {
        div { class: "grid gap-2",
            select { class: SELECT, onchange: move |e| value.set(e.value()),
                option { value: "", selected: value().is_empty(), {tr("— ohne Ordner —")} }
                for f in folders.iter() {
                    option { key: "{f.id}", value: "{f.id}", selected: value() == f.id.to_string(), "{f.name}" }
                }
                option { value: NEW_FOLDER, selected: value() == NEW_FOLDER, {tr("Neuer Ordner …")} }
            }
            if value() == NEW_FOLDER {
                input { class: INPUT, placeholder: tr("Name des neuen Ordners"), maxlength: 100, value: "{new_name}", oninput: move |e| new_name.set(e.value()) }
            }
        }
    }
}

/// Gewählten Ordner als ID – legt ihn an, wenn „Neuer Ordner“ gewählt ist.
pub async fn resolve_folder(value: &str, new_name: &str) -> api::ApiResult<Option<i64>> {
    if value == NEW_FOLDER {
        let name = new_name.trim();
        if name.is_empty() {
            return Ok(None);
        }
        let f = api::post::<Folder, _>("/folders", &FolderInput { name: name.to_string() }).await?;
        return Ok(Some(f.id));
    }
    Ok(value.parse::<i64>().ok())
}

/// Feed umbenennen und in einen anderen Ordner legen.
#[component]
pub fn FeedDialog(open: Signal<bool>, feed: Option<Feed>) -> Element {
    let mut title = use_signal(String::new);
    let folder = use_signal(String::new);
    let new_folder = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let init = feed.clone();
    use_effect(use_reactive!(|init| {
        if open() {
            let mut folder = folder;
            let mut new_folder = new_folder;
            title.set(init.as_ref().map(|f| f.title.clone()).unwrap_or_default());
            folder.set(init.as_ref().and_then(|f| f.folder_id).map(|id| id.to_string()).unwrap_or_default());
            new_folder.set(String::new());
        }
    }));
    let Some(f) = feed else { return rsx! {} };
    let id = f.id;
    rsx! {
        Modal { open, title: tr("Feed bearbeiten"), description: f.feed_url.clone(),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    spawn(async move {
                        busy.set(true);
                        let res = async {
                            let folder_id = resolve_folder(&folder(), &new_folder()).await?;
                            api::put::<Feed, _>(&format!("/feeds/{id}"), &UpdateFeedInput { title: title().trim().to_string(), folder_id }).await
                        }
                        .await;
                        busy.set(false);
                        match res {
                            Ok(_) => {
                                toast(tr("Feed gespeichert."));
                                open.set(false);
                                state::load_overview().await;
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Name"), hint: tr("Leer = Titel des Feeds"),
                    input { class: INPUT, maxlength: 200, value: "{title}", oninput: move |e| title.set(e.value()) }
                }
                Field { label: tr("Ordner"),
                    FolderSelect { value: folder, new_name: new_folder }
                }
                div { class: "flex justify-end gap-2 pt-1",
                    button { class: btn(BTN_OUTLINE), r#type: "button", onclick: move |_| open.set(false), {tr("Abbrechen")} }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: busy(), {tr("Speichern")} }
                }
            }
        }
    }
}

/// Ordner anlegen (`folder` = None) oder umbenennen.
#[component]
pub fn FolderDialog(open: Signal<bool>, folder: Option<Folder>) -> Element {
    let mut name = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let init = folder.clone();
    use_effect(use_reactive!(|init| {
        if open() {
            name.set(init.as_ref().map(|f| f.name.clone()).unwrap_or_default());
        }
    }));
    let id = folder.as_ref().map(|f| f.id);
    let title = if folder.is_some() { tr("Ordner umbenennen") } else { tr("Neuer Ordner") };
    rsx! {
        Modal { open, title,
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    let body = FolderInput { name: name().trim().to_string() };
                    spawn(async move {
                        busy.set(true);
                        let res: api::ApiResult<Folder> = match id {
                            Some(id) => api::put(&format!("/folders/{id}"), &body).await,
                            None => api::post("/folders", &body).await,
                        };
                        busy.set(false);
                        match res {
                            Ok(f) => {
                                toast(trf("Ordner „{0}“ gespeichert.", &[&f.name]));
                                open.set(false);
                                state::load_overview().await;
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Name"),
                    input { class: INPUT, required: true, maxlength: 100, placeholder: tr("z. B. Technik"), value: "{name}", oninput: move |e| name.set(e.value()) }
                }
                div { class: "flex justify-end gap-2 pt-1",
                    button { class: btn(BTN_OUTLINE), r#type: "button", onclick: move |_| open.set(false), {tr("Abbrechen")} }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: busy() || name().trim().is_empty(), {tr("Speichern")} }
                }
            }
        }
    }
}

/// Feed abbestellen (nach Rückfrage). `true`, wenn gelöscht.
pub async fn unsubscribe(feed: &Feed) -> bool {
    let ask = Ask::new(trf("„{0}“ abbestellen?", &[&feed.title]), tr("Der Feed und alle seine Artikel werden gelöscht – auch gemerkte.")).confirm(tr("Abbestellen")).destructive();
    if !confirm(ask).await {
        return false;
    }
    match api::delete(&format!("/feeds/{}", feed.id)).await {
        Ok(()) => {
            toast(trf("„{0}“ abbestellt.", &[&feed.title]));
            state::load_overview().await;
            true
        }
        Err(e) => {
            toast_error(e);
            false
        }
    }
}

/// Ordner löschen (nach Rückfrage); seine Feeds wandern nach „ohne Ordner“. `true`, wenn gelöscht.
pub async fn delete_folder(folder: &Folder) -> bool {
    let ask = Ask::new(trf("Ordner „{0}“ löschen?", &[&folder.name]), tr("Die Feeds darin bleiben abonniert und stehen danach ohne Ordner da.")).confirm(tr("Löschen")).destructive();
    if !confirm(ask).await {
        return false;
    }
    match api::delete(&format!("/folders/{}", folder.id)).await {
        Ok(()) => {
            toast(tr("Ordner gelöscht."));
            state::load_overview().await;
            true
        }
        Err(e) => {
            toast_error(e);
            false
        }
    }
}

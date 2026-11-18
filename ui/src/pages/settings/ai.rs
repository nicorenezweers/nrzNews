//! KI & Modelle (nur Administratoren): Anbieter, Schlüssel, Adresse, Modell, Ausweichmodelle
//! und Sprache für Zusammenfassungen, Überblick und Dieter. Schlüssel verlassen den Server nie –
//! hier erscheint nur ein Hinweis wie „sk-or-v1-0…cdef“.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::*;
use crate::i18n::{tr, trf};
use dioxus::prelude::*;
use nrznews_types::{AiModel, AiModels, AiProviderInfo, AiSettingsInput, AiSettingsView};

fn source_label(s: &str) -> String {
    match s {
        "settings" => tr("Hier hinterlegt"),
        "environment" => tr("Aus einer Umgebungsvariable des Servers"),
        "builtin" => tr("Eingebauter Schlüssel"),
        _ => tr("Kein Schlüssel"),
    }
}

/// Anbieter in Gruppen für die Auswahl.
fn group(p: &AiProviderInfo) -> &'static str {
    if p.local {
        "Lokal (eigener Rechner)"
    } else if p.id == "custom" || p.id == "azure" {
        "Eigene Adresse"
    } else {
        "Cloud"
    }
}

fn empty_provider() -> AiProviderInfo {
    AiProviderInfo {
        id: String::new(),
        name: String::new(),
        url: String::new(),
        model: String::new(),
        needs_key: false,
        local: false,
        key_url: String::new(),
        has_key: false,
        key_hint: String::new(),
        custom_url: String::new(),
    }
}

#[component]
pub fn Ai() -> Element {
    let mut current = use_signal(|| None::<AiSettingsView>);
    let mut disabled = use_signal(|| false);
    let mut provider = use_signal(String::new);
    let mut key = use_signal(String::new);
    let mut url = use_signal(String::new);
    let mut model = use_signal(String::new);
    let mut fallbacks = use_signal(Vec::<String>::new);
    let mut language = use_signal(|| "auto".to_string());
    let mut new_fallback = use_signal(String::new);
    let mut models = use_signal(Vec::<AiModel>::new);
    let mut model_filter = use_signal(String::new);
    let mut loading_models = use_signal(|| false);
    let mut saving = use_signal(|| false);

    let mut apply = move |s: AiSettingsView| {
        disabled.set(s.disabled);
        provider.set(s.provider.clone());
        model.set(s.model.clone());
        fallbacks.set(s.fallbacks.clone());
        language.set(if s.language.is_empty() { "auto".into() } else { s.language.clone() });
        url.set(s.providers.iter().find(|p| p.id == s.provider).map(|p| p.custom_url.clone()).unwrap_or_default());
        key.set(String::new());
        current.set(Some(s));
    };
    use_future(move || async move {
        match api::get::<AiSettingsView>("/settings/ai").await {
            Ok(s) => apply(s),
            Err(e) => toast_error(e),
        }
    });
    let input = move |clear_key: bool| AiSettingsInput {
        disabled: disabled(),
        provider: provider(),
        model: model().trim().to_string(),
        fallbacks: fallbacks(),
        key: key().trim().to_string(),
        clear_key,
        url: Some(url().trim().to_string()),
        language: Some(language()),
    };
    let save = move |clear_key: bool| {
        spawn(async move {
            saving.set(true);
            let res = api::put::<AiSettingsView, _>("/settings/ai", &input(clear_key)).await;
            saving.set(false);
            match res {
                Ok(s) => {
                    apply(s);
                    toast(tr("KI-Einstellungen gespeichert."));
                }
                Err(e) => toast_error(e),
            }
        });
    };
    let load_models = move |_| {
        loading_models.set(true);
        spawn(async move {
            // Zuerst speichern, damit Schlüssel und Adresse gelten
            match api::put::<AiSettingsView, _>("/settings/ai", &input(false)).await {
                Ok(s) => apply(s),
                Err(e) => {
                    toast_error(e);
                    loading_models.set(false);
                    return;
                }
            }
            match api::get::<AiModels>(&format!("/settings/ai/models?provider={}", api::enc(&provider()))).await {
                Ok(m) => {
                    if m.models.is_empty() {
                        toast(tr("Der Anbieter meldet keine Modelle."));
                    }
                    models.set(m.models);
                }
                Err(e) => toast_error(e),
            }
            loading_models.set(false);
        });
    };

    let Some(cur) = current() else { return rsx! { Loading {} } };
    let p = cur.providers.iter().find(|x| x.id == provider()).cloned().unwrap_or_else(empty_provider);
    let switched = provider() != cur.provider;
    let own_key = p.has_key;
    let show_url = p.url.is_empty() || p.local || !url().is_empty() || p.id == "custom" || p.id == "azure";
    let filter = model_filter().to_lowercase();
    let shown: Vec<AiModel> = models()
        .into_iter()
        .filter(|m| filter.is_empty() || m.id.to_lowercase().contains(&filter) || m.name.to_lowercase().contains(&filter))
        .collect();
    let groups = ["Cloud", "Lokal (eigener Rechner)", "Eigene Adresse"];
    let active_name = cur.providers.iter().find(|x| x.id == cur.provider).map(|x| x.name.clone()).unwrap_or_default();
    let p_default_url = p.url.clone();
    let has_auto = cur.languages.iter().any(|(c, _)| c == "auto");
    rsx! {
        Section { title: tr("Status"),
            div { class: "grid gap-3 sm:grid-cols-2 xl:grid-cols-3",
                div { class: "rounded-lg border p-4",
                    p { class: "text-xs text-muted-foreground", {tr("KI")} }
                    p { class: "mt-1 flex items-center gap-2 font-medium",
                        span { class: if cur.available { "size-2 rounded-full bg-success" } else { "size-2 rounded-full bg-destructive" } }
                        if cur.disabled { {tr("Ausgeschaltet")} } else if cur.available { {tr("Bereit")} } else { {tr("Nicht bereit")} }
                    }
                    if !cur.missing.is_empty() {
                        p { class: "mt-1 text-xs text-muted-foreground", {tr(&cur.missing)} }
                    }
                }
                div { class: "rounded-lg border p-4",
                    p { class: "text-xs text-muted-foreground", {tr("Anbieter und Schlüssel")} }
                    p { class: "mt-1 font-medium", "{active_name}" }
                    p { class: "text-xs text-muted-foreground", "{source_label(&cur.key_source)}" }
                    if !cur.key_hint.is_empty() {
                        p { class: "font-mono text-xs text-muted-foreground", "{cur.key_hint}" }
                    }
                }
                div { class: "rounded-lg border p-4",
                    p { class: "text-xs text-muted-foreground", {tr("Aktives Modell")} }
                    p { class: "mt-1 break-all font-mono text-sm", if cur.effective_model.is_empty() { "–" } else { "{cur.effective_model}" } }
                    if !cur.effective_url.is_empty() {
                        p { class: "break-all font-mono text-[11px] text-muted-foreground", "{cur.effective_url}" }
                    }
                }
            }
        }
        Section {
            title: tr("Anbieter und Modell"),
            description: tr("Die KI fasst Artikel zusammen, schreibt den Überblick und beantwortet Fragen an Dieter. Jeder Anbieter mit OpenAI-kompatibler API funktioniert, dazu Anthropic. Lokale Server (Ollama, LM Studio, vLLM) brauchen keinen Schlüssel."),
            form { class: "grid max-w-2xl gap-5",
                onsubmit: move |e| {
                    e.prevent_default();
                    save(false);
                },
                Toggle { checked: !disabled(), onchange: move |on: bool| disabled.set(!on), label: tr("KI verwenden") }
                Field { label: tr("Anbieter"),
                    select {
                        class: SELECT,
                        onchange: move |e| {
                            let id = e.value();
                            let Some(cur) = current() else { return };
                            url.set(cur.providers.iter().find(|x| x.id == id).map(|x| x.custom_url.clone()).unwrap_or_default());
                            if id != cur.provider {
                                model.set(String::new());
                                fallbacks.set(Vec::new());
                            } else {
                                model.set(cur.model.clone());
                                fallbacks.set(cur.fallbacks.clone());
                            }
                            key.set(String::new());
                            models.set(Vec::new());
                            provider.set(id);
                        },
                        for g in groups {
                            if cur.providers.iter().any(|x| group(x) == g) {
                                optgroup { label: tr(g),
                                    for x in cur.providers.iter().filter(|x| group(x) == g) {
                                        option { value: "{x.id}", selected: x.id == provider(),
                                            "{x.name}"
                                            if x.has_key { " ✓" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if switched {
                    Callout { text: trf("Noch nicht gespeichert – erst mit „Speichern“ gilt {0}.", &[&p.name]) }
                }
                Field {
                    label: if p.needs_key { tr("API-Schlüssel") } else { tr("API-Schlüssel (optional)") },
                    hint: if own_key { trf("Hinterlegt ({0}). Leer lassen = behalten.", &[&p.key_hint]) } else { tr("Wird auf dem Server gespeichert und nie wieder angezeigt.") },
                    div { class: "flex gap-2",
                        input {
                            class: "{INPUT} font-mono",
                            r#type: "password",
                            autocomplete: "off",
                            placeholder: if own_key { p.key_hint.clone() } else { String::new() },
                            value: "{key}",
                            oninput: move |e| key.set(e.value()),
                        }
                        if own_key {
                            button { class: btn(BTN_OUTLINE), r#type: "button",
                                onclick: move |_| {
                                    spawn(async move {
                                        let ask = Ask::new(tr("Schlüssel entfernen?"), tr("Der hinterlegte Schlüssel dieses Anbieters wird gelöscht.")).confirm(tr("Entfernen")).destructive();
                                        if confirm(ask).await {
                                            save(true);
                                        }
                                    });
                                },
                                {tr("Entfernen")}
                            }
                        }
                    }
                    if !p.key_url.is_empty() {
                        a { class: "mt-1 inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline", href: "{p.key_url}", target: "_blank", rel: "noopener noreferrer",
                            {trf("Schlüssel bei {0} holen", &[&p.name])}
                            icons::ExternalLink { class: "size-3" }
                        }
                    }
                }
                if show_url {
                    Field {
                        label: tr("Adresse der API"),
                        hint: if p.id == "azure" { tr("z. B. https://NAME.openai.azure.com/openai/v1 – Modell = Name der Bereitstellung") } else if p.url.is_empty() { tr("Basis-Adresse bis …/v1 (ohne /chat/completions)") } else { trf("Leer = {0}", &[&p.url]) },
                        input {
                            class: "{INPUT} font-mono",
                            r#type: "url",
                            placeholder: if p.url.is_empty() { "https://…/v1".to_string() } else { p.url.clone() },
                            value: "{url}",
                            oninput: move |e| url.set(e.value()),
                        }
                    }
                } else {
                    button { r#type: "button", class: "justify-self-start text-xs text-muted-foreground hover:text-foreground", onclick: move |_| url.set(p_default_url.clone()), {tr("Eigene Adresse (Proxy, Gateway) …")} }
                }
                Field {
                    label: tr("Modell"),
                    hint: if p.model.is_empty() { tr("Modell-ID des Anbieters – „Modelle laden“ zeigt die verfügbaren.") } else { trf("Leer = {0}", &[&p.model]) },
                    div { class: "flex gap-2",
                        input { class: "{INPUT} font-mono", list: "ai-models", placeholder: p.model.clone(), value: "{model}", oninput: move |e| model.set(e.value()) }
                        button { class: btn(BTN_OUTLINE), r#type: "button", disabled: loading_models(), onclick: load_models, title: tr("Modelle beim Anbieter abfragen"),
                            if loading_models() {
                                icons::LoaderCircle { class: "size-4 animate-spin" }
                            } else {
                                icons::RefreshCw { class: "size-4" }
                            }
                            span { class: "hidden sm:inline", {tr("Modelle laden")} }
                        }
                    }
                    datalist { id: "ai-models",
                        for m in models() {
                            option { value: "{m.id}", "{m.name}" }
                        }
                    }
                    if !models().is_empty() {
                        div { class: "mt-2 flex flex-wrap items-center gap-3",
                            input { class: input_class("h-8 max-w-xs"), placeholder: tr("Modelle filtern …"), value: "{model_filter}", oninput: move |e| model_filter.set(e.value()) }
                            span { class: "text-xs text-muted-foreground", {trf("{0} von {1}", &[&shown.len(), &models().len()])} }
                        }
                        div { class: "mt-2 flex max-h-64 flex-col overflow-y-auto rounded-md border text-sm",
                            for m in shown.into_iter().take(300) {
                                button {
                                    r#type: "button",
                                    class: if model() == m.id { "flex items-center justify-between gap-3 bg-accent px-3 py-1.5 text-left" } else { "flex items-center justify-between gap-3 px-3 py-1.5 text-left hover:bg-accent/50" },
                                    onclick: {
                                        let id = m.id.clone();
                                        move |_| model.set(id.clone())
                                    },
                                    span { class: "truncate", "{m.name}" }
                                    span { class: "truncate font-mono text-xs text-muted-foreground", "{m.id}" }
                                }
                            }
                        }
                    }
                }
                Field { label: tr("Sprache der KI"), hint: tr("Für Zusammenfassungen, Überblick und Dieter."),
                    select { class: SELECT, onchange: move |e| language.set(e.value()),
                        if !has_auto {
                            option { value: "auto", selected: language() == "auto", {tr("Wie der Artikel")} }
                        }
                        for (code , name) in cur.languages.iter().cloned() {
                            option { value: "{code}", selected: language() == code,
                                if code == "auto" { {tr("Wie der Artikel")} } else { "{name}" }
                            }
                        }
                    }
                }
                div { class: "grid gap-2",
                    span { class: "text-sm font-medium", {tr("Ausweichmodelle")} }
                    p { class: "text-xs text-muted-foreground", {tr("Werden der Reihe nach versucht, wenn das Modell ausgelastet ist oder Unbrauchbares liefert.")} }
                    for (i , f) in fallbacks().into_iter().enumerate() {
                        div { key: "{i}-{f}", class: "flex items-center gap-2 rounded-md border px-3 py-1.5",
                            span { class: "w-5 text-xs text-muted-foreground", "{i + 1}." }
                            span { class: "min-w-0 flex-1 truncate font-mono text-sm", "{f}" }
                            button { r#type: "button", class: "text-muted-foreground hover:text-foreground disabled:opacity-30", title: tr("Nach oben"), disabled: i == 0,
                                onclick: move |_| fallbacks.write().swap(i, i - 1),
                                icons::ChevronUp { class: "size-4" }
                            }
                            button { r#type: "button", class: "text-muted-foreground hover:text-destructive", title: tr("Entfernen"),
                                onclick: move |_| {
                                    fallbacks.write().remove(i);
                                },
                                icons::X { class: "size-4" }
                            }
                        }
                    }
                    input {
                        class: "{INPUT} font-mono",
                        list: "ai-models",
                        placeholder: tr("Modell-ID + Enter"),
                        value: "{new_fallback}",
                        oninput: move |e| new_fallback.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                let v = new_fallback().trim().to_string();
                                if !v.is_empty() && !fallbacks().contains(&v) {
                                    fallbacks.write().push(v);
                                }
                                new_fallback.set(String::new());
                            }
                        },
                    }
                }
                div {
                    button { class: btn(BTN_PRIMARY), r#type: "submit", disabled: saving(), {tr("Speichern")} }
                }
            }
        }
    }
}

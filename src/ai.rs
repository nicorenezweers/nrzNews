//! KI: Anbieter-Katalog (OpenRouter, OpenAI, Anthropic, Ollama, …), Einstellungen und Aufrufe
//! (übernommen aus nrzPaper, ohne Bilder/PDF/Texterkennung – nrzNews schickt nur Text).
//!
//! Anbieter: alles, was die OpenAI-Chat-API spricht (OpenRouter, OpenAI, Google Gemini,
//! Mistral, Groq, xAI, DeepSeek, Together, Fireworks, Perplexity, Cohere, NVIDIA,
//! Hugging Face, Qwen, Azure OpenAI, Ollama, LM Studio, vLLM, LocalAI und jede eigene
//! Adresse), dazu Anthropic über die Messages-API.
//!
//! Standard ist das kostenlose `openrouter/free` (mit eingebautem Schlüssel, falls beim Bauen
//! einer gesetzt wurde). Dessen Router landet manchmal bei einem ungeeigneten Modell; dann
//! werden der Reihe nach kostenlose Ersatzmodelle probiert.
//!
//! Schlüssel liegen je Anbieter in `app_settings` ("ai") und verlassen den Server nie – die
//! Oberfläche sieht nur, ob einer hinterlegt ist, und Anfang/Ende zur Wiedererkennung.

use crate::error::{AppError, AppResult};
use crate::services::settings;
use nrznews_types::{AiModel, AiModels, AiProviderInfo, AiSettingsInput, AiSettingsView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::time::Duration;

/// Eingebauter Schlüssel für OpenRouter (beim Bauen als Secret gesetzt, nie im Quelltext),
/// damit die KI ohne Einrichtung geht. Ein leerer Wert zählt als „keiner“: BuildKit setzt
/// die Variable bei fehlendem Secret auf "".
pub const BUILTIN_KEY: Option<&str> = match option_env!("NRZ_OPENROUTER_KEY") {
    Some(k) if !k.is_empty() => Some(k),
    _ => None,
};

/// Zeitlimit je KI-Anfrage (lokale Modelle sind langsam) …
const CHAT_TIMEOUT: Duration = Duration::from_secs(120);
/// … und für alle Versuche mit Ersatzmodellen zusammen (danach kein weiterer Versuch).
const TOTAL_TIMEOUT: Duration = Duration::from_secs(180);

static HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP-Client")
});

// ------------------------------------------------------------------ Anbieter

/// Wie mit dem Anbieter gesprochen wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// `POST {url}/chat/completions` (OpenAI-kompatibel)
    OpenAi,
    /// `POST {url}/messages` (Anthropic)
    Anthropic,
}

/// Wie der Schlüssel mitgeschickt wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Auth {
    /// `Authorization: Bearer …`
    Bearer,
    /// `api-key: …` (Azure OpenAI)
    ApiKey,
    /// `x-api-key: …` (Anthropic)
    XApiKey,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub protocol: Protocol,
    pub auth: Auth,
    /// Voreingestellte Adresse (leer = muss eingetragen werden)
    pub url: &'static str,
    /// Ohne Schlüssel nutzbar (lokale Server, eigene API)
    pub key_optional: bool,
    /// Läuft auf dem eigenen Rechner/Server
    pub local: bool,
    /// Voreingestelltes Modell (leer = in den Einstellungen wählen)
    pub model: &'static str,
    /// Wo es Schlüssel gibt
    pub keys_url: &'static str,
}

const fn p(
    id: &'static str,
    name: &'static str,
    url: &'static str,
    model: &'static str,
    keys_url: &'static str,
) -> Provider {
    Provider {
        id,
        name,
        protocol: Protocol::OpenAi,
        auth: Auth::Bearer,
        url,
        key_optional: false,
        local: false,
        model,
        keys_url,
    }
}

const fn local(
    id: &'static str,
    name: &'static str,
    url: &'static str,
    model: &'static str,
) -> Provider {
    Provider {
        key_optional: true,
        local: true,
        ..p(id, name, url, model, "")
    }
}

pub const PROVIDERS: &[Provider] = &[
    p(
        "openrouter",
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        DEFAULT_MODEL,
        "https://openrouter.ai/keys",
    ),
    p(
        "openai",
        "OpenAI",
        "https://api.openai.com/v1",
        "gpt-4o-mini",
        "https://platform.openai.com/api-keys",
    ),
    Provider {
        protocol: Protocol::Anthropic,
        auth: Auth::XApiKey,
        ..p(
            "anthropic",
            "Anthropic (Claude)",
            "https://api.anthropic.com/v1",
            "claude-haiku-5-5",
            "https://console.anthropic.com/settings/keys",
        )
    },
    p(
        "gemini",
        "Google Gemini",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "gemini-2.5-flash",
        "https://aistudio.google.com/apikey",
    ),
    p(
        "mistral",
        "Mistral AI",
        "https://api.mistral.ai/v1",
        "mistral-small-latest",
        "https://console.mistral.ai/api-keys",
    ),
    p(
        "xai",
        "xAI (Grok)",
        "https://api.x.ai/v1",
        "",
        "https://console.x.ai",
    ),
    p(
        "groq",
        "Groq",
        "https://api.groq.com/openai/v1",
        "llama-3.3-70b-versatile",
        "https://console.groq.com/keys",
    ),
    p(
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com/v1",
        "deepseek-chat",
        "https://platform.deepseek.com/api_keys",
    ),
    p(
        "together",
        "Together AI",
        "https://api.together.xyz/v1",
        "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        "https://api.together.ai/settings/api-keys",
    ),
    p(
        "fireworks",
        "Fireworks AI",
        "https://api.fireworks.ai/inference/v1",
        "",
        "https://fireworks.ai/account/api-keys",
    ),
    p(
        "perplexity",
        "Perplexity",
        "https://api.perplexity.ai",
        "sonar",
        "https://www.perplexity.ai/settings/api",
    ),
    p(
        "cohere",
        "Cohere",
        "https://api.cohere.ai/compatibility/v1",
        "command-a-03-2025",
        "https://dashboard.cohere.com/api-keys",
    ),
    p(
        "nvidia",
        "NVIDIA NIM",
        "https://integrate.api.nvidia.com/v1",
        "",
        "https://build.nvidia.com",
    ),
    p(
        "huggingface",
        "Hugging Face",
        "https://router.huggingface.co/v1",
        "",
        "https://huggingface.co/settings/tokens",
    ),
    p(
        "qwen",
        "Alibaba Qwen (DashScope)",
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        "qwen-plus",
        "https://dashscope.console.aliyun.com/apiKey",
    ),
    Provider {
        auth: Auth::ApiKey,
        ..p("azure", "Azure OpenAI", "", "", "https://portal.azure.com")
    },
    local(
        "ollama",
        "Ollama (lokal)",
        "http://localhost:11434/v1",
        "llama3.2",
    ),
    local(
        "lmstudio",
        "LM Studio (lokal)",
        "http://localhost:1234/v1",
        "",
    ),
    local(
        "vllm",
        "vLLM / LocalAI (lokal)",
        "http://localhost:8000/v1",
        "",
    ),
    Provider {
        key_optional: true,
        ..p("custom", "Eigene OpenAI-kompatible API", "", "", "")
    },
];

pub fn provider(id: &str) -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or(&PROVIDERS[0])
}

pub const DEFAULT_MODEL: &str = "openrouter/free";
pub const FALLBACK_MODELS: &[&str] = &[
    "nvidia/nemotron-3.5-super-vl:free",
    "google/gemma-4-31b-it:free",
    "google/gemma-4-26b-a4b-it:free",
];

/// Sprachen für Zusammenfassungen und Überblick ("auto" = wie der Artikel).
pub const LANGUAGES: &[(&str, &str)] = &[
    ("de", "Deutsch"),
    ("en", "English"),
    ("fr", "français"),
    ("it", "italiano"),
    ("es", "español"),
    ("pt", "português"),
    ("nl", "Nederlands"),
];

/// Anweisung für die Sprache der Antwort, z. B. „auf Deutsch“. `auto` = `auto_text`.
pub fn language_instruction(code: &str, auto_text: &str) -> String {
    match LANGUAGES.iter().find(|(c, _)| *c == code) {
        Some((_, name)) => format!("auf {name}"),
        None => auto_text.to_string(),
    }
}

// ------------------------------------------------------------------ Einstellungen

/// KI-Einstellungen in der Oberfläche (app_settings "ai"). Leere Felder = Vorgabe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub disabled: bool,
    /// Gewählter Anbieter (leer = Anbieter aus der Umgebung, sonst OpenRouter)
    pub provider: String,
    /// Schlüssel je Anbieter – beim Wechseln geht keiner verloren
    pub keys: BTreeMap<String, String>,
    /// Eigene Adresse je Anbieter (leer = Vorgabe des Anbieters)
    pub urls: BTreeMap<String, String>,
    pub model: String,
    /// Ersatzmodelle; leer = bei OpenRouter die eingebauten kostenlosen Modelle
    pub fallbacks: Vec<String>,
    /// Sprache der Zusammenfassungen (leer = "auto")
    pub language: String,
}

impl AiSettings {
    /// Hinterlegter Schlüssel für einen Anbieter.
    pub fn key_for(&self, provider: &str) -> &str {
        self.keys
            .get(provider)
            .map(|k| k.trim())
            .unwrap_or_default()
    }
}

pub const SETTINGS_KEY: &str = "ai";

/// Woher der Schlüssel kommt (für die Einstellungen; der Schlüssel selbst bleibt geheim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Settings,
    Environment,
    Builtin,
    None,
}

impl KeySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeySource::Settings => "settings",
            KeySource::Environment => "environment",
            KeySource::Builtin => "builtin",
            KeySource::None => "none",
        }
    }
}

/// Zugang zur KI: Anbieter, Schlüssel, Adresse, Modell, Ersatzmodelle.
#[derive(Clone)]
pub struct AiConfig {
    pub provider: &'static Provider,
    pub key: String,
    pub model: String,
    pub url: String,
    /// Weitere Modelle, wenn das gewünschte versagt (der Reihe nach).
    pub fallbacks: Vec<String>,
    /// KI ganz ausgeschaltet (Einstellungen)
    pub disabled: bool,
    /// Sprache der Zusammenfassungen: Code (de, en, …), "auto" = wie der Artikel
    pub language: String,
}

impl AiConfig {
    /// Nur aus der Konfiguration (Umgebung/eingebaut) – ohne die Einstellungen in der Datenbank.
    pub fn from(config: &crate::config::Config) -> Self {
        let provider = provider(config.ai_provider.trim());
        let openrouter = provider.id == "openrouter";
        let env_key = config
            .ai_key
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        let key = if env_key.is_empty() && openrouter {
            BUILTIN_KEY.map(String::from).unwrap_or_default()
        } else {
            env_key
        };
        let url = if config.ai_url.trim().is_empty() {
            provider.url
        } else {
            config.ai_url.trim()
        };
        let model = if config.ai_model.trim().is_empty() {
            provider.model.to_string()
        } else {
            config.ai_model.trim().to_string()
        };
        AiConfig {
            provider,
            key,
            model,
            url: url.trim_end_matches('/').to_string(),
            fallbacks: if openrouter {
                FALLBACK_MODELS.iter().map(|m| m.to_string()).collect()
            } else {
                Vec::new()
            },
            disabled: false,
            language: "auto".into(),
        }
    }

    /// Konfiguration plus Einstellungen aus der Oberfläche (die haben Vorrang).
    pub async fn load(pool: &SqlitePool, config: &crate::config::Config) -> Self {
        let stored = settings::get::<AiSettings>(pool, SETTINGS_KEY)
            .await
            .ok()
            .flatten();
        Self::with_settings(config, stored.as_ref())
    }

    pub fn with_settings(config: &crate::config::Config, s: Option<&AiSettings>) -> Self {
        let mut ai = Self::from(config);
        let Some(s) = s else { return ai };
        let base = ai.provider.id;
        let chosen = if s.provider.is_empty() {
            ai.provider
        } else {
            provider(&s.provider)
        };
        if chosen.id != base {
            // Anderer Anbieter als in der Umgebung: dessen Vorgaben, kein Umgebungs-Schlüssel
            ai.provider = chosen;
            ai.key = String::new();
            ai.url = chosen.url.to_string();
            ai.model = chosen.model.to_string();
            ai.fallbacks = Vec::new();
            if chosen.id == "openrouter" {
                ai.key = BUILTIN_KEY.map(String::from).unwrap_or_default();
                ai.fallbacks = FALLBACK_MODELS.iter().map(|m| m.to_string()).collect();
            }
        }
        let key = s.key_for(chosen.id);
        if !key.is_empty() {
            ai.key = key.to_string();
        }
        if let Some(url) = s
            .urls
            .get(chosen.id)
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
        {
            ai.url = url.trim_end_matches('/').to_string();
        }
        if !s.model.trim().is_empty() {
            ai.model = s.model.trim().to_string();
        }
        if !s.fallbacks.is_empty() {
            ai.fallbacks = s.fallbacks.clone();
        }
        ai.disabled = s.disabled;
        if !s.language.is_empty() {
            ai.language = s.language.clone();
        }
        ai
    }

    /// Woher der wirksame Schlüssel kommt.
    pub fn key_source(config: &crate::config::Config, s: Option<&AiSettings>) -> KeySource {
        let env_provider = provider(config.ai_provider.trim()).id;
        let id = match s.map(|s| s.provider.as_str()).filter(|p| !p.is_empty()) {
            Some(p) => provider(p).id,
            None => env_provider,
        };
        if s.is_some_and(|s| !s.key_for(id).is_empty()) {
            return KeySource::Settings;
        }
        if env_provider == id
            && config
                .ai_key
                .as_deref()
                .is_some_and(|k| !k.trim().is_empty())
        {
            return KeySource::Environment;
        }
        if id == "openrouter" && BUILTIN_KEY.is_some() {
            return KeySource::Builtin;
        }
        KeySource::None
    }

    pub fn available(&self) -> bool {
        self.missing().is_none()
    }

    /// Kurzform des Schlüssels für die Anzeige (Anfang und Ende).
    pub fn key_hint(&self) -> String {
        hint(&self.key)
    }

    /// Warum die KI nicht bereit ist (deutsche Meldung für Fehler und Einstellungen).
    pub fn missing(&self) -> Option<String> {
        const WHERE: &str = "bitte unter Einstellungen → KI & Modelle einrichten.";
        if self.disabled {
            Some("Die KI ist ausgeschaltet – ein Admin kann sie unter Einstellungen → KI & Modelle einschalten.".into())
        } else if self.key.is_empty() && !self.provider.key_optional {
            Some(format!(
                "Für {} fehlt ein API-Schlüssel – {WHERE}",
                self.provider.name
            ))
        } else if self.url.is_empty() {
            Some(format!(
                "Für {} fehlt die Adresse der API – {WHERE}",
                self.provider.name
            ))
        } else if self.model.is_empty() {
            Some(format!(
                "Für {} ist kein Modell gewählt – {WHERE}",
                self.provider.name
            ))
        } else {
            None
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = HTTP.request(method, format!("{}/{path}", self.url));
        if !self.key.is_empty() {
            req = match self.provider.auth {
                Auth::Bearer => req.bearer_auth(&self.key),
                Auth::ApiKey => req.header("api-key", &self.key),
                Auth::XApiKey => req.header("x-api-key", &self.key),
            };
        }
        if self.provider.protocol == Protocol::Anthropic {
            req = req.header("anthropic-version", "2023-06-01");
        }
        if self.provider.id == "openrouter" {
            req = req
                .header("X-Title", "nrzNews")
                .header("HTTP-Referer", "https://github.com/nicorenezweers/nrzNews");
        }
        req
    }
}

/// Anfang und Ende eines Schlüssels zur Wiedererkennung ("sk-or-v1-0…cdef").
pub fn hint(key: &str) -> String {
    let k: Vec<char> = key.chars().collect();
    if k.len() < 16 {
        return String::new();
    }
    format!(
        "{}…{}",
        k[..10].iter().collect::<String>(),
        k[k.len() - 4..].iter().collect::<String>()
    )
}

/// Einstellungen für die Oberfläche – ohne Schlüssel.
pub async fn settings_view(
    pool: &SqlitePool,
    config: &crate::config::Config,
) -> AppResult<AiSettingsView> {
    let stored = settings::get::<AiSettings>(pool, SETTINGS_KEY)
        .await?
        .unwrap_or_default();
    let effective = AiConfig::with_settings(config, Some(&stored));
    let env_provider = provider(config.ai_provider.trim()).id;
    let providers = PROVIDERS
        .iter()
        .map(|p| {
            let key = stored.key_for(p.id);
            let env_key = p.id == env_provider
                && config
                    .ai_key
                    .as_deref()
                    .is_some_and(|k| !k.trim().is_empty());
            AiProviderInfo {
                id: p.id.into(),
                name: p.name.into(),
                url: p.url.into(),
                model: p.model.into(),
                needs_key: !p.key_optional,
                local: p.local,
                key_url: p.keys_url.into(),
                has_key: !key.is_empty()
                    || env_key
                    || (p.id == "openrouter" && BUILTIN_KEY.is_some()),
                key_hint: hint(key),
                custom_url: stored.urls.get(p.id).cloned().unwrap_or_default(),
            }
        })
        .collect();
    Ok(AiSettingsView {
        disabled: stored.disabled,
        provider: effective.provider.id.into(),
        model: stored.model.clone(),
        fallbacks: stored.fallbacks.clone(),
        language: if stored.language.is_empty() {
            "auto".into()
        } else {
            stored.language.clone()
        },
        effective_model: effective.model.clone(),
        effective_url: effective.url.clone(),
        key_source: AiConfig::key_source(config, Some(&stored)).as_str().into(),
        key_hint: effective.key_hint(),
        available: effective.available(),
        missing: effective.missing().unwrap_or_default(),
        providers,
        languages: std::iter::once(("auto".to_string(), "Wie der Artikel".to_string()))
            .chain(
                LANGUAGES
                    .iter()
                    .map(|(c, n)| (c.to_string(), n.to_string())),
            )
            .collect(),
    })
}

/// Einstellungen speichern (Admin). Schlüssel nur ersetzen, wenn ein neuer kommt.
pub async fn save_settings(
    pool: &SqlitePool,
    config: &crate::config::Config,
    input: &AiSettingsInput,
) -> AppResult<AiSettingsView> {
    let mut stored = settings::get::<AiSettings>(pool, SETTINGS_KEY)
        .await?
        .unwrap_or_default();
    let wanted = input.provider.trim();
    if !wanted.is_empty() {
        let p = provider(wanted);
        if p.id != wanted {
            return Err(AppError::bad("Diesen KI-Anbieter gibt es nicht."));
        }
        stored.provider = p.id.to_string();
    }
    let id = if stored.provider.is_empty() {
        provider(config.ai_provider.trim()).id
    } else {
        provider(&stored.provider).id
    };
    let key = input.key.trim();
    if !key.is_empty() {
        if key.len() < 8 || key.len() > 4096 || key.contains(char::is_whitespace) {
            return Err(AppError::bad("Das sieht nicht wie ein API-Schlüssel aus."));
        }
        if id == "openrouter" && !key.starts_with("sk-") {
            return Err(AppError::bad(
                "Das sieht nicht wie ein OpenRouter-Schlüssel aus (sk-or-…).",
            ));
        }
        stored.keys.insert(id.to_string(), key.to_string());
    } else if input.clear_key {
        stored.keys.remove(id);
    }
    if let Some(url) = input.url.as_deref() {
        let url = url.trim().trim_end_matches('/');
        if url.is_empty() {
            stored.urls.remove(id);
        } else if !(url.starts_with("http://") || url.starts_with("https://"))
            || url.len() > 500
            || url::Url::parse(url).is_err()
        {
            return Err(AppError::bad(
                "Die Adresse muss mit http:// oder https:// beginnen.",
            ));
        } else {
            stored.urls.insert(id.to_string(), url.to_string());
        }
    }
    let clean = |m: &str| {
        m.trim()
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(200)
            .collect::<String>()
    };
    stored.model = clean(&input.model);
    stored.fallbacks = input
        .fallbacks
        .iter()
        .map(|m| clean(m))
        .filter(|m| !m.is_empty())
        .take(10)
        .collect();
    stored.disabled = input.disabled;
    if let Some(l) = input.language.as_deref() {
        if l != "auto" && !LANGUAGES.iter().any(|(c, _)| *c == l) {
            return Err(AppError::bad("Diese Sprache gibt es nicht."));
        }
        stored.language = l.to_string();
    }
    settings::set(pool, SETTINGS_KEY, &stored).await?;
    settings_view(pool, config).await
}

// ------------------------------------------------------------------ Anfragen

fn openai_body(model: &str, system: &str, prompt: &str, openrouter: bool) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 3000,
        "temperature": 0.2,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": prompt },
        ],
    });
    if openrouter {
        body["reasoning"] = json!({ "effort": "low" });
    }
    body
}

fn anthropic_body(model: &str, system: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 3000,
        "temperature": 0.2,
        "system": system,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": prompt }] }],
    })
}

async fn chat(
    ai: &AiConfig,
    model: &str,
    system: &str,
    prompt: &str,
) -> Result<(String, String), String> {
    let anthropic = ai.provider.protocol == Protocol::Anthropic;
    let (path, body) = if anthropic {
        ("messages", anthropic_body(model, system, prompt))
    } else {
        (
            "chat/completions",
            openai_body(model, system, prompt, ai.provider.id == "openrouter"),
        )
    };
    let resp = ai
        .request(reqwest::Method::POST, path)
        .timeout(CHAT_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "{model}: keine Antwort innerhalb von {} s",
                    CHAT_TIMEOUT.as_secs()
                )
            } else {
                format!("{} nicht erreichbar", ai.provider.name)
            }
        })?;
    let status = resp.status();
    let value: Value = resp.json().await.unwrap_or_default();
    let error = value["error"]["message"]
        .as_str()
        .or_else(|| value["error"].as_str())
        .or_else(|| value["message"].as_str().filter(|_| !status.is_success()));
    if let Some(msg) = error {
        return Err(format!("{model}: {msg} ({})", status.as_u16()));
    }
    if !status.is_success() {
        return Err(format!("{model}: HTTP {}", status.as_u16()));
    }
    let content = if anthropic {
        value["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    } else {
        value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    if content.trim().is_empty() {
        return Err(format!("{model}: leere Antwort"));
    }
    Ok((
        content,
        value["model"].as_str().unwrap_or(model).to_string(),
    ))
}

/// Textanfrage: gewünschtes Modell, dann die Ersatzmodelle. Gibt Antwort und tatsächlich
/// genutztes Modell zurück.
pub async fn complete(
    ai: &AiConfig,
    system: &str,
    prompt: &str,
) -> Result<(String, String), String> {
    if let Some(why) = ai.missing() {
        return Err(why);
    }
    let mut models = vec![ai.model.clone()];
    // Das freie Routing landet manchmal bei einem schlechten Modell: zweimal versuchen
    if ai.model == DEFAULT_MODEL && ai.provider.id == "openrouter" {
        models.push(DEFAULT_MODEL.into());
    }
    models.extend(ai.fallbacks.iter().filter(|m| **m != ai.model).cloned());
    let mut errors = Vec::new();
    let started = std::time::Instant::now();
    for m in &models {
        // Nicht endlos Ersatzmodelle probieren, wenn schon die ersten nur warten liessen
        if !errors.is_empty() && started.elapsed() > TOTAL_TIMEOUT {
            break;
        }
        match chat(ai, m, system, prompt).await {
            Ok(r) => return Ok(r),
            Err(e) => errors.push(e),
        }
    }
    Err(format!(
        "Die KI hat nicht geantwortet – {}",
        errors.join("; ")
    ))
}

/// Erstes JSON-Objekt in einer Antwort (Modelle setzen gern ```json davor).
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end)
        .then(|| serde_json::from_str(&text[start..=end]).ok())
        .flatten()
}

// ------------------------------------------------------------------ Modelle

/// Modelle des Anbieters (`GET {url}/models`).
pub async fn models(ai: &AiConfig) -> Result<Vec<AiModel>, String> {
    if ai.url.is_empty() {
        return Err(format!(
            "Für {} fehlt die Adresse der API.",
            ai.provider.name
        ));
    }
    let resp = ai
        .request(reqwest::Method::GET, "models")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| format!("{} nicht erreichbar.", ai.provider.name))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|_| format!("{}: unerwartete Antwort.", ai.provider.name))?;
    if !status.is_success() {
        let msg = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .unwrap_or("");
        return Err(format!(
            "{}: HTTP {} {msg}",
            ai.provider.name,
            status.as_u16()
        ));
    }
    let list = v["data"].as_array().or_else(|| v["models"].as_array());
    let mut out: Vec<AiModel> = list
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str().or_else(|| m["name"].as_str())?;
            let id = id.trim_start_matches("models/").to_string();
            let name = m["display_name"]
                .as_str()
                .or_else(|| m["name"].as_str())
                .unwrap_or(&id)
                .trim_start_matches("models/")
                .to_string();
            Some(AiModel { id, name })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// Modelle des gewählten Anbieters – oder eines anderen (`provider`) für die Auswahl.
pub async fn list_models(
    pool: &SqlitePool,
    config: &crate::config::Config,
    provider_id: Option<&str>,
) -> AppResult<AiModels> {
    let mut stored = settings::get::<AiSettings>(pool, SETTINGS_KEY)
        .await?
        .unwrap_or_default();
    if let Some(p) = provider_id.map(str::trim).filter(|p| !p.is_empty()) {
        if provider(p).id != p {
            return Err(AppError::bad("Diesen KI-Anbieter gibt es nicht."));
        }
        stored.provider = p.to_string();
        stored.model.clear();
    }
    let ai = AiConfig::with_settings(config, Some(&stored));
    let models = models(&ai).await.map_err(AppError::BadRequest)?;
    Ok(AiModels { models })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_catalog_is_consistent() {
        let mut ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PROVIDERS.len(), "IDs eindeutig");
        assert_eq!(provider("gibt-es-nicht").id, "openrouter");
        for p in PROVIDERS {
            assert!(p.url.is_empty() || p.url.starts_with("http"), "{}", p.id);
            assert!(!p.url.ends_with('/'), "{}", p.id);
        }
    }

    #[test]
    fn settings_choose_provider_key_and_url() {
        let config = crate::config::Config::default();
        let s = AiSettings {
            provider: "ollama".into(),
            model: "llama3.2".into(),
            urls: [(
                "ollama".to_string(),
                "http://192.168.0.5:11434/v1/".to_string(),
            )]
            .into(),
            ..Default::default()
        };
        let ai = AiConfig::with_settings(&config, Some(&s));
        assert_eq!(ai.provider.id, "ollama");
        assert_eq!(ai.url, "http://192.168.0.5:11434/v1");
        assert!(ai.available(), "Ollama braucht keinen Schlüssel");
        assert_eq!(AiConfig::key_source(&config, Some(&s)), KeySource::None);

        let s = AiSettings {
            provider: "anthropic".into(),
            keys: [(
                "anthropic".to_string(),
                "sk-ant-test-0123456789".to_string(),
            )]
            .into(),
            ..Default::default()
        };
        let ai = AiConfig::with_settings(&config, Some(&s));
        assert_eq!(
            (ai.url.as_str(), ai.model.as_str()),
            ("https://api.anthropic.com/v1", "claude-haiku-5-5")
        );
        assert_eq!(ai.key, "sk-ant-test-0123456789");
        assert!(ai.fallbacks.is_empty());
        assert_eq!(AiConfig::key_source(&config, Some(&s)), KeySource::Settings);

        // Ohne Schlüssel und ohne Modell: nicht bereit, mit verständlichem Grund
        let s = AiSettings {
            provider: "xai".into(),
            ..Default::default()
        };
        let ai = AiConfig::with_settings(&config, Some(&s));
        assert!(!ai.available());
        let why = ai.missing().unwrap_or_default();
        assert!(
            why.contains("xAI") && why.contains("Einstellungen → KI & Modelle"),
            "{why}"
        );
    }

    #[test]
    fn environment_overrides() {
        let config = crate::config::Config {
            ai_provider: "openai".into(),
            ai_key: Some("sk-env-0123456789abcdef".into()),
            ai_model: "gpt-x".into(),
            ..Default::default()
        };
        let ai = AiConfig::with_settings(&config, None);
        assert_eq!((ai.provider.id, ai.model.as_str()), ("openai", "gpt-x"));
        assert_eq!(AiConfig::key_source(&config, None), KeySource::Environment);
        // Anderer Anbieter in den Einstellungen: der Umgebungs-Schlüssel gilt dort nicht
        let s = AiSettings {
            provider: "mistral".into(),
            ..Default::default()
        };
        assert!(AiConfig::with_settings(&config, Some(&s)).key.is_empty());
    }

    #[test]
    fn bodies() {
        let a = anthropic_body("claude", "sys", "frage");
        assert_eq!(a["system"], "sys");
        assert_eq!(a["messages"][0]["content"][0]["text"], "frage");
        let o = openai_body("gpt", "sys", "frage", false);
        assert_eq!(o["messages"][0]["role"], "system");
        assert_eq!(o["messages"][1]["content"], "frage");
        assert!(
            o.get("reasoning").is_none(),
            "nur OpenRouter kennt reasoning"
        );
        assert!(openai_body("m", "s", "p", true).get("reasoning").is_some());
        assert_eq!(extract_json("```json\n{\"a\": 1}\n```").unwrap()["a"], 1);
    }
}

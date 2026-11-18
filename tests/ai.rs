//! KI: Einstellungen (nur Admins, Schlüssel bleiben geheim), Anthropic (Messages-API, eigene
//! Kopfzeilen), lokaler OpenAI-kompatibler Server ohne Schlüssel, Modell-Liste,
//! Zusammenfassung (gespeichert), Überblick mit Quellen und Dieter – gegen Schein-Server.

mod common;

use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Eine aufgezeichnete Anfrage: Methode, Pfad, Kopfzeilen (klein geschrieben), Körper.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    head: String,
    body: Value,
}

type Answer = Arc<dyn Fn(&str, &Value) -> Value + Send + Sync>;

/// Schein-KI: zeichnet Anfragen auf und antwortet je nach Pfad und Körper.
async fn fake(answer: Answer, log: Arc<Mutex<Vec<Seen>>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (log, answer) = (log.clone(), answer.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 16384];
                let (head_end, len) = loop {
                    let n = match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + len {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let (method, path) = (
                    first.next().unwrap_or("").to_string(),
                    first.next().unwrap_or("").to_string(),
                );
                let body: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                let reply = answer(&path, &body).to_string();
                log.lock().unwrap().push(Seen {
                    method,
                    path,
                    head: head.to_lowercase(),
                    body,
                });
                let out = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = sock.write_all(out.as_bytes()).await;
            });
        }
    });
    url
}

/// Text der Anfrage (System + Nutzer), egal ob OpenAI- oder Anthropic-Format.
fn prompt_of(body: &Value) -> (String, String) {
    if body.get("system").is_some() {
        (
            body["system"].as_str().unwrap_or_default().to_string(),
            body["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    } else {
        (
            body["messages"][0]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            body["messages"][1]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }
}

async fn feed(pool: &SqlitePool, user: i64, title: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO feeds (user_id, title, feed_url, created_at) VALUES (?, ?, ?, 0) RETURNING id",
    )
    .bind(user)
    .bind(title)
    .bind(format!("https://x.example/{title}"))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn article(
    pool: &SqlitePool,
    feed: i64,
    user: i64,
    title: &str,
    text: &str,
    published: i64,
    read: bool,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO articles (feed_id, user_id, guid, title, content_text, published_at, fetched_at, read)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(feed)
    .bind(user)
    .bind(title)
    .bind(title)
    .bind(text)
    .bind(published)
    .bind(published)
    .bind(read)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn put_ai(c: &Client, body: Value) -> (Status, Value) {
    common::put(c, "/api/v1/settings/ai", body).await
}

const SUMMARY: &str =
    "- Rocket 0.5 ist stabil\n- Async überall\n- Neue Guards\n\n**Fazit:** Ein grosser Schritt.";

#[rocket::async_test]
async fn anthropic_settings_models_and_cached_summary() {
    let (c, _dir) = common::client().await;
    let admin = common::login_as(&c, "admin", true).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let url = fake(
        Arc::new(|path: &str, _body: &Value| {
            if path.ends_with("/models") {
                return json!({ "data": [
                    { "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5" },
                    { "id": "claude-haiku-5-5", "display_name": "Claude Haiku 5.5" }
                ] });
            }
            json!({ "model": "claude-haiku-5-5", "content": [{ "type": "text", "text": format!("```markdown\n{SUMMARY}\n```") }] })
        }),
        log.clone(),
    )
    .await;

    let (st, v) = common::get(&c, "/api/v1/settings/ai").await;
    assert_eq!(st, Status::Ok, "{v}");
    assert_eq!(
        (v["provider"].clone(), v["language"].clone()),
        (json!("openrouter"), json!("auto"))
    );
    assert!(v["providers"].as_array().unwrap().len() >= 20);
    assert_eq!(v["languages"][0], json!(["auto", "Wie der Artikel"]));

    // Anthropic: Schlüssel je Anbieter, eigene Adresse (Schein-Server)
    let (st, v) = put_ai(
        &c,
        json!({ "disabled": false, "provider": "anthropic", "key": "sk-ant-test-0123456789", "url": format!("{url}/v1/"), "model": "", "fallbacks": [] }),
    )
    .await;
    assert_eq!(st, Status::Ok, "{v}");
    assert_eq!(v["provider"], "anthropic");
    assert_eq!(v["effectiveModel"], "claude-haiku-5-5");
    assert_eq!(v["effectiveUrl"], format!("{url}/v1"));
    assert_eq!(
        (
            v["keySource"].clone(),
            v["available"].clone(),
            v["missing"].clone()
        ),
        (json!("settings"), json!(true), json!(""))
    );
    assert!(
        !v.to_string().contains("sk-ant-test-0123456789"),
        "Schlüssel bleibt geheim"
    );
    let anth = v["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "anthropic")
        .unwrap()
        .clone();
    assert_eq!(
        (anth["hasKey"].clone(), anth["keyHint"].clone()),
        (json!(true), json!("sk-ant-tes…6789"))
    );
    assert_eq!(anth["customUrl"], format!("{url}/v1"));

    let (st, m) = common::get(&c, "/api/v1/settings/ai/models").await;
    assert_eq!(st, Status::Ok, "{m}");
    assert_eq!(m["models"][0]["id"], "claude-haiku-5-5", "sortiert");
    assert_eq!(m["models"][0]["name"], "Claude Haiku 5.5");
    let seen = log.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("GET", "/v1/models")
    );
    assert!(seen.head.contains("x-api-key: sk-ant-test-0123456789"));

    // Zusammenfassung: gespeichert, beim zweiten Mal ohne Anfrage
    let pool = common::pool(&c);
    let f = feed(pool, admin.id, "Rocket-Blog").await;
    let text = "Rocket 0.5 ist erschienen. ".repeat(40);
    let id = article(
        pool,
        f,
        admin.id,
        "Rocket 0.5",
        &text,
        nrznews::db::now() - 60,
        false,
    )
    .await;
    let before = log.lock().unwrap().len();
    let (st, s) = common::post(&c, format!("/api/v1/articles/{id}/summary"), json!({})).await;
    assert_eq!(st, Status::Ok, "{s}");
    assert_eq!(s["summary"], SUMMARY, "Codeblock-Zaun entfernt");
    assert_eq!(
        (s["model"].clone(), s["cached"].clone()),
        (json!("claude-haiku-5-5"), json!(false))
    );
    let seen = log.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("POST", "/v1/messages")
    );
    assert!(
        seen.head.contains("x-api-key: sk-ant-test-0123456789")
            && seen.head.contains("anthropic-version: 2023-06-01")
    );
    assert!(!seen.head.contains("authorization:"));
    let (system, prompt) = prompt_of(&seen.body);
    assert!(
        system.contains("Stichpunkte") && system.contains("in der Sprache des Artikels"),
        "{system}"
    );
    assert!(
        prompt.contains("Titel: Rocket 0.5") && prompt.contains("Quelle: Rocket-Blog"),
        "{prompt}"
    );

    let (_, s) = common::post(&c, format!("/api/v1/articles/{id}/summary"), json!({})).await;
    assert_eq!(s["cached"], true);
    assert_eq!(
        log.lock().unwrap().len(),
        before + 1,
        "keine zweite Anfrage"
    );
    let (_, a) = common::get(&c, format!("/api/v1/articles/{id}")).await;
    assert_eq!(a["aiSummary"], SUMMARY);
    let (_, s) = common::post(
        &c,
        format!("/api/v1/articles/{id}/summary?refresh=true"),
        json!({}),
    )
    .await;
    assert_eq!(s["cached"], false);
    assert_eq!(log.lock().unwrap().len(), before + 2);

    // Sprache fest: steht in der Anweisung
    let (st, v) = put_ai(
        &c,
        json!({ "disabled": false, "model": "", "fallbacks": [], "language": "fr" }),
    )
    .await;
    assert_eq!(
        (st, v["language"].clone(), v["keySource"].clone()),
        (Status::Ok, json!("fr"), json!("settings"))
    );
    common::post(
        &c,
        format!("/api/v1/articles/{id}/summary?refresh=true"),
        json!({}),
    )
    .await;
    let (system, _) = prompt_of(&log.lock().unwrap().last().unwrap().body);
    assert!(system.contains("auf français"), "{system}");

    // Ungültiges
    for bad in [
        json!({ "disabled": false, "provider": "gibts-nicht", "model": "", "fallbacks": [] }),
        json!({ "disabled": false, "provider": "custom", "url": "ftp://x", "model": "", "fallbacks": [] }),
        json!({ "disabled": false, "provider": "openrouter", "key": "falsch-12345", "model": "", "fallbacks": [] }),
        json!({ "disabled": false, "provider": "anthropic", "model": "", "fallbacks": [], "language": "xx" }),
    ] {
        assert_eq!(put_ai(&c, bad.clone()).await.0, Status::BadRequest, "{bad}");
    }
    // Eigene API ohne Adresse und Modell: nicht bereit, mit Grund
    let (_, v) = put_ai(
        &c,
        json!({ "disabled": false, "provider": "custom", "url": "", "model": "", "fallbacks": [] }),
    )
    .await;
    assert_eq!(v["available"], false);
    assert!(v["missing"].as_str().unwrap().contains("Adresse"), "{v}");
    // Zurück zu Anthropic: Schlüssel ist noch da
    let (_, v) = put_ai(
        &c,
        json!({ "disabled": false, "provider": "anthropic", "model": "", "fallbacks": [] }),
    )
    .await;
    assert_eq!(v["keySource"], "settings");
    // Schlüssel entfernen
    let (_, v) = put_ai(&c, json!({ "disabled": false, "provider": "anthropic", "clearKey": true, "model": "", "fallbacks": [] })).await;
    assert_eq!(
        (v["keySource"].clone(), v["available"].clone()),
        (json!("none"), json!(false))
    );

    // Nur Admins sehen und ändern die Einstellungen; andere Benutzer nicht die Artikel
    common::logout(&c).await;
    assert_eq!(
        common::get(&c, "/api/v1/settings/ai").await.0,
        Status::Unauthorized
    );
    common::login_as(&c, "member", false).await;
    assert_eq!(
        common::get(&c, "/api/v1/settings/ai").await.0,
        Status::Forbidden
    );
    assert_eq!(
        put_ai(&c, json!({ "disabled": true })).await.0,
        Status::Forbidden
    );
    assert_eq!(
        common::get(&c, "/api/v1/settings/ai/models").await.0,
        Status::Forbidden
    );
    let (st, _) = common::post(&c, format!("/api/v1/articles/{id}/summary"), json!({})).await;
    assert_eq!(st, Status::NotFound, "fremder Artikel");
}

#[rocket::async_test]
async fn local_openai_compatible_briefing_and_dieter() {
    let (c, _dir) = common::client().await;
    let admin = common::login_as(&c, "admin", true).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let url = fake(
        Arc::new(|path: &str, body: &Value| {
            if path.ends_with("/models") {
                return json!({ "object": "list", "data": [{ "id": "llama3.2" }, { "id": "qwen3" }] });
            }
            let (system, _) = prompt_of(body);
            let text = if system.contains("Suchanfragen") {
                r#"{"queries": ["Rocket Release"]}"#.to_string()
            } else if system.contains("Dieter") {
                "Rocket 0.5 ist erschienen [1]. Dazu passt auch [7].".to_string()
            } else {
                "## Rust\n- Neues Release [2]\n- Async überall [3, 2]\n\n## Sonst\n- Erfunden [42]".to_string()
            };
            json!({ "model": "llama3.2", "choices": [{ "message": { "content": text } }] })
        }),
        log.clone(),
    )
    .await;
    let (st, v) = put_ai(
        &c,
        json!({ "disabled": false, "provider": "ollama", "url": format!("{url}/v1"), "model": "llama3.2", "fallbacks": [] }),
    )
    .await;
    assert_eq!(st, Status::Ok, "{v}");
    assert_eq!(
        (v["available"].clone(), v["keySource"].clone()),
        (json!(true), json!("none"))
    );
    let (_, m) = common::get(&c, "/api/v1/settings/ai/models?provider=ollama").await;
    assert_eq!(m["models"].as_array().unwrap().len(), 2);

    let pool = common::pool(&c);
    let rust = feed(pool, admin.id, "Rust-Blog").await;
    let other = feed(pool, admin.id, "Leer").await;
    let t = nrznews::db::now();
    let newest = article(
        pool,
        rust,
        admin.id,
        "Wetterbericht",
        "Sonne und Regen.",
        t - 60,
        false,
    )
    .await;
    let release = article(
        pool,
        rust,
        admin.id,
        "Rocket Release 0.5",
        "Rocket 0.5 bringt async Guards und mehr.",
        t - 120,
        false,
    )
    .await;
    let asyncs = article(
        pool,
        rust,
        admin.id,
        "Async überall",
        "Tokio und Rocket.",
        t - 180,
        false,
    )
    .await;
    article(pool, rust, admin.id, "Schon gelesen", "Alt.", t - 240, true).await;
    article(
        pool,
        rust,
        admin.id,
        "Vorgestern",
        "Zu alt.",
        t - 3 * 86400,
        false,
    )
    .await;

    // Überblick: ungelesene der letzten 24 h, neueste zuerst, Verweise neu nummeriert
    let (st, b) = common::post(&c, "/api/v1/briefing", json!({})).await;
    assert_eq!(st, Status::Ok, "{b}");
    assert_eq!(b["articleCount"], 3);
    assert_eq!(b["model"], "llama3.2");
    assert_eq!(
        b["text"],
        "## Rust\n- Neues Release [1]\n- Async überall [2][1]\n\n## Sonst\n- Erfunden"
    );
    let sources: Vec<i64> = b["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["articleId"].as_i64().unwrap())
        .collect();
    assert_eq!(
        sources,
        vec![release, asyncs],
        "in der Reihenfolge der Verweise"
    );
    assert_eq!(b["sources"][0]["feedTitle"], "Rust-Blog");
    let seen = log.lock().unwrap().last().cloned().unwrap();
    assert_eq!(seen.path, "/v1/chat/completions");
    assert!(
        !seen.head.contains("authorization:"),
        "lokal ohne Schlüssel"
    );
    assert!(seen.body.get("reasoning").is_none());
    let (system, prompt) = prompt_of(&seen.body);
    assert!(
        system.contains("## Thema") && system.contains("auf Deutsch"),
        "{system}"
    );
    assert!(
        prompt.contains("[1] Wetterbericht — Rust-Blog")
            && prompt.contains("[2] Rocket Release 0.5"),
        "{prompt}"
    );
    assert!(
        !prompt.contains("Schon gelesen") && !prompt.contains("Vorgestern"),
        "{prompt}"
    );
    let _ = newest;

    let (_, b) = common::post(
        &c,
        "/api/v1/briefing",
        json!({ "unreadOnly": false, "hours": 100 }),
    )
    .await;
    assert_eq!(b["articleCount"], 5);
    // Nichts Neues: keine KI-Anfrage
    let calls = log.lock().unwrap().len();
    let (st, b) = common::post(&c, "/api/v1/briefing", json!({ "feedId": other })).await;
    assert_eq!(
        (st, b["articleCount"].clone(), b["sources"].clone()),
        (Status::Ok, json!(0), json!([]))
    );
    assert!(b["text"].as_str().unwrap().contains("keine"));
    assert_eq!(log.lock().unwrap().len(), calls);

    // Dieter: Suchbegriffe → Volltextsuche → Antwort mit Quellen
    let (st, a) = common::post(
        &c,
        "/api/v1/assistant",
        json!({ "messages": [{ "role": "user", "content": "Was gibt es Neues zu Rocket?" }] }),
    )
    .await;
    assert_eq!(st, Status::Ok, "{a}");
    assert_eq!(
        a["answer"],
        "Rocket 0.5 ist erschienen [1]. Dazu passt auch."
    );
    assert_eq!(a["sources"].as_array().unwrap().len(), 1);
    assert_eq!(a["sources"][0]["articleId"], release);
    assert_eq!(a["model"], "llama3.2");
    let calls: Vec<Seen> = log.lock().unwrap().iter().rev().take(2).cloned().collect();
    let (system, prompt) = prompt_of(&calls[0].body);
    assert!(system.contains("Dieter"), "{system}");
    assert!(
        prompt.contains("[1] Rocket Release 0.5") && prompt.contains("async Guards"),
        "{prompt}"
    );
    let (system, prompt) = prompt_of(&calls[1].body);
    assert!(
        system.contains("Suchanfragen") && prompt.contains("Nutzer: Was gibt es Neues zu Rocket?"),
        "{prompt}"
    );

    let (st, e) = common::post(
        &c,
        "/api/v1/assistant",
        json!({ "messages": [{ "role": "user", "content": "  " }] }),
    )
    .await;
    assert_eq!(st, Status::BadRequest, "{e}");
    let (st, _) = common::post(&c, "/api/v1/assistant", json!({ "messages": [] })).await;
    assert_eq!(st, Status::BadRequest);
}

#[rocket::async_test]
async fn clear_errors_when_ai_is_not_ready() {
    let (c, _dir) = common::client().await;
    let admin = common::login_as(&c, "admin", true).await;
    let pool = common::pool(&c);
    let f = feed(pool, admin.id, "F").await;
    let id = article(
        pool,
        f,
        admin.id,
        "Titel",
        &"Text ".repeat(50),
        nrznews::db::now(),
        false,
    )
    .await;

    put_ai(
        &c,
        json!({ "disabled": true, "model": "", "fallbacks": [] }),
    )
    .await;
    for (uri, body) in [
        (format!("/api/v1/articles/{id}/summary"), json!({})),
        ("/api/v1/briefing".into(), json!({})),
        (
            "/api/v1/assistant".into(),
            json!({ "messages": [{ "role": "user", "content": "Hallo?" }] }),
        ),
    ] {
        let (st, e) = common::post(&c, &uri, body).await;
        assert_eq!(st, Status::BadRequest, "{uri}: {e}");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Einstellungen → KI & Modelle"),
            "{e}"
        );
    }
    let (_, v) = put_ai(
        &c,
        json!({ "disabled": false, "provider": "xai", "model": "", "fallbacks": [] }),
    )
    .await;
    assert_eq!(v["available"], false);
    let (st, e) = common::post(&c, format!("/api/v1/articles/{id}/summary"), json!({})).await;
    assert_eq!(st, Status::BadRequest);
    let msg = e["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("xAI")
            && msg.contains("Schlüssel")
            && msg.contains("Einstellungen → KI & Modelle"),
        "{msg}"
    );

    // Anbieter nicht erreichbar: verständliche Meldung statt 500
    put_ai(
        &c,
        json!({ "disabled": false, "provider": "ollama", "url": "http://127.0.0.1:9/v1", "model": "llama3.2", "fallbacks": [] }),
    )
    .await;
    let (st, e) = common::post(&c, format!("/api/v1/articles/{id}/summary"), json!({})).await;
    assert_eq!(st, Status::BadRequest, "{e}");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nicht erreichbar"),
        "{e}"
    );
    let (st, e) = common::get(&c, "/api/v1/settings/ai/models").await;
    assert_eq!(st, Status::BadRequest, "{e}");
    assert_eq!(
        common::post(&c, "/api/v1/articles/999999/summary", json!({}))
            .await
            .0,
        Status::NotFound
    );
}

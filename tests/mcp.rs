//! MCP-Server unter `/mcp`: API-Tokens (anlegen, nur einmal sichtbar, löschen, gesperrte
//! Konten), Protokoll (initialize, Benachrichtigungen, tools/list, Fehler) und die Werkzeuge
//! mit Trennung der Benutzer.

mod common;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use sqlx::SqlitePool;

async fn feed(pool: &SqlitePool, user: i64, title: &str, error: Option<&str>) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO feeds (user_id, title, feed_url, last_error, last_fetched_at, created_at)
         VALUES (?, ?, ?, ?, 1000, 0) RETURNING id",
    )
    .bind(user)
    .bind(title)
    .bind(format!("https://x.example/{title}"))
    .bind(error)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn article(
    pool: &SqlitePool,
    user: i64,
    feed: i64,
    title: &str,
    text: &str,
    read: bool,
    starred: bool,
) -> i64 {
    let ts = nrznews::db::now();
    sqlx::query_scalar(
        "INSERT INTO articles (feed_id, user_id, guid, url, title, content_html, content_text, published_at, fetched_at, read, starred)
         VALUES (?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(feed)
    .bind(user)
    .bind(format!("{feed}-{title}"))
    .bind(format!("https://x.example/a/{title}"))
    .bind(title)
    .bind(text)
    .bind(ts - 60)
    .bind(ts - 60)
    .bind(read)
    .bind(starred)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Token über die API anlegen (mit der Sitzung des Clients).
async fn new_token(client: &Client, name: &str) -> (i64, String) {
    let (status, body) = common::post(client, "/api/v1/me/tokens", json!({ "name": name })).await;
    assert_eq!(status, Status::Created, "{body}");
    (
        body["token"]["id"].as_i64().unwrap(),
        body["secret"].as_str().unwrap().to_string(),
    )
}

async fn rpc(client: &Client, token: Option<&str>, body: Value) -> (Status, Value) {
    let mut req = client
        .post("/mcp")
        .header(ContentType::JSON)
        .body(body.to_string());
    if let Some(t) = token {
        req = req.header(Header::new("Authorization", format!("Bearer {t}")));
    }
    let res = req.dispatch().await;
    let status = res.status();
    (
        status,
        res.into_json::<Value>().await.unwrap_or(Value::Null),
    )
}

async fn tool(client: &Client, token: &str, name: &str, args: Value) -> Value {
    let (status, body) = rpc(
        client,
        Some(token),
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": name, "arguments": args } }),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["result"]["isError"], false, "{body}");
    // Text und strukturierter Inhalt sind dasselbe JSON
    let text: Value =
        serde_json::from_str(body["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, body["result"]["structuredContent"]);
    text
}

#[rocket::async_test]
async fn tokens_are_shown_once_and_can_be_revoked() {
    let (client, _dir) = common::client().await;
    common::login_as(&client, "anna", true).await;
    let (id, secret) = new_token(&client, "nrzDashboard").await;
    assert!(
        secret.starts_with("nrznews_") && secret.len() > 40,
        "{secret}"
    );

    let (status, list) = common::get(&client, "/api/v1/me/tokens").await;
    assert_eq!(status, Status::Ok);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "nrzDashboard");
    assert!(secret.starts_with(list[0]["prefix"].as_str().unwrap()));
    assert!(
        !list.to_string().contains(&secret),
        "Token steht in der Liste"
    );
    assert!(list[0]["lastUsedAt"].is_null());

    // Leerer Name
    let (status, _) = common::post(&client, "/api/v1/me/tokens", json!({ "name": "  " })).await;
    assert_eq!(status, Status::BadRequest);

    // Benutzen setzt lastUsedAt
    let (status, _) = rpc(
        &client,
        Some(&secret),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
    )
    .await;
    assert_eq!(status, Status::Ok);
    let (_, list) = common::get(&client, "/api/v1/me/tokens").await;
    assert!(list[0]["lastUsedAt"].is_i64());

    // Löschen: danach ungültig
    let (status, _) = common::delete(&client, format!("/api/v1/me/tokens/{id}")).await;
    assert_eq!(status, Status::NoContent);
    let (status, _) = rpc(
        &client,
        Some(&secret),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
    )
    .await;
    assert_eq!(status, Status::Unauthorized);
    let (status, _) = common::delete(&client, format!("/api/v1/me/tokens/{id}")).await;
    assert_eq!(status, Status::NotFound);

    // Ohne Anmeldung keine Token-Verwaltung
    common::logout(&client).await;
    let (status, _) = common::get(&client, "/api/v1/me/tokens").await;
    assert_eq!(status, Status::Unauthorized);
}

#[rocket::async_test]
async fn mcp_requires_a_valid_token() {
    let (client, _dir) = common::client().await;
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .body(ping.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
    assert!(res
        .headers()
        .get_one("WWW-Authenticate")
        .unwrap()
        .starts_with("Bearer"));
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("Token"));

    for bad in ["nrznews_falsch", "irgendwas", "Bearer"] {
        let (status, _) = rpc(&client, Some(bad), ping.clone()).await;
        assert_eq!(status, Status::Unauthorized, "{bad}");
    }

    // Sitzungs-Cookie allein reicht nicht
    common::login_as(&client, "anna", true).await;
    let (status, _) = rpc(&client, None, ping.clone()).await;
    assert_eq!(status, Status::Unauthorized);

    // Gesperrtes Konto: Token wirkt nicht mehr
    let (_, secret) = new_token(&client, "Test").await;
    let (status, _) = rpc(&client, Some(&secret), ping.clone()).await;
    assert_eq!(status, Status::Ok);
    sqlx::query("UPDATE users SET disabled = 1 WHERE username = 'anna'")
        .execute(common::pool(&client))
        .await
        .unwrap();
    let (status, _) = rpc(&client, Some(&secret), ping).await;
    assert_eq!(status, Status::Unauthorized);

    // Kein Server-Strom per GET
    assert_eq!(
        client.get("/mcp").dispatch().await.status(),
        Status::MethodNotAllowed
    );
}

#[rocket::async_test]
async fn protocol_basics() {
    let (client, _dir) = common::client().await;
    common::login_as(&client, "anna", true).await;
    let (_, token) = new_token(&client, "Agent").await;
    let t = Some(token.as_str());

    let (status, init) = rpc(
        &client,
        t,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
    )
    .await;
    assert_eq!(status, Status::Ok);
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "nrzNews");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // Unbekannte Version → die neueste
    let (_, init) = rpc(&client, t, json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } })).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    // Benachrichtigung: 202 ohne Körper
    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("Authorization", format!("Bearer {token}")))
        .body(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Accepted);
    assert!(res.into_string().await.unwrap_or_default().is_empty());

    let (_, list) = rpc(
        &client,
        t,
        json!({ "jsonrpc": "2.0", "id": "a", "method": "tools/list" }),
    )
    .await;
    assert_eq!(list["id"], "a");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "dashboard_overview",
            "list_articles",
            "get_article",
            "list_feeds"
        ]
    );
    for tool in list["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
    }

    // Fehler
    let (status, body) = rpc(
        &client,
        t,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "gibts/nicht" }),
    )
    .await;
    assert_eq!(status, Status::Ok);
    assert_eq!(body["error"]["code"], -32601);
    let (_, body) = rpc(&client, t, json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "loeschen" } })).await;
    assert_eq!(body["error"]["code"], -32602);
    let (_, body) = rpc(&client, t, json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "list_articles", "arguments": { "limit": 500 } } })).await;
    assert_eq!(body["error"]["code"], -32602);
    let res = client
        .post("/mcp")
        .header(Header::new("Authorization", format!("Bearer {token}")))
        .body("{kaputt")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::BadRequest);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["error"]["code"], -32700);

    // Fremde Webseite (DNS-Rebinding)
    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("Authorization", format!("Bearer {token}")))
        .header(Header::new("Origin", "https://boese.example"))
        .body(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Forbidden);
}

#[rocket::async_test]
async fn tools_return_the_users_news() {
    let (client, _dir) = common::client().await;
    let anna = common::login_as(&client, "anna", true).await;
    let (_, token) = new_token(&client, "nrzDashboard").await;
    let pool = common::pool(&client);

    // Bert: eigener Feed – darf bei Anna nirgends auftauchen
    let bert = nrznews::services::users::create(
        pool,
        "bert",
        common::PASSWORD,
        "",
        nrznews_types::Role::Member,
    )
    .await
    .unwrap();
    let bf = feed(pool, bert.id, "Bert-Blog", None).await;
    article(pool, bert.id, bf, "Geheim von Bert", "privat", false, false).await;

    let heise = feed(pool, anna.id, "Heise", None).await;
    let kaputt = feed(pool, anna.id, "Kaputt", Some("HTTP 500")).await;
    let a1 = article(
        pool,
        anna.id,
        heise,
        "Rust 2.0 erschienen",
        "Grosse Neuigkeiten aus der Rust-Welt. Äpfel inklusive.",
        false,
        false,
    )
    .await;
    article(pool, anna.id, heise, "Zweiter Artikel", "Text", false, true).await;
    article(
        pool,
        anna.id,
        kaputt,
        "Alter Artikel",
        "Schon gelesen",
        true,
        false,
    )
    .await;

    let ov = tool(
        &client,
        &token,
        "dashboard_overview",
        json!({ "headlines": 3 }),
    )
    .await;
    assert_eq!(ov["app"], "nrzNews");
    assert_eq!(ov["user"], "anna");
    assert_eq!(ov["counts"]["unread"], 2);
    assert_eq!(ov["counts"]["today"], 2);
    assert_eq!(ov["counts"]["starred"], 1);
    assert_eq!(ov["counts"]["feeds"], 2);
    assert_eq!(ov["counts"]["feedsWithErrors"], 1);
    assert_eq!(ov["counts"]["newInPeriod"], 3);
    assert_eq!(ov["counts"]["articles"], 3);
    assert_eq!(ov["periodHours"], 24);
    let metrics = ov["metrics"].as_array().unwrap();
    let keys: Vec<&str> = metrics.iter().map(|m| m["key"].as_str().unwrap()).collect();
    assert_eq!(
        keys,
        ["unread", "today", "new", "starred", "feeds", "feed_errors"]
    );
    assert_eq!(metrics[0]["value"], 2);
    assert_eq!(metrics[5]["status"], "warning");
    assert_eq!(ov["topFeeds"][0]["title"], "Heise");
    assert_eq!(ov["topFeeds"][0]["unread"], 2);
    assert_eq!(ov["feedErrors"][0]["title"], "Kaputt");
    assert_eq!(ov["feedErrors"][0]["error"], "HTTP 500");
    assert_eq!(ov["latest"].as_array().unwrap().len(), 2);
    assert_eq!(ov["lastFetchedAt"], 1000);
    assert!(!ov.to_string().contains("Bert"));

    let none = tool(
        &client,
        &token,
        "dashboard_overview",
        json!({ "headlines": 0, "hours": 1 }),
    )
    .await;
    assert!(none["latest"].as_array().unwrap().is_empty());
    assert_eq!(none["metrics"][2]["label"], "Neu (1 h)");

    let list = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "all", "limit": 2 }),
    )
    .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    assert!(list["nextCursor"].is_string());
    let rest = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "all", "limit": 2, "cursor": list["nextCursor"] }),
    )
    .await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert!(rest["nextCursor"].is_null());
    let starred = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "starred" }),
    )
    .await;
    assert_eq!(starred["items"][0]["title"], "Zweiter Artikel");
    let found = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "all", "query": "apfel" }),
    )
    .await;
    assert_eq!(found["items"][0]["id"], a1);
    let by_feed = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "all", "feedId": kaputt }),
    )
    .await;
    assert_eq!(by_feed["items"][0]["title"], "Alter Artikel");
    let foreign = tool(
        &client,
        &token,
        "list_articles",
        json!({ "view": "all", "query": "Geheim" }),
    )
    .await;
    assert!(foreign["items"].as_array().unwrap().is_empty());

    let art = tool(&client, &token, "get_article", json!({ "id": a1 })).await;
    assert_eq!(art["title"], "Rust 2.0 erschienen");
    assert_eq!(art["feedTitle"], "Heise");
    assert!(art["text"].as_str().unwrap().contains("Rust-Welt"));
    assert_eq!(art["truncated"], false);
    assert_eq!(art["read"], false, "Lesen per MCP ändert nichts");

    // Fremder Artikel: Werkzeugfehler, keine Daten
    let bert_article: i64 = sqlx::query_scalar("SELECT id FROM articles WHERE user_id = ?")
        .bind(bert.id)
        .fetch_one(pool)
        .await
        .unwrap();
    let (_, body) = rpc(
        &client,
        Some(&token),
        json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "get_article", "arguments": { "id": bert_article } } }),
    )
    .await;
    assert_eq!(body["result"]["isError"], true);
    assert!(!body.to_string().contains("privat"));

    let feeds = tool(&client, &token, "list_feeds", json!({})).await;
    assert_eq!(feeds["feeds"].as_array().unwrap().len(), 2);
    let k = feeds["feeds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["title"] == "Kaputt")
        .unwrap();
    assert_eq!(k["error"], "HTTP 500");
    assert_eq!(k["total"], 1);
}

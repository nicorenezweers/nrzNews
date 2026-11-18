//! Gemeinsame Helfer der Integrationstests: Server mit leerem Datenordner (ohne
//! Hintergrundarbeiten), Konten anlegen und anmelden, JSON-Aufrufe wie die Oberfläche.
//!
//! Benutzung in `tests/<name>.rs`:
//!
//! ```ignore
//! mod common;
//!
//! #[rocket::async_test]
//! async fn beispiel() {
//!     let (client, _dir) = common::client().await;
//!     let alice = common::login_as(&client, "alice", false).await;
//!     let (status, body) = common::call(&client, "GET", "/api/v1/me", None).await;
//!     assert_eq!(status, rocket::http::Status::Ok);
//!     assert_eq!(body["id"], alice.id);
//! }
//! ```
#![allow(dead_code)]

use nrznews_types::{Role, User};
use rocket::figment::Figment;
use rocket::http::{Accept, ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};
use sqlx::SqlitePool;

/// Passwort aller Konten aus [`login_as`].
pub const PASSWORD: &str = "geheim-123";

/// Grundkonfiguration für Tests: leerer Datenordner, keine Hintergrundarbeiten, kein Log,
/// KI ins Leere (kein Anbieter aus der Umgebung des Entwicklers).
pub fn figment(dir: &tempfile::TempDir) -> Figment {
    nrznews::config::figment()
        .merge(("data_dir", dir.path()))
        .merge(("background", false))
        .merge(("log_level", "off"))
        .merge(("cookie_secure", false))
        .merge(("registration_open", false))
        .merge(("ai_provider", ""))
        .merge(("ai_url", "http://127.0.0.1:9"))
}

/// Server ohne Konto, Konfiguration angepasst (z. B. `|f| f.merge(("fetch_timeout_secs", 2))`).
/// Der Client merkt sich Cookies (`tracked`). Den TempDir behalten, solange der Client läuft.
pub async fn client_with(adjust: impl FnOnce(Figment) -> Figment) -> (Client, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("Temp-Ordner");
    let rocket = nrznews::build_with(adjust(figment(&dir)))
        .await
        .expect("Server bauen");
    let client = Client::tracked(rocket).await.expect("Client starten");
    (client, dir)
}

/// Server ohne Konto, Standard-Testkonfiguration.
pub async fn client() -> (Client, tempfile::TempDir) {
    client_with(|f| f).await
}

/// Datenbank des Servers (für Vorbereitungen und Prüfungen direkt in SQL).
pub fn pool(client: &Client) -> &SqlitePool {
    client.rocket().state::<SqlitePool>().expect("Pool")
}

/// Konto anlegen (direkt über `services::users::create`, Passwort [`PASSWORD`]) und über
/// die API anmelden – der Client behält das Sitzungs-Cookie.
pub async fn login_as(client: &Client, username: &str, admin: bool) -> User {
    let role = if admin { Role::Admin } else { Role::Member };
    nrznews::services::users::create(pool(client), username, PASSWORD, "", role)
        .await
        .expect("Konto anlegen");
    let (status, body) = login(client, username, PASSWORD).await;
    assert_eq!(status, Status::Ok, "Anmeldung von {username}: {body}");
    serde_json::from_value(body).expect("User")
}

/// JSON-Aufruf mit der Sitzung des Clients; Antwortkörper als JSON (`Null`, wenn leer).
pub async fn call(
    client: &Client,
    method: &str,
    uri: impl AsRef<str>,
    body: Option<Value>,
) -> (Status, Value) {
    let uri = uri.as_ref().to_string();
    let req = match method {
        "GET" => client.get(uri),
        "POST" => client.post(uri),
        "PUT" => client.put(uri),
        "PATCH" => client.patch(uri),
        "DELETE" => client.delete(uri),
        other => panic!("Methode {other} fehlt im Test-Helfer"),
    }
    .header(Accept::JSON);
    let req = match body {
        Some(b) => req.header(ContentType::JSON).body(b.to_string()),
        None => req,
    };
    let res = req.dispatch().await;
    let status = res.status();
    (
        status,
        res.into_json::<Value>().await.unwrap_or(Value::Null),
    )
}

pub async fn get(client: &Client, uri: impl AsRef<str>) -> (Status, Value) {
    call(client, "GET", uri, None).await
}

pub async fn post(client: &Client, uri: impl AsRef<str>, body: Value) -> (Status, Value) {
    call(client, "POST", uri, Some(body)).await
}

pub async fn put(client: &Client, uri: impl AsRef<str>, body: Value) -> (Status, Value) {
    call(client, "PUT", uri, Some(body)).await
}

pub async fn delete(client: &Client, uri: impl AsRef<str>) -> (Status, Value) {
    call(client, "DELETE", uri, None).await
}

pub async fn login(client: &Client, username: &str, password: &str) -> (Status, Value) {
    post(
        client,
        "/api/v1/auth/login",
        json!({ "username": username, "password": password }),
    )
    .await
}

pub async fn register(client: &Client, username: &str, password: &str) -> (Status, Value) {
    post(
        client,
        "/api/v1/auth/register",
        json!({ "username": username, "password": password }),
    )
    .await
}

pub async fn logout(client: &Client) -> Status {
    call(client, "POST", "/api/v1/auth/logout", None).await.0
}

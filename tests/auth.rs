//! Anmeldung mit Rockets lokalem Client: Ersteinrichtung (erstes Konto = Admin),
//! Registrierung offen/geschlossen, Login/Logout, falsches Passwort, Bremse gegen
//! Passwort-Raten, CSRF-Schutz über `Origin`, eigenes Konto und Einstellungen,
//! Passwort ändern, Ablauf und Verlängerung von Sitzungen.

mod common;

use common::{call, get, login, login_as, logout, pool, post, put, register, PASSWORD};
use nrznews::auth::session;
use nrznews::services::users;
use nrznews_types::{AuthStatus, Role, User};
use rocket::http::{ContentType, Header, Status};
use serde_json::{json, Value};

/// Sitzungen eines Kontos in der Datenbank.
async fn sessions(client: &rocket::local::asynchronous::Client, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool(client))
        .await
        .unwrap()
}

#[rocket::async_test]
async fn first_start_sets_up_the_first_account_as_admin() {
    let (client, _dir) = common::client().await;

    let (s, v) = get(&client, "/api/v1/auth/status").await;
    assert_eq!(s, Status::Ok);
    let status: AuthStatus = serde_json::from_value(v).unwrap();
    assert!(status.setup_required);
    assert!(!status.authenticated);
    assert!(
        status.registration_open,
        "das erste Konto darf sich immer registrieren"
    );
    assert_eq!(status.user, None);
    assert_eq!(status.version, env!("CARGO_PKG_VERSION"));

    let res = client
        .post("/api/v1/auth/register")
        .header(ContentType::JSON)
        .body(
            json!({ "username": "alice", "password": PASSWORD, "displayName": "Alice A." })
                .to_string(),
        )
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Created);
    let cookie = res.headers().get_one("Set-Cookie").unwrap().to_string();
    for flag in [
        "nrznews_session=",
        "HttpOnly",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=2592000",
    ] {
        assert!(cookie.contains(flag), "{flag} fehlt im Cookie: {cookie}");
    }
    assert!(
        !cookie.contains("Secure"),
        "ohne cookie_secure kein Secure: {cookie}"
    );
    let user: User = res.into_json().await.unwrap();
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.display_name, "Alice A.");
    assert!(user.last_login_at.is_some());

    let (_, v) = get(&client, "/api/v1/auth/status").await;
    let status: AuthStatus = serde_json::from_value(v).unwrap();
    assert!(status.authenticated && !status.setup_required);
    assert!(
        !status.registration_open,
        "danach ist die Registrierung geschlossen"
    );
    assert_eq!(status.user.map(|u| u.id), Some(user.id));

    // In der Datenbank: Argon2id-Hash, Sitzung nur als SHA-256
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user.id)
        .fetch_one(pool(&client))
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    let token_hash: String =
        sqlx::query_scalar("SELECT token_hash FROM sessions WHERE user_id = ?")
            .bind(user.id)
            .fetch_one(pool(&client))
            .await
            .unwrap();
    assert_eq!(token_hash.len(), 64);
    assert!(token_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!cookie.contains(&token_hash));
    client.terminate().await;
}

#[rocket::async_test]
async fn secure_cookie_when_configured() {
    let (client, _dir) = common::client_with(|f| f.merge(("cookie_secure", true))).await;
    let res = client
        .post("/api/v1/auth/register")
        .header(ContentType::JSON)
        .body(json!({ "username": "alice", "password": PASSWORD }).to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Created);
    let cookie = res.headers().get_one("Set-Cookie").unwrap().to_string();
    assert!(cookie.contains("Secure"), "{cookie}");
    drop(res);
    client.terminate().await;
}

#[rocket::async_test]
async fn registration_is_closed_after_setup_and_opens_when_enabled() {
    let (client, _dir) = common::client().await;
    assert_eq!(
        register(&client, "alice", PASSWORD).await.0,
        Status::Created
    );
    assert_eq!(logout(&client).await, Status::NoContent);

    let (s, v) = register(&client, "bob", PASSWORD).await;
    assert_eq!(s, Status::Forbidden);
    assert_eq!(v["error"]["code"], "forbidden");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Registrierung"));

    // Der Admin öffnet sie
    assert_eq!(login(&client, "alice", PASSWORD).await.0, Status::Ok);
    let (s, v) = put(
        &client,
        "/api/v1/settings/instance",
        json!({ "registrationOpen": true, "refreshMinutes": 30, "retentionDays": 60 }),
    )
    .await;
    assert_eq!(s, Status::Ok, "{v}");
    logout(&client).await;

    let (_, v) = get(&client, "/api/v1/auth/status").await;
    assert_eq!(v["registrationOpen"], true);
    let (s, v) = register(&client, "bob", PASSWORD).await;
    assert_eq!(s, Status::Created, "{v}");
    assert_eq!(v["role"], "member", "nur das erste Konto wird Admin");
    // bob ist gleich angemeldet
    let (s, v) = get(&client, "/api/v1/me").await;
    assert_eq!((s, v["username"].clone()), (Status::Ok, json!("bob")));
    client.terminate().await;
}

#[rocket::async_test]
async fn registration_open_by_configuration() {
    let (client, _dir) = common::client_with(|f| f.merge(("registration_open", true))).await;
    login_as(&client, "alice", true).await;
    logout(&client).await;
    let (_, v) = get(&client, "/api/v1/auth/status").await;
    assert_eq!(v["registrationOpen"], true);
    assert_eq!(register(&client, "bob", PASSWORD).await.0, Status::Created);
    client.terminate().await;
}

#[rocket::async_test]
async fn invalid_registrations_are_rejected() {
    let (client, _dir) = common::client().await;
    for (name, password, needle) in [
        ("a", PASSWORD, "Benutzername"),
        ("mit leerzeichen", PASSWORD, "Benutzername"),
        ("jürgen", PASSWORD, "Benutzername"),
        (&"x".repeat(41) as &str, PASSWORD, "Benutzername"),
        ("alice", "kurz", "Passwort"),
    ] {
        let (s, v) = register(&client, name, password).await;
        assert_eq!(s, Status::BadRequest, "{name}: {v}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains(needle),
            "{v}"
        );
    }
    // Kaputtes JSON → JSON-Fehler
    let res = client
        .post("/api/v1/auth/register")
        .header(ContentType::JSON)
        .body("{ kein json")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::BadRequest);
    let v: Value = res.into_json().await.unwrap();
    assert_eq!(v["error"]["code"], "bad_request");

    // Benutzernamen sind eindeutig, ohne Gross-/Kleinschreibung
    assert_eq!(
        register(&client, "al.ice_2-x", PASSWORD).await.0,
        Status::Created
    );
    let err = users::create(pool(&client), "AL.ICE_2-X", PASSWORD, "", Role::Member)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Status::Conflict);
    client.terminate().await;
}

#[rocket::async_test]
async fn login_and_logout() {
    let (client, _dir) = common::client().await;
    let alice = login_as(&client, "alice", false).await;
    assert!(alice.last_login_at.is_some());
    assert_eq!(get(&client, "/api/v1/me").await.0, Status::Ok);
    assert_eq!(sessions(&client, alice.id).await, 1);

    assert_eq!(logout(&client).await, Status::NoContent);
    assert_eq!(
        sessions(&client, alice.id).await,
        0,
        "Sitzung in der DB gelöscht"
    );
    let (s, v) = get(&client, "/api/v1/me").await;
    assert_eq!(s, Status::Unauthorized);
    assert_eq!(v["error"]["code"], "unauthorized");
    assert_eq!(v["error"]["message"], "Bitte anmelden.");

    // Benutzername ohne Gross-/Kleinschreibung
    let (s, v) = login(&client, "ALICE", PASSWORD).await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(v["id"], alice.id);
    // Abmelden ohne Sitzung schadet nicht
    logout(&client).await;
    assert_eq!(logout(&client).await, Status::NoContent);
    client.terminate().await;
}

#[rocket::async_test]
async fn wrong_password_and_unknown_user_get_the_same_answer() {
    let (client, _dir) = common::client().await;
    login_as(&client, "alice", false).await;
    logout(&client).await;

    let (s1, v1) = login(&client, "alice", "falsches-passwort").await;
    let (s2, v2) = login(&client, "niemand", "falsches-passwort").await;
    assert_eq!(s1, Status::Unauthorized);
    assert_eq!(s2, Status::Unauthorized);
    assert_eq!(v1, v2);
    assert_eq!(v1["error"]["message"], "Benutzername oder Passwort falsch.");
    assert_eq!(get(&client, "/api/v1/me").await.0, Status::Unauthorized);
    client.terminate().await;
}

#[rocket::async_test]
async fn repeated_failures_are_throttled_per_username_and_ip() {
    let (client, _dir) = common::client().await;
    login_as(&client, "alice", false).await;
    login_as(&client, "bob", false).await;
    logout(&client).await;

    let attempt = |user: &'static str, password: &'static str, ip: &'static str| {
        let client = &client;
        async move {
            let res = client
                .post("/api/v1/auth/login")
                .remote(format!("{ip}:5000").parse().unwrap())
                .header(ContentType::JSON)
                .body(json!({ "username": user, "password": password }).to_string())
                .dispatch()
                .await;
            let status = res.status();
            (
                status,
                res.into_json::<Value>().await.unwrap_or(Value::Null),
            )
        }
    };

    for _ in 0..nrznews::auth::throttle::MAX_FAILURES {
        assert_eq!(
            attempt("alice", "falsch-falsch", "10.0.0.1").await.0,
            Status::Unauthorized
        );
    }
    // Jetzt hilft auch das richtige Passwort nicht – bis das Zeitfenster vorbei ist
    let (s, v) = attempt("alice", PASSWORD, "10.0.0.1").await;
    assert_eq!(s, Status::TooManyRequests);
    assert_eq!(v["error"]["code"], "too_many_requests");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("Minuten"),
        "{v}"
    );

    // Andere IP und anderes Konto sind nicht betroffen
    assert_eq!(attempt("alice", PASSWORD, "10.0.0.2").await.0, Status::Ok);
    assert_eq!(attempt("bob", PASSWORD, "10.0.0.1").await.0, Status::Ok);
    client.terminate().await;
}

#[rocket::async_test]
async fn changing_requests_from_foreign_origins_are_rejected() {
    let (client, _dir) = common::client().await;
    login_as(&client, "alice", false).await;

    let put_me = |origin: Option<&'static str>| {
        let client = &client;
        async move {
            let mut req = client
                .put("/api/v1/me")
                .header(Header::new("Host", "reader.example:8340"))
                .header(ContentType::JSON)
                .body(json!({ "displayName": "Alice" }).to_string());
            if let Some(o) = origin {
                req = req.header(Header::new("Origin", o));
            }
            let res = req.dispatch().await;
            let status = res.status();
            (
                status,
                res.into_json::<Value>().await.unwrap_or(Value::Null),
            )
        }
    };

    let (s, v) = put_me(Some("https://boese.example")).await;
    assert_eq!(s, Status::Forbidden);
    assert_eq!(v["error"]["code"], "forbidden");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("fremden Seite"),
        "{v}"
    );

    assert_eq!(
        put_me(Some("http://reader.example:8340")).await.0,
        Status::Ok
    );
    // Ohne Origin (z. B. curl, Skripte mit Cookie) geht es
    assert_eq!(put_me(None).await.0, Status::Ok);

    // Hinter einem Proxy zählt X-Forwarded-Host
    let status = client
        .put("/api/v1/me")
        .header(Header::new("Host", "127.0.0.1:8340"))
        .header(Header::new("X-Forwarded-Host", "rss.example.org"))
        .header(Header::new("Origin", "https://rss.example.org"))
        .header(ContentType::JSON)
        .body("{}")
        .dispatch()
        .await
        .status();
    assert_eq!(status, Status::Ok);

    // Lesen von fremden Seiten ist kein Problem (CORS schützt die Antwort)
    let status = client
        .get("/api/v1/me")
        .header(Header::new("Host", "reader.example:8340"))
        .header(Header::new("Origin", "https://boese.example"))
        .dispatch()
        .await
        .status();
    assert_eq!(status, Status::Ok);

    // Login-CSRF: eine fremde Seite darf niemanden anmelden
    logout(&client).await;
    let res = client
        .post("/api/v1/auth/login")
        .header(Header::new("Host", "reader.example:8340"))
        .header(Header::new("Origin", "https://boese.example"))
        .header(ContentType::JSON)
        .body(json!({ "username": "alice", "password": PASSWORD }).to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Forbidden);
    assert!(res.headers().get_one("Set-Cookie").is_none());
    drop(res);
    client.terminate().await;
}

#[rocket::async_test]
async fn own_account_and_preferences() {
    let (client, _dir) = common::client().await;
    let alice = login_as(&client, "alice", false).await;

    let (s, v) = get(&client, "/api/v1/me").await;
    assert_eq!(s, Status::Ok);
    let me: User = serde_json::from_value(v).unwrap();
    assert_eq!(me.id, alice.id);
    assert_eq!(me.preferences, nrznews_types::Preferences::default());
    assert_eq!(me.name(), "alice");

    let prefs = json!({
        "language": "en", "unreadOnly": false, "markReadOnOpen": false,
        "readerSize": "l", "compactList": true
    });
    let (s, v) = put(
        &client,
        "/api/v1/me",
        json!({ "displayName": "  Alice  ", "preferences": prefs }),
    )
    .await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(v["displayName"], "Alice");
    assert_eq!(v["preferences"], prefs);

    // Nur gesetzte Felder ändern sich
    let (s, v) = put(&client, "/api/v1/me", json!({ "displayName": "A." })).await;
    assert_eq!(s, Status::Ok);
    assert_eq!(v["preferences"], prefs);
    let (_, v) = get(&client, "/api/v1/me").await;
    assert_eq!(
        (v["displayName"].clone(), v["preferences"].clone()),
        (json!("A."), prefs.clone())
    );

    // Fehlende Felder in den Einstellungen bekommen ihre Vorgabe
    let (s, v) = put(
        &client,
        "/api/v1/me",
        json!({ "preferences": { "language": "en" } }),
    )
    .await;
    assert_eq!(s, Status::Ok);
    assert_eq!(v["preferences"]["readerSize"], "m");

    for (bad, needle) in [
        (json!({ "preferences": { "language": "fr" } }), "Sprache"),
        (
            json!({ "preferences": { "readerSize": "xl" } }),
            "Schriftgrösse",
        ),
        (json!({ "displayName": "x".repeat(81) }), "Anzeigename"),
    ] {
        let (s, v) = put(&client, "/api/v1/me", bad).await;
        assert_eq!(s, Status::BadRequest, "{v}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains(needle),
            "{v}"
        );
    }
    // Nicht angemeldet
    logout(&client).await;
    assert_eq!(
        put(&client, "/api/v1/me", json!({})).await.0,
        Status::Unauthorized
    );
    client.terminate().await;
}

#[rocket::async_test]
async fn changing_the_password_ends_other_sessions() {
    let (client, _dir) = common::client().await;
    let alice = login_as(&client, "alice", false).await;
    // Zwei weitere Geräte
    session::create(pool(&client), alice.id, "Handy")
        .await
        .unwrap();
    session::create(pool(&client), alice.id, "Tablet")
        .await
        .unwrap();
    assert_eq!(sessions(&client, alice.id).await, 3);

    let (s, v) = put(
        &client,
        "/api/v1/me/password",
        json!({ "current": "falsch-falsch", "new": "neues-passwort" }),
    )
    .await;
    assert_eq!(s, Status::BadRequest);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("aktuelle Passwort"),
        "{v}"
    );
    let (s, v) = put(
        &client,
        "/api/v1/me/password",
        json!({ "current": PASSWORD, "new": "kurz" }),
    )
    .await;
    assert_eq!(s, Status::BadRequest, "{v}");

    let (s, _) = put(
        &client,
        "/api/v1/me/password",
        json!({ "current": PASSWORD, "new": "neues-passwort" }),
    )
    .await;
    assert_eq!(s, Status::NoContent);
    assert_eq!(
        sessions(&client, alice.id).await,
        1,
        "nur diese Sitzung bleibt"
    );
    assert_eq!(get(&client, "/api/v1/me").await.0, Status::Ok);

    logout(&client).await;
    assert_eq!(
        login(&client, "alice", PASSWORD).await.0,
        Status::Unauthorized
    );
    assert_eq!(
        login(&client, "alice", "neues-passwort").await.0,
        Status::Ok
    );
    client.terminate().await;
}

#[rocket::async_test]
async fn sessions_expire_and_slide() {
    let (client, _dir) = common::client().await;
    let alice = login_as(&client, "alice", false).await;
    let now = nrznews::db::now();

    // Lange nicht benutzt, aber noch gültig: wird beim nächsten Aufruf um 30 Tage verlängert
    sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE user_id = ?")
        .bind(now - 7200)
        .bind(now + 60)
        .bind(alice.id)
        .execute(pool(&client))
        .await
        .unwrap();
    let res = client.get("/api/v1/me").dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    // Das Cookie wird mit verlängert
    let cookie = res
        .headers()
        .get_one("Set-Cookie")
        .unwrap_or_default()
        .to_string();
    assert!(
        cookie.contains("nrznews_session=") && cookie.contains("Max-Age=2592000"),
        "{cookie}"
    );
    drop(res);
    // Innerhalb der nächsten Stunde nicht noch einmal (spart Schreibzugriffe)
    let res = client.get("/api/v1/me").dispatch().await;
    assert!(res.headers().get_one("Set-Cookie").is_none());
    drop(res);
    let (seen, expires): (i64, i64) =
        sqlx::query_as("SELECT last_seen_at, expires_at FROM sessions WHERE user_id = ?")
            .bind(alice.id)
            .fetch_one(pool(&client))
            .await
            .unwrap();
    assert!(seen >= now);
    assert!(expires >= now + session::LIFETIME_SECS);

    // Abgelaufen: abgewiesen, und das Aufräumen löscht sie
    sqlx::query("UPDATE sessions SET expires_at = ? WHERE user_id = ?")
        .bind(now - 1)
        .bind(alice.id)
        .execute(pool(&client))
        .await
        .unwrap();
    assert_eq!(get(&client, "/api/v1/me").await.0, Status::Unauthorized);
    // Der Status (mit dem die Oberfläche startet) entfernt das nutzlose Cookie. (Bei
    // Fehlerantworten verwirft Rocket Cookie-Änderungen, darum nicht schon beim 401.)
    let res = client.get("/api/v1/auth/status").dispatch().await;
    let cookie = res
        .headers()
        .get_one("Set-Cookie")
        .unwrap_or_default()
        .to_string();
    assert!(
        cookie.starts_with("nrznews_session=;") && cookie.contains("Max-Age=0"),
        "{cookie}"
    );
    let v: Value = res.into_json().await.unwrap();
    assert_eq!(v["authenticated"], false);
    assert_eq!(session::purge_expired(pool(&client)).await.unwrap(), 1);
    client.terminate().await;
}

#[rocket::async_test]
async fn forged_cookies_are_ignored() {
    let (client, _dir) = common::client().await;
    let alice = login_as(&client, "alice", false).await;
    logout(&client).await;
    // Ein Token, das in der DB steht, hilft ohne Rockets Verschlüsselung nichts
    let token = session::create(pool(&client), alice.id, "x").await.unwrap();
    let res = client
        .get("/api/v1/me")
        .cookie(rocket::http::Cookie::new(session::COOKIE, token))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
    let (s, _) = call(&client, "GET", "/api/v1/me", None).await;
    assert_eq!(s, Status::Unauthorized);
    let _ = post(&client, "/api/v1/auth/logout", json!({})).await;
    drop(res);
    client.terminate().await;
}

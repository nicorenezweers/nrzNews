//! Verwaltung: Konten anlegen, ändern, sperren, löschen (mit allen Daten), Schutz des
//! eigenen Kontos und des letzten Admins, Rechte (Mitglieder bekommen 403) und die
//! Einstellungen der Instanz.

mod common;

use common::{delete, get, login, login_as, logout, pool, post, put, PASSWORD};
use nrznews::auth::session;
use nrznews::services::users;
use nrznews_types::{InstanceSettings, Role, User};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::json;

async fn count(client: &Client, sql: &'static str, id: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(pool(client))
        .await
        .unwrap()
}

#[rocket::async_test]
async fn admin_manages_accounts() {
    let (client, _dir) = common::client().await;
    let admin = login_as(&client, "admin", true).await;

    let (s, v) = post(
        &client,
        "/api/v1/admin/users",
        json!({ "username": "bob", "password": PASSWORD, "displayName": "Bob B." }),
    )
    .await;
    assert_eq!(s, Status::Created, "{v}");
    let bob: User = serde_json::from_value(v).unwrap();
    assert_eq!((bob.role, bob.disabled), (Role::Member, false));
    assert_eq!(bob.name(), "Bob B.");

    // Doppelter Name → 409, ungültige Eingaben → 400
    let (s, v) = post(
        &client,
        "/api/v1/admin/users",
        json!({ "username": "BOB", "password": PASSWORD }),
    )
    .await;
    assert_eq!(s, Status::Conflict);
    assert_eq!(v["error"]["code"], "conflict");
    let (s, _) = post(
        &client,
        "/api/v1/admin/users",
        json!({ "username": "carl", "password": "kurz" }),
    )
    .await;
    assert_eq!(s, Status::BadRequest);
    let (s, v) = post(
        &client,
        "/api/v1/admin/users",
        json!({ "username": "carl", "password": PASSWORD, "role": "admin" }),
    )
    .await;
    assert_eq!(s, Status::Created);
    assert_eq!(v["role"], "admin");

    let (s, v) = get(&client, "/api/v1/admin/users").await;
    assert_eq!(s, Status::Ok);
    let list: Vec<User> = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(
        list.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(),
        ["admin", "bob", "carl"]
    );
    assert!(
        !v.to_string().contains("argon2"),
        "kein Passwort-Hash in Antworten"
    );

    // Ändern: nur gesetzte Felder
    let url = format!("/api/v1/admin/users/{}", bob.id);
    let (s, v) = put(
        &client,
        &url,
        json!({ "displayName": "Robert", "role": "admin" }),
    )
    .await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(
        (v["displayName"].clone(), v["role"].clone()),
        (json!("Robert"), json!("admin"))
    );
    let (s, v) = put(&client, &url, json!({ "role": "member" })).await;
    assert_eq!((s, v["displayName"].clone()), (Status::Ok, json!("Robert")));

    // Unbekannte Konten
    assert_eq!(
        put(&client, "/api/v1/admin/users/999", json!({})).await.0,
        Status::NotFound
    );
    assert_eq!(
        delete(&client, "/api/v1/admin/users/999").await.0,
        Status::NotFound
    );
    let (s, v) = get(&client, "/api/v1/admin/users/abc").await;
    assert_eq!(s, Status::NotFound, "{v}");
    assert_eq!(v["error"]["code"], "not_found");

    assert_eq!(admin.role, Role::Admin);
    client.terminate().await;
}

#[rocket::async_test]
async fn disabling_and_password_reset_end_sessions() {
    let (client, _dir) = common::client().await;
    let bob = login_as(&client, "bob", false).await;
    logout(&client).await;
    let admin = login_as(&client, "admin", true).await;
    session::create(pool(&client), bob.id, "Bobs Handy")
        .await
        .unwrap();
    let bob_sessions = "SELECT count(*) FROM sessions WHERE user_id = ?";
    assert_eq!(count(&client, bob_sessions, bob.id).await, 1);

    // Leeres Passwortfeld ändert nichts
    let url = format!("/api/v1/admin/users/{}", bob.id);
    let (s, v) = put(
        &client,
        &url,
        json!({ "password": "", "displayName": "Bob" }),
    )
    .await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(count(&client, bob_sessions, bob.id).await, 1);

    // Neues Passwort durch den Admin: Bobs Sitzungen enden
    let (s, v) = put(&client, &url, json!({ "password": "neu-gesetzt" })).await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(count(&client, bob_sessions, bob.id).await, 0);
    assert_eq!(
        put(&client, &url, json!({ "password": "kurz" })).await.0,
        Status::BadRequest
    );

    // Sperren: Sitzungen enden, Anmeldung geht nicht mehr
    session::create(pool(&client), bob.id, "Bobs Handy")
        .await
        .unwrap();
    let (s, v) = put(&client, &url, json!({ "disabled": true })).await;
    assert_eq!((s, v["disabled"].clone()), (Status::Ok, json!(true)));
    assert_eq!(count(&client, bob_sessions, bob.id).await, 0);
    logout(&client).await;
    let (s, v) = login(&client, "bob", "neu-gesetzt").await;
    assert_eq!(s, Status::Forbidden);
    assert!(
        v["error"]["message"].as_str().unwrap().contains("gesperrt"),
        "{v}"
    );
    // … und eine Sitzung, die es noch gäbe, gilt nicht
    let token = session::create(pool(&client), bob.id, "x").await.unwrap();
    assert!(session::lookup(pool(&client), &token)
        .await
        .unwrap()
        .is_none());

    // Entsperren
    assert_eq!(login(&client, "admin", PASSWORD).await.0, Status::Ok);
    assert_eq!(
        put(&client, &url, json!({ "disabled": false })).await.0,
        Status::Ok
    );
    logout(&client).await;
    assert_eq!(login(&client, "bob", "neu-gesetzt").await.0, Status::Ok);
    assert_eq!(admin.role, Role::Admin);
    client.terminate().await;
}

#[rocket::async_test]
async fn admins_cannot_lock_themselves_out() {
    let (client, _dir) = common::client().await;
    let admin = login_as(&client, "admin", true).await;
    let me = format!("/api/v1/admin/users/{}", admin.id);

    for (body, needle) in [
        (json!({ "role": "member" }), "Administratorrechte"),
        (json!({ "disabled": true }), "sperren"),
        (json!({ "password": "anderes-passwort" }), "„Konto“"),
    ] {
        let (s, v) = put(&client, &me, body).await;
        assert_eq!(s, Status::BadRequest, "{v}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains(needle),
            "{v}"
        );
    }
    let (s, v) = delete(&client, &me).await;
    assert_eq!(s, Status::BadRequest);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("eigenes Konto"),
        "{v}"
    );
    // Harmlose Änderungen am eigenen Konto gehen
    assert_eq!(
        put(
            &client,
            &me,
            json!({ "displayName": "Chefin", "role": "admin" })
        )
        .await
        .0,
        Status::Ok
    );

    // Der letzte aktive Admin bleibt – auch wenn die Fachlogik von anderswo gerufen wird
    let member = users::create(pool(&client), "mia", PASSWORD, "", Role::Member)
        .await
        .unwrap();
    let err = users::delete(pool(&client), &member, admin.id)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Status::Conflict);
    let demote = nrznews_types::UpdateUserInput {
        role: Some(Role::Member),
        ..Default::default()
    };
    let err = users::update(pool(&client), &member, admin.id, &demote)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Status::Conflict);
    let disable = nrznews_types::UpdateUserInput {
        disabled: Some(true),
        ..Default::default()
    };
    let err = users::update(pool(&client), &member, admin.id, &disable)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Status::Conflict);

    // Mit einem zweiten aktiven Admin geht es
    let second = users::create(pool(&client), "zweite", PASSWORD, "", Role::Admin)
        .await
        .unwrap();
    let (s, v) = put(
        &client,
        format!("/api/v1/admin/users/{}", second.id),
        json!({ "role": "member" }),
    )
    .await;
    assert_eq!((s, v["role"].clone()), (Status::Ok, json!("member")));
    let (s, _) = put(
        &client,
        format!("/api/v1/admin/users/{}", second.id),
        json!({ "role": "admin" }),
    )
    .await;
    assert_eq!(s, Status::Ok);
    assert_eq!(
        delete(&client, format!("/api/v1/admin/users/{}", second.id))
            .await
            .0,
        Status::NoContent
    );
    client.terminate().await;
}

#[rocket::async_test]
async fn deleting_an_account_removes_all_its_data() {
    let (client, _dir) = common::client().await;
    let bob = login_as(&client, "bob", false).await;
    let other = users::create(pool(&client), "eve", PASSWORD, "", Role::Member)
        .await
        .unwrap();
    logout(&client).await;
    login_as(&client, "admin", true).await;

    // Daten von bob und eve direkt anlegen
    let db = pool(&client);
    for (uid, word) in [(bob.id, "bobwort"), (other.id, "evewort")] {
        let folder: i64 = sqlx::query_scalar(
            "INSERT INTO folders (user_id, name, created_at) VALUES (?, 'Technik', 0) RETURNING id",
        )
        .bind(uid)
        .fetch_one(db)
        .await
        .unwrap();
        let feed: i64 = sqlx::query_scalar(
            "INSERT INTO feeds (user_id, folder_id, feed_url, created_at) VALUES (?, ?, 'https://example.org/feed', 0) RETURNING id",
        )
        .bind(uid)
        .bind(folder)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO articles (feed_id, user_id, guid, title, content_text, published_at, fetched_at)
             VALUES (?, ?, 'g1', 'Titel', ?, 0, 0)",
        )
        .bind(feed)
        .bind(uid)
        .bind(word)
        .execute(db)
        .await
        .unwrap();
    }
    session::create(db, bob.id, "x").await.unwrap();

    assert_eq!(
        delete(&client, format!("/api/v1/admin/users/{}", bob.id))
            .await
            .0,
        Status::NoContent
    );
    for sql in [
        "SELECT count(*) FROM users WHERE id = ?",
        "SELECT count(*) FROM sessions WHERE user_id = ?",
        "SELECT count(*) FROM folders WHERE user_id = ?",
        "SELECT count(*) FROM feeds WHERE user_id = ?",
        "SELECT count(*) FROM articles WHERE user_id = ?",
    ] {
        assert_eq!(count(&client, sql, bob.id).await, 0, "{sql}");
        if !sql.contains("sessions") {
            assert_eq!(count(&client, sql, other.id).await, 1, "eve behält: {sql}");
        }
    }
    // Volltextindex ohne Reste
    let hits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM articles_fts WHERE articles_fts MATCH 'bobwort'")
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(hits, 0);
    let hits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM articles_fts WHERE articles_fts MATCH 'evewort'")
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(hits, 1);
    client.terminate().await;
}

#[rocket::async_test]
async fn members_and_guests_cannot_use_admin_routes() {
    let (client, _dir) = common::client().await;
    login_as(&client, "admin", true).await;
    logout(&client).await;
    login_as(&client, "mia", false).await;

    let routes: [(&str, &str); 7] = [
        ("GET", "/api/v1/admin/users"),
        ("POST", "/api/v1/admin/users"),
        ("PUT", "/api/v1/admin/users/1"),
        ("DELETE", "/api/v1/admin/users/1"),
        ("GET", "/api/v1/settings/instance"),
        ("PUT", "/api/v1/settings/instance"),
        ("GET", "/api/v1/admin/users"),
    ];
    let body = || Some(json!({ "username": "x", "password": PASSWORD, "registrationOpen": true }));
    for (method, url) in routes {
        let (s, v) = common::call(&client, method, url, body()).await;
        assert_eq!(s, Status::Forbidden, "{method} {url}: {v}");
        assert_eq!(
            v["error"]["message"], "Nur für Administratoren.",
            "{method} {url}"
        );
    }
    logout(&client).await;
    for (method, url) in routes {
        let (s, v) = common::call(&client, method, url, body()).await;
        assert_eq!(s, Status::Unauthorized, "{method} {url}: {v}");
    }
    // Nichts hat sich geändert
    assert_eq!(users::count(pool(&client)).await.unwrap(), 2);
    client.terminate().await;
}

#[rocket::async_test]
async fn instance_settings_are_validated_and_saved() {
    let (client, _dir) = common::client().await;
    login_as(&client, "admin", true).await;

    let (s, v) = get(&client, "/api/v1/settings/instance").await;
    assert_eq!(s, Status::Ok);
    let settings: InstanceSettings = serde_json::from_value(v).unwrap();
    assert_eq!(settings, InstanceSettings::default());

    for (bad, needle) in [
        (
            json!({ "registrationOpen": false, "refreshMinutes": 4, "retentionDays": 60 }),
            "Abrufintervall",
        ),
        (
            json!({ "registrationOpen": false, "refreshMinutes": 1441, "retentionDays": 60 }),
            "Abrufintervall",
        ),
        (
            json!({ "registrationOpen": false, "refreshMinutes": 30, "retentionDays": 3651 }),
            "Aufbewahrung",
        ),
    ] {
        let (s, v) = put(&client, "/api/v1/settings/instance", bad).await;
        assert_eq!(s, Status::BadRequest, "{v}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains(needle),
            "{v}"
        );
    }
    // Negative Zahlen sind gar kein gültiges JSON für u32 → 422
    let (s, v) = put(
        &client,
        "/api/v1/settings/instance",
        json!({ "refreshMinutes": -1 }),
    )
    .await;
    assert_eq!(s, Status::UnprocessableEntity, "{v}");
    assert_eq!(v["error"]["code"], "unprocessable");

    let wanted = json!({ "registrationOpen": true, "refreshMinutes": 5, "retentionDays": 0 });
    let (s, v) = put(&client, "/api/v1/settings/instance", wanted.clone()).await;
    assert_eq!((s, v.clone()), (Status::Ok, wanted.clone()));
    assert_eq!(get(&client, "/api/v1/settings/instance").await.1, wanted);
    // Fehlende Felder bekommen ihre Vorgabe
    let (s, v) = put(
        &client,
        "/api/v1/settings/instance",
        json!({ "refreshMinutes": 1440 }),
    )
    .await;
    assert_eq!(s, Status::Ok);
    assert_eq!(
        v,
        json!({ "registrationOpen": false, "refreshMinutes": 1440, "retentionDays": 60 })
    );
    client.terminate().await;
}

//! Artikel: Liste mit Ansichten, Feed/Ordner, Volltextsuche und Keyset-Cursor, Lesen,
//! Gelesen/Gemerkt, „alles gelesen“ (mit `before`), Trennung der Benutzer und die HTTP-API.

mod common;

use nrznews::config::Config;
use nrznews::error::AppError;
use nrznews::services::articles::{self, ListQuery};
use nrznews_types::{ArticleStateInput, MarkReadInput, View};
use rocket::http::Status;
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::HashSet;

async fn pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        background: false,
        ..Config::default()
    };
    let pool = nrznews::db::connect(&config).await.unwrap();
    (dir, pool)
}

async fn user(pool: &SqlitePool, name: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, created_at) VALUES (?, 'x', 0) RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn folder(pool: &SqlitePool, user: i64, name: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO folders (user_id, name, created_at) VALUES (?, ?, 0) RETURNING id",
    )
    .bind(user)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn feed(pool: &SqlitePool, user: i64, folder: Option<i64>, title: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO feeds (user_id, folder_id, title, feed_url, icon_url, created_at)
         VALUES (?, ?, ?, ?, 'https://x.example/i.png', 0) RETURNING id",
    )
    .bind(user)
    .bind(folder)
    .bind(title)
    .bind(format!("https://x.example/{title}"))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Artikel anlegen; gibt die ID zurück.
async fn article(
    pool: &SqlitePool,
    feed: i64,
    user: i64,
    title: &str,
    text: &str,
    published: i64,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO articles (feed_id, user_id, guid, url, title, content_html, content_text, published_at, fetched_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(feed)
    .bind(user)
    .bind(format!("{feed}-{title}-{published}"))
    .bind(format!("https://x.example/a/{title}"))
    .bind(title)
    .bind(format!("<p>{text}</p>"))
    .bind(text)
    .bind(published)
    .bind(published)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn set(pool: &SqlitePool, id: i64, read: bool, starred: bool) {
    sqlx::query("UPDATE articles SET read = ?, starred = ? WHERE id = ?")
        .bind(read)
        .bind(starred)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

fn q() -> ListQuery {
    ListQuery::default()
}

#[tokio::test]
async fn list_pages_with_cursor_and_filters() {
    let (_d, pool) = pool().await;
    let alice = user(&pool, "alice").await;
    let tech = folder(&pool, alice, "Technik").await;
    let a = feed(&pool, alice, Some(tech), "A").await;
    let b = feed(&pool, alice, None, "B").await;
    let t = nrznews::db::now();
    let mut ids = Vec::new();
    for i in 0..45 {
        // Alle 3 Artikel derselbe Zeitpunkt: der Cursor muss Gleichstände (per ID) auflösen
        ids.push(
            article(
                &pool,
                a,
                alice,
                &format!("A{i}"),
                "Text",
                t - 7000 * (i / 3),
            )
            .await,
        );
    }
    for i in 0..5 {
        ids.push(
            article(
                &pool,
                b,
                alice,
                &format!("B{i}"),
                "Text",
                t - 10 * 86400 - i,
            )
            .await,
        );
    }
    set(&pool, ids[0], true, true).await;
    set(&pool, ids[1], true, false).await;
    set(&pool, ids[46], false, true).await;

    // Vorgabe 40, dann der Rest
    let p1 = articles::list(&pool, alice, &q()).await.unwrap();
    assert_eq!(p1.items.len(), 40);
    let cursor = p1.next_cursor.clone().expect("weitere Seite");
    let p2 = articles::list(
        &pool,
        alice,
        &ListQuery {
            cursor: Some(cursor),
            ..q()
        },
    )
    .await
    .unwrap();
    assert_eq!(p2.items.len(), 10);
    assert!(p2.next_cursor.is_none());
    let all: Vec<_> = p1.items.iter().chain(&p2.items).collect();
    let unique: HashSet<i64> = all.iter().map(|a| a.id).collect();
    assert_eq!(unique.len(), 50, "keine doppelten, keine fehlenden");
    assert!(
        all.windows(2)
            .all(|w| (w[0].published_at, w[0].id) > (w[1].published_at, w[1].id)),
        "neueste zuerst"
    );
    assert_eq!(all[0].feed_title, "A");
    assert_eq!(all[0].feed_icon, "https://x.example/i.png");

    // Kleine Seiten über Gleichstände hinweg
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let p = articles::list(
            &pool,
            alice,
            &ListQuery {
                limit: Some(7),
                cursor: cursor.clone(),
                ..q()
            },
        )
        .await
        .unwrap();
        assert!(p.items.len() <= 7);
        seen.extend(p.items.iter().map(|a| a.id));
        match p.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len(), 50);
    assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 50);

    // Höchstens 100
    let big = articles::list(
        &pool,
        alice,
        &ListQuery {
            limit: Some(1000),
            ..q()
        },
    )
    .await
    .unwrap();
    assert_eq!((big.items.len(), big.next_cursor.is_none()), (50, true));

    // Ansichten, Feed, Ordner
    let count = |query: ListQuery| {
        let pool = pool.clone();
        async move {
            articles::list(
                &pool,
                alice,
                &ListQuery {
                    limit: Some(100),
                    ..query
                },
            )
            .await
            .unwrap()
            .items
            .len()
        }
    };
    assert_eq!(
        count(ListQuery {
            view: View::Unread,
            ..q()
        })
        .await,
        48
    );
    assert_eq!(
        count(ListQuery {
            view: View::Starred,
            ..q()
        })
        .await,
        2
    );
    assert_eq!(
        count(ListQuery {
            view: View::Today,
            ..q()
        })
        .await,
        39,
        "13 Zeitpunkte × 3 Artikel"
    );
    assert_eq!(
        count(ListQuery {
            feed: Some(b),
            ..q()
        })
        .await,
        5
    );
    assert_eq!(
        count(ListQuery {
            folder: Some(tech),
            ..q()
        })
        .await,
        45
    );
    assert_eq!(
        count(ListQuery {
            folder: Some(tech),
            view: View::Starred,
            ..q()
        })
        .await,
        1
    );

    // Kaputter Cursor: 400
    assert!(matches!(
        articles::list(
            &pool,
            alice,
            &ListQuery {
                cursor: Some("x".into()),
                ..q()
            }
        )
        .await,
        Err(AppError::BadRequest(_))
    ));

    // Bob sieht nichts davon – auch nicht mit Alices Feed- oder Ordner-ID
    let bob = user(&pool, "bob").await;
    assert!(articles::list(&pool, bob, &q())
        .await
        .unwrap()
        .items
        .is_empty());
    assert!(articles::list(
        &pool,
        bob,
        &ListQuery {
            feed: Some(a),
            ..q()
        }
    )
    .await
    .unwrap()
    .items
    .is_empty());
    assert!(articles::list(
        &pool,
        bob,
        &ListQuery {
            folder: Some(tech),
            ..q()
        }
    )
    .await
    .unwrap()
    .items
    .is_empty());
}

#[tokio::test]
async fn search_with_prefixes_umlauts_and_safe_syntax() {
    let (_d, pool) = pool().await;
    let alice = user(&pool, "alice").await;
    let bob = user(&pool, "bob").await;
    let fa = feed(&pool, alice, None, "A").await;
    let fb = feed(&pool, bob, None, "B").await;
    let t = nrznews::db::now();
    let apfel = article(&pool, fa, alice, "Grüne Äpfel", "Ernte im Herbst", t - 10).await;
    let rocket = article(
        &pool,
        fa,
        alice,
        "Rocket 0.5 erschienen",
        "Rust und Rocket, endlich stabil",
        t - 20,
    )
    .await;
    article(&pool, fa, alice, "Wetter", "Regen am Wochenende", t - 30).await;
    article(&pool, fb, bob, "Bobs Äpfel", "Rust bei Bob", t - 5).await;

    let search = |s: &str| {
        let pool = pool.clone();
        let s = s.to_string();
        async move {
            articles::list(
                &pool,
                alice,
                &ListQuery {
                    q: Some(s),
                    ..ListQuery::default()
                },
            )
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(search("apfel").await, vec![apfel], "Umlaute gefaltet");
    assert_eq!(search("ÄPFEL").await, vec![apfel]);
    assert_eq!(search("rock").await, vec![rocket], "Präfix");
    assert_eq!(search("rust stabil").await, vec![rocket], "alle Wörter");
    assert!(search("rust regen").await.is_empty());
    assert_eq!(
        search("rust").await,
        vec![rocket],
        "Bobs Artikel bleiben bei Bob"
    );
    // Eingaben, die FTS5-Syntax wären, gehen nie schief
    for s in [
        "\"",
        "(",
        "rust OR",
        "NEAR(a b)",
        "-rust",
        "a*b",
        "title:rust",
        "^",
        "'",
        "*",
        "AND",
        "\"rust\"",
    ] {
        let r = articles::list(
            &pool,
            alice,
            &ListQuery {
                q: Some(s.into()),
                ..ListQuery::default()
            },
        )
        .await;
        assert!(r.is_ok(), "{s}: {:?}", r.err());
    }
    assert!(search("  ").await.len() == 3, "leere Suche = keine Suche");
}

#[tokio::test]
async fn read_state_and_mark_read() {
    let (_d, pool) = pool().await;
    let alice = user(&pool, "alice").await;
    let bob = user(&pool, "bob").await;
    let tech = folder(&pool, alice, "Technik").await;
    let fa = feed(&pool, alice, Some(tech), "A").await;
    let fb = feed(&pool, alice, None, "B").await;
    let fbob = feed(&pool, bob, None, "Bob").await;
    let t = nrznews::db::now();
    let long = "Wort ".repeat(450);
    let a1 = article(&pool, fa, alice, "a1", &long, t - 10).await;
    let a2 = article(&pool, fa, alice, "a2", "kurz", t - 2 * 86400).await;
    let b1 = article(&pool, fb, alice, "b1", "x", t - 20).await;
    let b2 = article(&pool, fb, alice, "b2", "x", t - 3 * 86400).await;
    let bobs = article(&pool, fbob, bob, "bob", "x", t - 30).await;

    // Lesen
    let a = articles::get(&pool, alice, a1).await.unwrap();
    assert_eq!(
        (
            a.reading_minutes,
            a.feed_title.as_str(),
            a.ai_summary.clone()
        ),
        (3, "A", None)
    );
    assert_eq!(a.content_html, format!("<p>{long}</p>"));
    let list = articles::list(&pool, alice, &ListQuery::default())
        .await
        .unwrap();
    let excerpt = &list.items.iter().find(|i| i.id == a1).unwrap().excerpt;
    assert!(
        excerpt.chars().count() <= 301 && excerpt.ends_with('…'),
        "{excerpt}"
    );
    assert!(matches!(
        articles::get(&pool, bob, a1).await,
        Err(AppError::NotFound)
    ));

    // Gelesen / gemerkt – nur gesetzte Felder
    let s = articles::set_state(
        &pool,
        alice,
        a1,
        &ArticleStateInput {
            read: Some(true),
            starred: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((s.read, s.starred), (true, false));
    let s = articles::set_state(
        &pool,
        alice,
        a1,
        &ArticleStateInput {
            read: None,
            starred: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!((s.read, s.starred), (true, true));
    let (read_at, starred_at): (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT read_at, starred_at FROM articles WHERE id = ?")
            .bind(a1)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(read_at.is_some() && starred_at.is_some());
    let s = articles::set_state(
        &pool,
        alice,
        a1,
        &ArticleStateInput {
            read: Some(false),
            starred: Some(false),
        },
    )
    .await
    .unwrap();
    assert_eq!((s.read, s.starred), (false, false));
    assert!(matches!(
        articles::set_state(
            &pool,
            bob,
            a1,
            &ArticleStateInput {
                read: Some(true),
                starred: None
            }
        )
        .await,
        Err(AppError::NotFound)
    ));

    let mark = |input: MarkReadInput, user: i64| {
        let pool = pool.clone();
        async move {
            articles::mark_read(&pool, user, &input)
                .await
                .unwrap()
                .updated
        }
    };
    // Bob markiert alles – Alice bleibt ungelesen
    assert_eq!(mark(MarkReadInput::default(), bob).await, 1);
    // Nur bis `before`
    assert_eq!(
        mark(
            MarkReadInput {
                feed_id: Some(fb),
                before: Some(t - 86400),
                ..Default::default()
            },
            alice
        )
        .await,
        1
    );
    assert!(articles::get(&pool, alice, b2).await.unwrap().read);
    assert!(!articles::get(&pool, alice, b1).await.unwrap().read);
    // Gemerkte
    articles::set_state(
        &pool,
        alice,
        a2,
        &ArticleStateInput {
            starred: Some(true),
            read: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        mark(
            MarkReadInput {
                view: Some(View::Starred),
                ..Default::default()
            },
            alice
        )
        .await,
        1
    );
    // Heute, im Ordner
    assert_eq!(
        mark(
            MarkReadInput {
                folder_id: Some(tech),
                view: Some(View::Today),
                ..Default::default()
            },
            alice
        )
        .await,
        1
    );
    assert!(articles::get(&pool, alice, a1).await.unwrap().read);
    // Fremder Ordner: nichts
    assert_eq!(
        mark(
            MarkReadInput {
                folder_id: Some(tech),
                ..Default::default()
            },
            bob
        )
        .await,
        0
    );
    // Rest
    assert_eq!(mark(MarkReadInput::default(), alice).await, 1);
    assert_eq!(mark(MarkReadInput::default(), alice).await, 0);
    assert!(articles::get(&pool, bob, bobs).await.unwrap().read);
}

#[rocket::async_test]
async fn http_articles() {
    let (c, _dir) = common::client().await;
    assert_eq!(
        common::get(&c, "/api/v1/articles").await.0,
        Status::Unauthorized
    );
    let alice = common::login_as(&c, "alice", false).await;
    let pool = common::pool(&c);
    let fa = feed(pool, alice.id, None, "A").await;
    let t = nrznews::db::now();
    let mut ids = Vec::new();
    for i in 0..3 {
        ids.push(
            article(
                pool,
                fa,
                alice.id,
                &format!("Artikel {i}"),
                "Inhalt über Rocket",
                t - i * 60,
            )
            .await,
        );
    }

    let (st, page) = common::get(&c, "/api/v1/articles?view=unread&limit=2").await;
    assert_eq!(st, Status::Ok, "{page}");
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["items"][0]["feedTitle"], "A");
    assert_eq!(page["items"][0]["excerpt"], "Inhalt über Rocket");
    let cursor = page["nextCursor"].as_str().unwrap().to_string();
    let (_, page) = common::get(
        &c,
        format!("/api/v1/articles?view=unread&limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert!(page["nextCursor"].is_null());
    let (st, _) = common::get(&c, "/api/v1/articles?cursor=kaputt").await;
    assert_eq!(st, Status::BadRequest);
    let (_, found) = common::get(&c, "/api/v1/articles?q=rock").await;
    assert_eq!(found["items"].as_array().unwrap().len(), 3);
    let (_, found) = common::get(&c, format!("/api/v1/articles?feed={fa}&view=starred")).await;
    assert_eq!(found["items"].as_array().unwrap().len(), 0);

    let (st, a) = common::get(&c, format!("/api/v1/articles/{}", ids[0])).await;
    assert_eq!(st, Status::Ok);
    assert_eq!(
        (a["readingMinutes"].clone(), a["read"].clone()),
        (json!(1), json!(false))
    );
    let (st, s) = common::put(
        &c,
        format!("/api/v1/articles/{}", ids[0]),
        json!({ "starred": true }),
    )
    .await;
    assert_eq!(
        (st, s["starred"].clone(), s["read"].clone()),
        (Status::Ok, json!(true), json!(false))
    );
    let (st, r) = common::post(
        &c,
        "/api/v1/articles/mark-read",
        json!({ "before": t - 30 }),
    )
    .await;
    assert_eq!((st, r["updated"].clone()), (Status::Ok, json!(2)));
    let (_, ov) = common::get(&c, "/api/v1/overview").await;
    assert_eq!(
        (
            ov["unread"].clone(),
            ov["starred"].clone(),
            ov["today"].clone()
        ),
        (json!(1), json!(1), json!(1))
    );

    // Bob: nichts von Alice
    common::logout(&c).await;
    common::login_as(&c, "bob", false).await;
    assert_eq!(
        common::get(&c, format!("/api/v1/articles/{}", ids[0]))
            .await
            .0,
        Status::NotFound
    );
    assert_eq!(
        common::put(
            &c,
            format!("/api/v1/articles/{}", ids[0]),
            json!({ "read": true })
        )
        .await
        .0,
        Status::NotFound
    );
    let (_, page) = common::get(&c, "/api/v1/articles").await;
    assert!(page["items"].as_array().unwrap().is_empty());
    let (_, r) = common::post(&c, "/api/v1/articles/mark-read", json!({})).await;
    assert_eq!(r["updated"], 0);
}

//! Feeds: Lesen (RSS 2.0, Atom, JSON Feed), Säubern, Finden und Abrufen gegen einen lokalen
//! Server (inklusive 304), Abonnieren, Aktualisieren (neue Artikel, Gelesen bleibt), Fehler
//! mit Wartezeit, Aufräumen, OPML hin und zurück, Trennung der Benutzer und die HTTP-API.

mod common;
#[path = "fixtures/server.rs"]
mod server;

use nrznews::config::Config;
use nrznews::error::AppError;
use nrznews::feeds::fetch::{Fetched, Fetcher};
use nrznews::feeds::{discover, parse, refresher, sanitize};
use nrznews::services::{articles, feeds, folders};
use nrznews_types::{ArticleStateInput, SubscribeInput, UpdateFeedInput};
use rocket::http::{ContentType, Status};
use serde_json::json;
use server::{fixture, rss, TestServer};
use sqlx::SqlitePool;
use time::macros::datetime;
use url::Url;

/// Fester Zeitpunkt für die Parser-Tests (nach allen Daten in den Fixtures).
const NOW: i64 = 1_790_000_000;

struct Env {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    config: Config,
    fetcher: Fetcher,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        background: false,
        fetch_timeout_secs: 5,
        ..Config::default()
    };
    let pool = nrznews::db::connect(&config).await.unwrap();
    let fetcher = Fetcher::from_config(&config);
    Env {
        _dir: dir,
        pool,
        config,
        fetcher,
    }
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

fn sub(url: &str) -> SubscribeInput {
    SubscribeInput {
        url: url.into(),
        folder_id: None,
        title: String::new(),
    }
}

// ------------------------------------------------------------------ Lesen

#[test]
fn parses_rss2_with_cdata_relative_urls_and_missing_dates() {
    let feed = parse::parse(
        fixture("rss2.xml").as_bytes(),
        "https://example.org/feed.xml",
        NOW,
    )
    .unwrap();
    assert_eq!(feed.kind, "rss");
    assert_eq!(feed.title, "Beispiel & Blog");
    assert_eq!(feed.site_url, "https://example.org/");
    assert_eq!(feed.description, "Ein Blog über Rust und Äpfel");
    assert_eq!(feed.entries.len(), 5);

    let first = &feed.entries[0];
    assert_eq!(first.guid, "post-1");
    assert_eq!(
        first.title, "Erster Beitrag & mehr",
        "Tags weg, Entities aufgelöst"
    );
    assert_eq!(
        first.url, "https://example.org/posts/erster",
        "relativer Link aufgelöst"
    );
    assert_eq!(first.author, "Anna Muster");
    assert_eq!(
        first.published_at,
        datetime!(2025-06-10 09:00 UTC).unix_timestamp()
    );
    assert_eq!(
        first.image_url, "https://cdn.example.org/thumb-1.jpg",
        "media:thumbnail zuerst"
    );
    let html = &first.content_html;
    assert!(html.contains("<b>Welt</b>"), "{html}");
    assert!(
        !html.contains("Kurzfassung"),
        "content:encoded vor description"
    );
    assert!(
        html.contains(r#"href="https://example.org/about""#),
        "{html}"
    );
    assert!(
        html.contains(r#"target="_blank""#) && html.contains(r#"rel="noopener noreferrer""#),
        "{html}"
    );
    assert!(
        html.contains(r#"src="https://example.org/posts/bilder/foto.jpg""#),
        "{html}"
    );
    assert!(html.contains(r#"loading="lazy""#), "{html}");
    for bad in [
        "<script",
        "alert",
        "<style",
        "color: red",
        "<iframe",
        "onclick",
        "pixel.gif",
        "<form",
        "<input",
        "Senden",
        "style=",
    ] {
        assert!(!html.contains(bad), "{bad} in {html}");
    }
    assert!(
        first
            .content_text
            .starts_with("Hallo Welt! Siehe über uns."),
        "{}",
        first.content_text
    );
    assert!(!first.content_text.contains("böse"));

    let second = &feed.entries[1];
    assert_eq!(
        second.guid, "https://example.org/posts/zweiter",
        "ohne GUID: der Link"
    );
    assert_eq!(second.published_at, NOW, "ohne Datum: jetzt");
    assert_eq!(second.author, "Bob");
    assert_eq!(
        second.image_url, "https://example.org/img/zwei.png",
        "erstes Bild im Inhalt"
    );
    assert!(second.content_html.contains("<i>Zusammenfassung</i>"));

    let podcast = &feed.entries[2];
    assert_eq!(
        podcast.url, "https://example.org/podcast/7",
        "GUID als Permalink"
    );
    assert!(
        podcast.content_html.contains("<audio"),
        "{}",
        podcast.content_html
    );
    assert!(podcast
        .content_html
        .contains(r#"src="https://media.example.org/folge-7.mp3""#));
    assert_eq!(
        podcast.published_at,
        datetime!(2025-06-11 16:30 UTC).unix_timestamp()
    );

    assert_eq!(
        feed.entries[3].published_at, NOW,
        "Zukunft wird auf jetzt gekürzt"
    );

    let untitled = &feed.entries[4];
    assert!(
        untitled.title.starts_with("Ein Eintrag ganz ohne Titel"),
        "{}",
        untitled.title
    );
    assert_eq!(untitled.guid.len(), 64, "SHA-256 als Ersatz-ID");
    // Die Ersatz-ID hängt nicht vom Abrufzeitpunkt ab – sonst käme der Eintrag immer neu
    let again = parse::parse(
        fixture("rss2.xml").as_bytes(),
        "https://example.org/feed.xml",
        NOW + 999,
    )
    .unwrap();
    assert_eq!(again.entries[4].guid, untitled.guid);
}

#[test]
fn parses_atom_with_xml_base_and_dates() {
    let feed = parse::parse(
        fixture("atom.xml").as_bytes(),
        "https://atom.example.com/blog/atom.xml",
        NOW,
    )
    .unwrap();
    assert_eq!(feed.kind, "atom");
    assert_eq!(feed.title, "Atom Journal");
    assert_eq!(feed.site_url, "https://atom.example.com/blog/");
    assert_eq!(feed.icon_url, "https://atom.example.com/favicon.png");

    let e = &feed.entries[0];
    assert_eq!(e.title, "Relative Links");
    assert_eq!(e.url, "https://atom.example.com/blog/eintrag-1.html");
    assert_eq!(e.author, "Carla", "Autor des Feeds als Ersatz");
    assert_eq!(
        e.published_at,
        datetime!(2025-06-12 08:00 UTC).unix_timestamp(),
        "nur updated"
    );
    assert!(
        !e.content_html.contains("Nur die Zusammenfassung"),
        "content vor summary"
    );
    assert!(
        e.content_html
            .contains(r#"href="https://atom.example.com/blog/anderer.html""#),
        "{}",
        e.content_html
    );
    assert!(
        e.content_html
            .contains(r#"src="https://atom.example.com/blog/img/a.png""#),
        "{}",
        e.content_html
    );
    assert!(
        e.content_html
            .contains("https://atom.example.com/blog/img/a@2x.png 2x"),
        "srcset: {}",
        e.content_html
    );
    assert_eq!(e.image_url, "https://atom.example.com/blog/img/a.png");

    let e = &feed.entries[1];
    assert_eq!(
        e.published_at,
        datetime!(2025-06-01 10:00 UTC).unix_timestamp(),
        "published vor updated"
    );
    assert_eq!(
        e.url, "https://atom.example.com/blog/eintrag-2.html",
        "Enclosure ist nicht der Link"
    );
    assert!(
        e.content_html
            .contains("Erste Zeile<br>Zweite Zeile mit &lt;Klammern&gt;"),
        "{}",
        e.content_html
    );
    assert!(
        e.content_html.contains("<video") && e.content_html.contains("video.mp4"),
        "{}",
        e.content_html
    );
}

#[test]
fn parses_json_feed() {
    let feed = parse::parse(
        fixture("feed.json").as_bytes(),
        "https://json.example.net/feed.json",
        NOW,
    )
    .unwrap();
    assert_eq!((feed.kind, feed.title.as_str()), ("json", "JSON Beispiel"));
    assert_eq!(feed.site_url, "https://json.example.net/");
    assert_eq!(feed.icon_url, "https://json.example.net/fav.png");
    let e = &feed.entries[0];
    assert_eq!((e.guid.as_str(), e.author.as_str()), ("1", "Dora"));
    assert!(e
        .content_html
        .contains(r#"href="https://json.example.net/rel""#));
    assert_eq!(e.image_url, "https://json.example.net/b.jpg");
    let e = &feed.entries[1];
    assert_eq!(e.url, "https://json.example.net/2");
    assert_eq!(
        e.title, "Nur Text. Zweiter Absatz <kein Tag>.",
        "ohne Titel: Anfang des Textes"
    );
    assert!(
        e.content_html
            .starts_with("<p>Nur Text.</p><p>Zweiter Absatz &lt;kein Tag&gt;.</p>"),
        "{}",
        e.content_html
    );
    assert!(e.content_html.contains("<audio") && e.content_html.contains("ep2.m4a"));
}

#[test]
fn html_is_not_a_feed() {
    assert!(parse::parse(fixture("page.html").as_bytes(), "https://example.org/", NOW).is_err());
    assert!(parse::parse(b"", "https://example.org/", NOW).is_err());
    assert!(parse::parse(b"{\"kein\": \"feed\"}", "https://example.org/", NOW).is_err());
}

// ------------------------------------------------------------------ Säubern

#[test]
fn sanitizer() {
    let base = Url::parse("https://blog.example/posts/a").unwrap();
    let out = sanitize::clean_html(
        r#"<h2>Titel</h2><script>x()</script><a href="b">Link</a> <a href="javascript:alert(1)">böse</a>
           <img src="/pixel.gif" width="1" height="1"><img src="bild.png" alt="Bild">
           <table><tr><td colspan="2">Zelle</td></tr></table><pre><code>let a = 1;</code></pre>
           <figure><img src="https://cdn.example/x.webp"><figcaption>Bild</figcaption></figure>
           <video src="v.mp4" controls autoplay></video><object data="x.swf"></object>"#,
        Some(&base),
    );
    assert!(out.contains("<h2>Titel</h2>"));
    assert!(!out.contains("script") && !out.contains("x()"), "{out}");
    assert!(out.contains(r#"<a href="https://blog.example/posts/b" rel="noopener noreferrer" target="_blank">Link</a>"#) || (out.contains(r#"href="https://blog.example/posts/b""#) && out.contains(r#"target="_blank""#)), "{out}");
    assert!(!out.contains("javascript:"), "{out}");
    assert!(!out.contains("pixel.gif"), "Zählpixel weg: {out}");
    assert!(
        out.contains(r#"src="https://blog.example/posts/bild.png""#),
        "{out}"
    );
    assert!(out.contains(r#"<td colspan="2">Zelle</td>"#), "{out}");
    assert!(out.contains("<pre><code>let a = 1;</code></pre>"), "{out}");
    assert!(out.contains("<figcaption>Bild</figcaption>"), "{out}");
    assert!(
        out.contains(r#"src="https://blog.example/posts/v.mp4""#) && !out.contains("autoplay"),
        "{out}"
    );
    assert!(!out.contains("object") && !out.contains("x.swf"), "{out}");
    // Ohne Basis fallen relative Adressen weg
    let out = sanitize::clean_html(r#"<a href="relativ">x</a><img src="/y.png">"#, None);
    assert!(!out.contains("relativ") && !out.contains("y.png"), "{out}");

    assert_eq!(
        sanitize::html_to_text("<p>Ein&nbsp;Absatz</p><ul><li>eins</li><li>zwei</li></ul>"),
        "Ein Absatz eins zwei"
    );
    assert_eq!(sanitize::reading_minutes(&"wort ".repeat(201)), 2);
    let long = "Wort ".repeat(100);
    let e = sanitize::excerpt(&long, 300);
    assert!(
        e.ends_with('…') && e.chars().count() <= 301 && !e.contains("Wo…"),
        "{e}"
    );
}

// ------------------------------------------------------------------ Finden & Abrufen

async fn site() -> TestServer {
    let s = TestServer::start().await;
    s.set("/", "text/html; charset=utf-8", &fixture("page.html"));
    s.set(
        "/feed.xml",
        "application/rss+xml",
        &fixture("rss2.xml").replace("https://example.org", "{base}"),
    );
    s.set("/atom.xml", "application/atom+xml", &fixture("atom.xml"));
    s.set("/feed.json", "application/feed+json", &fixture("feed.json"));
    s.set("/icon-32.png", "image/png", "PNG");
    s
}

#[tokio::test]
async fn discovery_finds_feeds_on_pages_and_by_probing() {
    let e = env().await;
    let s = site().await;

    // Seite mit <link rel="alternate">
    let found = discover::discover(&e.fetcher, &format!("{}/", s.url))
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].url, format!("{}/feed.xml", s.url));
    assert_eq!(
        (found[0].title.as_str(), found[0].kind.as_str()),
        ("Beispiel & Blog", "rss"),
        "geprüft, Titel laut Feed"
    );
    assert_eq!(found[1].title, "Atom Journal");
    assert_eq!(
        (found[1].url.clone(), found[1].kind.as_str()),
        (format!("{}/atom.xml", s.url), "atom")
    );

    // Direkt ein Feed: Titel aus dem Feed
    let found = discover::discover(&e.fetcher, &format!("{}/feed.json", s.url))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].title.as_str(), found[0].kind.as_str()),
        ("JSON Beispiel", "json")
    );

    // Seite ohne Links: übliche Pfade ausprobieren
    let s2 = TestServer::start().await;
    s2.set("/", "text/html", &fixture("plain.html"));
    s2.set(
        "/rss.xml",
        "application/rss+xml",
        &rss("Geprobt", &[("a", "A", "Text", NOW)]),
    );
    let found = discover::discover(&e.fetcher, &s2.url).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].url.clone(), found[0].title.as_str()),
        (format!("{}/rss.xml", s2.url), "Geprobt")
    );

    // Verlinkter, aber leerer Feed kommt nach dem mit Einträgen
    let s3 = TestServer::start().await;
    s3.set(
        "/",
        "text/html",
        r#"<html><head>
            <link rel="alternate" type="application/atom+xml" title="ATOM" href="/leer.xml">
            <link rel="alternate" type="application/rss+xml" title="RSS" href="/voll.xml">
           </head></html>"#,
    );
    s3.set("/leer.xml", "application/rss+xml", &rss("Leer", &[]));
    s3.set(
        "/voll.xml",
        "application/rss+xml",
        &rss("Voll", &[("a", "A", "Text", NOW)]),
    );
    let found = discover::discover(&e.fetcher, &s3.url).await.unwrap();
    let urls: Vec<&str> = found.iter().map(|f| f.title.as_str()).collect();
    assert_eq!(urls, vec!["Voll", "Leer"]);
    // Verlinkt, aber nicht abrufbar: trotzdem angeboten (Titel aus dem Link)
    s3.remove("/leer.xml");
    s3.remove("/voll.xml");
    let found = discover::discover(&e.fetcher, &s3.url).await.unwrap();
    assert_eq!((found.len(), found[0].title.as_str()), (2, "ATOM"));
    // Links im Text, die nach Feed aussehen
    s3.set(
        "/",
        "text/html",
        r#"<html><body><a href="/impressum">Impressum</a><a href="/news/rss">RSS</a></body></html>"#,
    );
    s3.set("/news/rss", "application/rss+xml", &rss("Nachrichten", &[]));
    let found = discover::discover(&e.fetcher, &s3.url).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].url.clone(), found[0].title.as_str()),
        (format!("{}/news/rss", s3.url), "Nachrichten")
    );

    // Nichts zu finden
    s2.remove("/rss.xml");
    let err = discover::discover(&e.fetcher, &s2.url).await.unwrap_err();
    assert!(err.contains("keinen Feed"), "{err}");
    // Nicht erreichbar / ungültig
    assert!(discover::discover(&e.fetcher, "http://127.0.0.1:9/")
        .await
        .is_err());
    assert!(discover::discover(&e.fetcher, "ftp://example.org")
        .await
        .is_err());

    // Icon der Webseite
    let icon = discover::find_icon(&e.fetcher, &format!("{}/", s.url), "", "").await;
    assert_eq!(icon, format!("{}/icon-32.png", s.url));
    // Ohne Icon-Link: /favicon.ico nur, wenn es dort ein Bild gibt
    assert_eq!(
        discover::find_icon(&e.fetcher, &format!("{}/", s2.url), "", "").await,
        ""
    );
    s2.set("/favicon.ico", "image/x-icon", "ICO");
    assert_eq!(
        discover::find_icon(&e.fetcher, &format!("{}/", s2.url), "", "").await,
        format!("{}/favicon.ico", s2.url)
    );
}

#[tokio::test]
async fn conditional_fetch_with_etag() {
    let e = env().await;
    let s = TestServer::start().await;
    s.set_full(
        "/feed.xml",
        200,
        "application/rss+xml",
        &rss("E", &[]),
        Some("\"v1\""),
    );
    let url = format!("{}/feed.xml", s.url);
    let Fetched::Page(page) = e.fetcher.fetch(&url, None, None).await.unwrap() else {
        panic!("erster Abruf liefert Inhalt");
    };
    assert_eq!(page.etag.as_deref(), Some("\"v1\""));
    assert_eq!(page.content_type, "application/rss+xml");
    let head = &s.requests("/feed.xml")[0].head;
    assert!(head.contains("user-agent: nrznews/"), "{head}");
    assert!(matches!(
        e.fetcher.fetch(&url, Some("\"v1\""), None).await.unwrap(),
        Fetched::NotModified
    ));
    assert!(s.requests("/feed.xml")[1]
        .head
        .contains("if-none-match: \"v1\""));
    // Fehler als kurze deutsche Meldung
    let err = e
        .fetcher
        .fetch(&format!("{}/fehlt", s.url), None, None)
        .await
        .unwrap_err();
    assert!(err.contains("404"), "{err}");
}

// ------------------------------------------------------------------ Abonnieren & Aktualisieren

#[tokio::test]
async fn subscribe_refresh_keeps_read_state_and_dedupes() {
    let e = env().await;
    let s = TestServer::start().await;
    s.set("/", "text/html", &fixture("page.html"));
    s.set("/icon-32.png", "image/png", "PNG");
    let t = nrznews::db::now();
    s.set_full(
        "/feed.xml",
        200,
        "application/rss+xml",
        &rss(
            "Mein Feed",
            &[
                ("a", "Alpha", "Erster Text", t - 3600),
                ("b", "Beta", "Zweiter Text", t - 7200),
            ],
        ),
        Some("\"1\""),
    );
    let alice = user(&e.pool, "alice").await;

    // Über die Webseite abonnieren: der erste gefundene Feed, sofort abgerufen
    let feed = feeds::subscribe(&e.pool, &e.fetcher, &e.config, alice, &sub(&s.url))
        .await
        .unwrap();
    assert_eq!(feed.title, "Mein Feed");
    assert_eq!(feed.feed_url, format!("{}/feed.xml", s.url));
    assert_eq!(feed.site_url, format!("{}/", s.url));
    assert_eq!(feed.icon_url, format!("{}/icon-32.png", s.url));
    assert_eq!((feed.unread, feed.total), (2, 2));
    assert!(feed.last_fetched_at.is_some() && feed.last_error.is_none());

    // Doppelt abonnieren: 409
    for url in [s.url.clone(), format!("{}/feed.xml", s.url)] {
        match feeds::subscribe(&e.pool, &e.fetcher, &e.config, alice, &sub(&url)).await {
            Err(AppError::Conflict(msg)) => assert_eq!(msg, "Diesen Feed hast du schon abonniert."),
            other => panic!("Konflikt erwartet: {:?}", other.map(|f| f.id)),
        }
    }

    // Lesen und merken
    let page = articles::list(&e.pool, alice, &articles::ListQuery::default())
        .await
        .unwrap();
    let alpha = page.items.iter().find(|a| a.title == "Alpha").unwrap().id;
    articles::set_state(
        &e.pool,
        alice,
        alpha,
        &ArticleStateInput {
            read: Some(true),
            starred: Some(true),
        },
    )
    .await
    .unwrap();

    // Unverändert (304): nichts Neues, kein Fehler
    let r = feeds::refresh_all(&e.pool, &e.fetcher, &e.config, alice)
        .await
        .unwrap();
    assert_eq!((r.feeds, r.new_articles, r.errors), (1, 0, 0));
    assert!(s
        .requests("/feed.xml")
        .last()
        .unwrap()
        .head
        .contains("if-none-match: \"1\""));

    // Geänderter Titel und ein neuer Artikel: Gelesen/Gemerkt bleiben
    s.set_full(
        "/feed.xml",
        200,
        "application/rss+xml",
        &rss(
            "Mein Feed",
            &[
                ("c", "Gamma", "Neu", t - 60),
                ("a", "Alpha (korrigiert)", "Erster Text, ergänzt", t - 3600),
                ("b", "Beta", "Zweiter Text", t - 7200),
            ],
        ),
        Some("\"2\""),
    );
    let feed = feeds::refresh(&e.pool, &e.fetcher, &e.config, alice, feed.id)
        .await
        .unwrap();
    assert_eq!((feed.unread, feed.total), (2, 3));
    let a = articles::get(&e.pool, alice, alpha).await.unwrap();
    assert_eq!(a.title, "Alpha (korrigiert)");
    assert!(a.read && a.starred, "Zustand bleibt beim Aktualisieren");
    assert!(a.content_html.contains("ergänzt"));
    // Volltextindex folgt dem geänderten Text
    let hits = articles::list(
        &e.pool,
        alice,
        &articles::ListQuery {
            q: Some("ergänzt".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(hits.items.len(), 1);

    // Fehler: Meldung, Fehlerzähler, Wartezeit wächst
    s.set_full("/feed.xml", 500, "text/plain", "kaputt", None);
    let before = nrznews::db::now();
    let r = feeds::refresh_all(&e.pool, &e.fetcher, &e.config, alice)
        .await
        .unwrap();
    assert_eq!(r.errors, 1);
    let (err, count, next): (Option<String>, i64, i64) =
        sqlx::query_as("SELECT last_error, error_count, next_fetch_at FROM feeds WHERE id = ?")
            .bind(feed.id)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(err.unwrap().contains("500"));
    assert_eq!(count, 1);
    assert!(next >= before + 2 * 30 * 60, "Intervall × 2");
    feeds::refresh(&e.pool, &e.fetcher, &e.config, alice, feed.id)
        .await
        .unwrap();
    let (count, next): (i64, i64) =
        sqlx::query_as("SELECT error_count, next_fetch_at FROM feeds WHERE id = ?")
            .bind(feed.id)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    assert!(next >= before + 4 * 30 * 60, "Intervall × 4");
    let listed = feeds::get(&e.pool, alice, feed.id).await.unwrap();
    assert!(listed.last_error.is_some());

    // Wieder ok: Fehler weg
    s.set(
        "/feed.xml",
        "application/rss+xml",
        &rss("Mein Feed", &[("c", "Gamma", "Neu", t - 60)]),
    );
    let feed = feeds::refresh(&e.pool, &e.fetcher, &e.config, alice, feed.id)
        .await
        .unwrap();
    assert!(feed.last_error.is_none());
    assert_eq!(feed.total, 3, "verschwundene Einträge bleiben");
}

#[tokio::test]
async fn first_fetch_marks_old_articles_read_and_background_refresh() {
    let e = env().await;
    let s = TestServer::start().await;
    let t = nrznews::db::now();
    s.set(
        "/feed.xml",
        "application/rss+xml",
        &rss(
            "Alt und neu",
            &[
                ("neu", "Neu", "x", t - 86400),
                ("alt", "Alt", "y", t - 40 * 86400),
            ],
        ),
    );
    let alice = user(&e.pool, "alice").await;
    let feed = feeds::subscribe(
        &e.pool,
        &e.fetcher,
        &e.config,
        alice,
        &sub(&format!("{}/feed.xml", s.url)),
    )
    .await
    .unwrap();
    assert_eq!(
        (feed.unread, feed.total),
        (1, 2),
        "älter als 30 Tage gilt als gelesen"
    );

    // Später auftauchende alte Artikel bleiben ungelesen (nur der erste Abruf zählt)
    s.set(
        "/feed.xml",
        "application/rss+xml",
        &rss(
            "Alt und neu",
            &[("nachzuegler", "Nachzügler", "z", t - 40 * 86400)],
        ),
    );
    sqlx::query("UPDATE feeds SET next_fetch_at = 0")
        .execute(&e.pool)
        .await
        .unwrap();
    let r = refresher::refresh_due(&e.pool, &e.fetcher, &e.config)
        .await
        .unwrap();
    assert_eq!((r.feeds, r.new_articles), (1, 1));
    let feed = feeds::get(&e.pool, alice, feed.id).await.unwrap();
    assert_eq!((feed.unread, feed.total), (2, 3));
    // Nicht fällig: nichts zu tun
    let r = refresher::refresh_due(&e.pool, &e.fetcher, &e.config)
        .await
        .unwrap();
    assert_eq!(r.feeds, 0);
}

#[tokio::test]
async fn cleanup_keeps_newest_starred_and_unread() {
    let e = env().await;
    let alice = user(&e.pool, "alice").await;
    let feed: i64 = sqlx::query_scalar("INSERT INTO feeds (user_id, feed_url, created_at) VALUES (?, 'https://x.example/f', 0) RETURNING id")
        .bind(alice)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let old = nrznews::db::now() - 100 * 86400;
    // 30 alte gelesene Artikel, einer davon gemerkt, einer ungelesen
    for i in 0..30i64 {
        sqlx::query(
            "INSERT INTO articles (feed_id, user_id, guid, title, published_at, fetched_at, read, starred)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(feed)
        .bind(alice)
        .bind(format!("g{i}"))
        .bind(format!("T{i}"))
        .bind(old - i * 3600)
        .bind(old)
        .bind(i != 25)
        .bind(i == 29)
        .execute(&e.pool)
        .await
        .unwrap();
    }
    assert_eq!(
        refresher::cleanup_older_than(&e.pool, 0).await.unwrap(),
        0,
        "0 = nie"
    );
    let deleted = refresher::cleanup_older_than(&e.pool, 60).await.unwrap();
    assert_eq!(deleted, 8, "30 − 20 neueste − gemerkt − ungelesen");
    let left: Vec<String> =
        sqlx::query_scalar("SELECT guid FROM articles ORDER BY published_at DESC")
            .fetch_all(&e.pool)
            .await
            .unwrap();
    assert_eq!(left.len(), 22);
    assert!(left.contains(&"g25".to_string()) && left.contains(&"g29".to_string()));
    assert_eq!(refresher::cleanup(&e.pool, &e.config).await.unwrap(), 0);
}

// ------------------------------------------------------------------ Ordner, Benutzer, Löschen

#[tokio::test]
async fn users_are_isolated_and_delete_cascades() {
    let e = env().await;
    let s = site().await;
    let alice = user(&e.pool, "alice").await;
    let bob = user(&e.pool, "bob").await;
    let folder = folders::create(&e.pool, alice, "Technik").await.unwrap();
    assert!(matches!(
        folders::create(&e.pool, alice, " Technik ").await,
        Err(AppError::Conflict(_))
    ));
    let input = SubscribeInput {
        url: format!("{}/feed.xml", s.url),
        folder_id: Some(folder.id),
        title: "Eigener Name".into(),
    };
    let feed = feeds::subscribe(&e.pool, &e.fetcher, &e.config, alice, &input)
        .await
        .unwrap();
    assert_eq!(feed.title, "Eigener Name", "eigener Name bleibt");
    assert_eq!(feed.folder_id, Some(folder.id));
    assert_eq!(
        folders::get(&e.pool, alice, folder.id)
            .await
            .unwrap()
            .unread,
        feed.unread
    );

    // Bob sieht nichts von Alice und kann nichts ändern
    assert!(feeds::list(&e.pool, bob).await.unwrap().is_empty());
    assert!(folders::list(&e.pool, bob).await.unwrap().is_empty());
    assert!(matches!(
        feeds::get(&e.pool, bob, feed.id).await,
        Err(AppError::NotFound)
    ));
    let upd = UpdateFeedInput {
        title: "gekapert".into(),
        folder_id: None,
    };
    assert!(matches!(
        feeds::update(&e.pool, bob, feed.id, &upd).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        feeds::delete(&e.pool, bob, feed.id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        feeds::refresh(&e.pool, &e.fetcher, &e.config, bob, feed.id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        folders::rename(&e.pool, bob, folder.id, "x").await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        folders::delete(&e.pool, bob, folder.id).await,
        Err(AppError::NotFound)
    ));
    // … und kann seine Feeds nicht in Alices Ordner legen
    let bobs = SubscribeInput {
        folder_id: Some(folder.id),
        ..sub(&format!("{}/feed.json", s.url))
    };
    assert!(matches!(
        feeds::subscribe(&e.pool, &e.fetcher, &e.config, bob, &bobs).await,
        Err(AppError::BadRequest(_))
    ));
    // Derselbe Feed bei Bob: eigenes Abo, eigene Artikel
    let bob_feed = feeds::subscribe(
        &e.pool,
        &e.fetcher,
        &e.config,
        bob,
        &sub(&format!("{}/feed.xml", s.url)),
    )
    .await
    .unwrap();
    assert_ne!(bob_feed.id, feed.id);
    assert_eq!(bob_feed.title, "Beispiel & Blog");
    let ov = feeds::overview(&e.pool, bob).await.unwrap();
    assert_eq!((ov.feeds.len(), ov.folders.len()), (1, 0));
    assert_eq!(ov.unread, bob_feed.unread);

    // Umbenennen: leer = wieder der Titel des Feeds
    let renamed = feeds::update(
        &e.pool,
        alice,
        feed.id,
        &UpdateFeedInput {
            title: "  ".into(),
            folder_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (renamed.title.as_str(), renamed.folder_id),
        ("Beispiel & Blog", None)
    );

    // Ordner löschen: Feed bleibt (ohne Ordner)
    feeds::update(
        &e.pool,
        alice,
        feed.id,
        &UpdateFeedInput {
            title: String::new(),
            folder_id: Some(folder.id),
        },
    )
    .await
    .unwrap();
    folders::delete(&e.pool, alice, folder.id).await.unwrap();
    assert_eq!(
        feeds::get(&e.pool, alice, feed.id).await.unwrap().folder_id,
        None
    );

    // Feed löschen: Artikel und Suchindex gehen mit, Bobs bleiben
    feeds::delete(&e.pool, alice, feed.id).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM articles WHERE user_id = ?")
        .bind(alice)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let fts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM articles_fts WHERE articles_fts MATCH 'hallo'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(fts, 1, "nur noch Bobs Artikel im Index");
    assert!(bob_feed.total > 0);
    assert_eq!(
        feeds::get(&e.pool, bob, bob_feed.id).await.unwrap().total,
        bob_feed.total
    );
}

// ------------------------------------------------------------------ OPML

#[tokio::test]
async fn opml_import_export_round_trip() {
    let e = env().await;
    let s = site().await;
    let alice = user(&e.pool, "alice").await;
    folders::create(&e.pool, alice, "technik").await.unwrap();
    let opml = fixture("subscriptions.opml").replace("{base}", &s.url);

    let r = feeds::import_opml(&e.pool, alice, &opml).await.unwrap();
    assert_eq!((r.added, r.skipped, r.folders), (3, 0, 0), "{r:?}");
    assert_eq!(r.errors.len(), 1, "ftp:// geht nicht: {:?}", r.errors);
    assert!(r.errors[0].starts_with("Kaputt:"), "{:?}", r.errors);
    // Nicht sofort abgerufen, aber fällig
    let list = feeds::list(&e.pool, alice).await.unwrap();
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|f| f.last_fetched_at.is_none()));
    let technik = folders::list(&e.pool, alice).await.unwrap();
    assert_eq!(
        technik.len(),
        1,
        "vorhandener Ordner (ohne Gross/Klein) wird benutzt"
    );
    let in_folder: Vec<&str> = list
        .iter()
        .filter(|f| f.folder_id == Some(technik[0].id))
        .map(|f| f.title.as_str())
        .collect();
    assert_eq!(
        in_folder.len(),
        2,
        "Unterordner landen im obersten Ordner: {in_folder:?}"
    );

    // Zweimal importieren: alles übersprungen
    let r = feeds::import_opml(&e.pool, alice, &opml).await.unwrap();
    assert_eq!((r.added, r.skipped), (0, 3));
    assert!(matches!(
        feeds::import_opml(&e.pool, alice, "kein xml").await,
        Err(AppError::BadRequest(_))
    ));

    // Abruf im Hintergrund holt die Feeds (OPML-Titel bleiben)
    refresher::refresh_due(&e.pool, &e.fetcher, &e.config)
        .await
        .unwrap();
    let list = feeds::list(&e.pool, alice).await.unwrap();
    assert!(
        list.iter()
            .all(|f| f.last_fetched_at.is_some() && f.total > 0),
        "{list:?}"
    );
    assert!(list.iter().any(|f| f.title == "Rust Blog"));

    // Export → Import bei einem anderen Benutzer ergibt dieselben Abos
    let xml = feeds::export_opml(&e.pool, alice, "alice").await.unwrap();
    assert!(
        xml.starts_with("<?xml") && xml.contains("<opml version=\"2.0\">"),
        "{xml}"
    );
    let bob = user(&e.pool, "bob").await;
    let r = feeds::import_opml(&e.pool, bob, &xml).await.unwrap();
    assert_eq!((r.added, r.folders, r.errors.len()), (3, 1, 0));
    let a: Vec<(String, Option<String>)> = names(&e.pool, alice).await;
    let b: Vec<(String, Option<String>)> = names(&e.pool, bob).await;
    assert_eq!(a, b);
}

async fn names(pool: &SqlitePool, user: i64) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT f.feed_url, lower(d.name) FROM feeds f LEFT JOIN folders d ON d.id = f.folder_id
         WHERE f.user_id = ? ORDER BY f.feed_url",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

// ------------------------------------------------------------------ HTTP-API

#[rocket::async_test]
async fn http_feeds_folders_overview_and_opml() {
    let (c, _dir) = common::client().await;
    let s = site().await;
    // Ohne Anmeldung: 401
    assert_eq!(
        common::get(&c, "/api/v1/feeds").await.0,
        Status::Unauthorized
    );
    common::login_as(&c, "alice", false).await;

    let (st, folder) = common::post(&c, "/api/v1/folders", json!({ "name": "Technik" })).await;
    assert_eq!(st, Status::Created, "{folder}");
    let (st, found) = common::post(&c, "/api/v1/feeds/discover", json!({ "url": s.url })).await;
    assert_eq!(st, Status::Ok, "{found}");
    assert_eq!(found[0]["kind"], "rss");
    let (st, feed) = common::post(
        &c,
        "/api/v1/feeds",
        json!({ "url": format!("{}/feed.xml", s.url), "folderId": folder["id"] }),
    )
    .await;
    assert_eq!(st, Status::Created, "{feed}");
    assert_eq!(feed["title"], "Beispiel & Blog");
    assert_eq!(feed["folderId"], folder["id"]);
    let (st, again) = common::post(
        &c,
        "/api/v1/feeds",
        json!({ "url": format!("{}/feed.xml", s.url) }),
    )
    .await;
    assert_eq!(st, Status::Conflict);
    assert_eq!(
        again["error"]["message"],
        "Diesen Feed hast du schon abonniert."
    );
    let (st, bad) =
        common::post(&c, "/api/v1/feeds", json!({ "url": "http://127.0.0.1:9/" })).await;
    assert_eq!(st, Status::BadRequest, "{bad}");

    let (_, ov) = common::get(&c, "/api/v1/overview").await;
    assert_eq!(ov["feeds"].as_array().unwrap().len(), 1);
    assert_eq!(ov["folders"][0]["unread"], feed["unread"]);
    assert_eq!(ov["unread"], feed["unread"]);

    let id = feed["id"].as_i64().unwrap();
    let (st, upd) = common::put(
        &c,
        format!("/api/v1/feeds/{id}"),
        json!({ "title": "Neu", "folderId": null }),
    )
    .await;
    assert_eq!(
        (st, upd["title"].clone(), upd["folderId"].clone()),
        (Status::Ok, json!("Neu"), json!(null))
    );
    let (st, r) = common::post(&c, "/api/v1/feeds/refresh", json!({})).await;
    assert_eq!(
        (st, r["feeds"].clone(), r["errors"].clone()),
        (Status::Ok, json!(1), json!(0)),
        "{r}"
    );
    let (st, _) = common::call(&c, "POST", format!("/api/v1/feeds/{id}/refresh"), None).await;
    assert_eq!(st, Status::Ok);

    // OPML herunterladen und (bei Bob) hochladen
    let res = c.get("/api/v1/opml").dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(
        res.headers().get_one("Content-Disposition"),
        Some("attachment; filename=\"nrznews.opml\"")
    );
    assert!(res
        .content_type()
        .unwrap()
        .to_string()
        .starts_with("text/x-opml"));
    let xml = res.into_string().await.unwrap();
    assert!(xml.contains(&format!("{}/feed.xml", s.url)));
    common::logout(&c).await;
    common::login_as(&c, "bob", false).await;
    assert_eq!(
        common::call(&c, "POST", format!("/api/v1/feeds/{id}/refresh"), None)
            .await
            .0,
        Status::NotFound
    );
    assert_eq!(
        common::delete(&c, format!("/api/v1/feeds/{id}")).await.0,
        Status::NotFound
    );
    let res = c
        .post("/api/v1/opml")
        .header(ContentType::XML)
        .body(xml)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let r: serde_json::Value = res.into_json().await.unwrap();
    assert_eq!(
        (r["added"].clone(), r["folders"].clone()),
        (json!(1), json!(0)),
        "{r}"
    );
    let (_, list) = common::get(&c, "/api/v1/feeds").await;
    assert_eq!(list[0]["title"], "Neu");
    let bob_feed = list[0]["id"].as_i64().unwrap();
    assert_eq!(
        common::delete(&c, format!("/api/v1/feeds/{bob_feed}"))
            .await
            .0,
        Status::NoContent
    );
    let (st, _) = common::delete(&c, format!("/api/v1/folders/{}", folder["id"])).await;
    assert_eq!(st, Status::NotFound, "Alices Ordner");
}

/// Mit Hintergrundarbeiten: nach dem OPML-Import holt der Refresher die Feeds sofort
/// (geweckt), ohne auf die nächste Minute zu warten.
#[rocket::async_test]
async fn background_refresher_fetches_after_import() {
    let (c, _dir) = common::client_with(|f| f.merge(("background", true))).await;
    let s = site().await;
    common::login_as(&c, "alice", false).await;
    let opml = fixture("subscriptions.opml").replace("{base}", &s.url);
    let res = c
        .post("/api/v1/opml")
        .header(ContentType::XML)
        .body(opml)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    drop(res);
    let mut fetched = 0;
    for _ in 0..100 {
        let (_, list) = common::get(&c, "/api/v1/feeds").await;
        fetched = list
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| !f["lastFetchedAt"].is_null())
            .count();
        if fetched == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(fetched, 3, "im Hintergrund abgerufen");
    c.terminate().await;
}

// ------------------------------------------------------------------ Netz (nur auf Wunsch)

/// `cargo test --test feeds -- --ignored`: echter Feed aus dem Netz.
#[tokio::test]
#[ignore = "braucht Internet"]
async fn network_rust_blog() {
    let e = env().await;
    let alice = user(&e.pool, "alice").await;
    let feed = feeds::subscribe(
        &e.pool,
        &e.fetcher,
        &e.config,
        alice,
        &sub("blog.rust-lang.org/feed.xml"),
    )
    .await
    .unwrap();
    assert!(feed.title.contains("Rust"), "{}", feed.title);
    assert!(feed.total > 5);
    assert!(!feed.icon_url.is_empty());
    let page = articles::list(&e.pool, alice, &articles::ListQuery::default())
        .await
        .unwrap();
    assert!(page
        .items
        .iter()
        .all(|a| a.url.starts_with("https://blog.rust-lang.org/")));
}

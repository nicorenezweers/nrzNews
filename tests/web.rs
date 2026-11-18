//! Rund um die API: Health-Check, Oberfläche (eingebettete Dioxus-App bzw. Hinweisseite)
//! mit Rückfall auf index.html, Fehler als JSON oder HTML, Sicherheits-Header, eingebettete
//! Schrift, Icons und Übersetzungen.

mod common;

use common::get;
use rocket::http::{Accept, ContentType, Header, Status};
use serde_json::Value;

#[rocket::async_test]
async fn health() {
    let (client, _dir) = common::client().await;
    let (s, v) = get(&client, "/api/v1/health").await;
    assert_eq!(s, Status::Ok);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["name"], "nrzNews");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    client.terminate().await;
}

#[rocket::async_test]
async fn unknown_api_paths_answer_with_json() {
    let (client, _dir) = common::client().await;
    for (method, url) in [
        ("GET", "/api/v1/unknown"),
        ("GET", "/api/v1"),
        ("GET", "/api/unknown/deeper"),
        ("POST", "/api/v1/unknown"),
        ("DELETE", "/api/v1/nichts/1"),
    ] {
        let req = match method {
            "GET" => client.get(url),
            "POST" => client.post(url),
            _ => client.delete(url),
        };
        // Auch ohne Accept: application/json – der Pfad entscheidet
        let res = req.dispatch().await;
        assert_eq!(res.status(), Status::NotFound, "{method} {url}");
        assert_eq!(
            res.content_type(),
            Some(ContentType::JSON),
            "{method} {url}"
        );
        let v: Value = res.into_json().await.unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert!(v["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()));
    }
    client.terminate().await;
}

#[rocket::async_test]
async fn app_pages_fall_back_to_the_spa() {
    let (client, _dir) = common::client().await;
    let embedded = nrznews::routes::spa::available();
    for url in [
        "/",
        "/login",
        "/feeds/12",
        "/settings/users",
        "/articles/5?view=unread",
    ] {
        let res = client.get(url).dispatch().await;
        let expected = if embedded {
            Status::Ok
        } else {
            Status::ServiceUnavailable
        };
        assert_eq!(res.status(), expected, "{url}");
        assert_eq!(res.content_type(), Some(ContentType::HTML), "{url}");
        assert_eq!(
            res.headers().get_one("Cache-Control"),
            Some("no-cache"),
            "{url}"
        );
        let body = res.into_string().await.unwrap();
        assert!(
            body.contains("<html") || body.contains("<!DOCTYPE") || body.contains("<!doctype"),
            "{url}"
        );
        if !embedded {
            assert!(body.contains("cargo xtask ui"));
        }
    }
    // Fehlende Dateien der App sind 404, nicht index.html
    for url in [
        "/assets/alt-abc123.js",
        "/assets/x.wasm",
        "/main.css",
        "/static/gibtsnicht.png",
    ] {
        let res = client.get(url).dispatch().await;
        assert_eq!(res.status(), Status::NotFound, "{url}");
        // Kein API-Pfad und kein JSON verlangt → HTML-Fehlerseite
        assert_eq!(res.content_type(), Some(ContentType::HTML), "{url}");
    }
    // Mit Accept: application/json gibt es auch ausserhalb der API JSON
    let res = client
        .get("/static/gibtsnicht.png")
        .header(Accept::JSON)
        .dispatch()
        .await;
    assert_eq!(res.content_type(), Some(ContentType::JSON));
    drop(res);
    client.terminate().await;
}

#[rocket::async_test]
async fn security_headers_on_every_response() {
    let (client, _dir) = common::client().await;
    for url in [
        "/",
        "/api/v1/health",
        "/api/v1/unknown",
        "/static/fonts/inter-latin.woff2",
    ] {
        let res = client.get(url).dispatch().await;
        let h = res.headers();
        let csp = h.get_one("Content-Security-Policy").unwrap_or_default();
        for part in [
            "default-src 'self'",
            "script-src 'self' 'wasm-unsafe-eval'",
            "img-src 'self' data: blob: https: http:",
            "media-src 'self' https: http:",
            "frame-ancestors 'none'",
            "object-src 'none'",
        ] {
            assert!(csp.contains(part), "{url}: {part} fehlt in {csp}");
        }
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "keine Inline-Skripte"
        );
        assert_eq!(
            h.get_one("X-Content-Type-Options"),
            Some("nosniff"),
            "{url}"
        );
        assert_eq!(h.get_one("Referrer-Policy"), Some("no-referrer"), "{url}");
        assert_eq!(h.get_one("X-Frame-Options"), Some("DENY"), "{url}");
        assert!(
            h.get_one("Permissions-Policy")
                .is_some_and(|p| p.contains("camera=()")),
            "{url}"
        );
    }
    client.terminate().await;
}

#[rocket::async_test]
async fn fonts_icons_and_robots_are_embedded() {
    let (client, _dir) = common::client().await;
    let res = client
        .get("/static/fonts/inter-latin.woff2")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.content_type(), Some(ContentType::WOFF2));
    assert_eq!(
        res.headers().get_one("Cache-Control"),
        Some("public, max-age=31536000, immutable")
    );
    let body = res.into_bytes().await.unwrap();
    assert_eq!(&body[..4], b"wOF2");

    // Icons kommen von Paket 4 – geprüft wird, was beim Übersetzen da war
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (url, file) in [
        ("/static/icon.png", "static/icon.png"),
        ("/static/favicon-32.png", "static/favicon-32.png"),
        ("/favicon.ico", "static/favicon-32.png"),
    ] {
        let res = client.get(url).dispatch().await;
        if manifest.join(file).is_file() {
            assert_eq!(res.status(), Status::Ok, "{url}");
            assert_eq!(res.content_type(), Some(ContentType::PNG), "{url}");
            let body = res.into_bytes().await.unwrap();
            assert_eq!(&body[1..4], b"PNG", "{url}");
        } else {
            assert_eq!(res.status(), Status::NotFound, "{url}");
        }
    }
    // Text (SVG) kommt gzip-komprimiert, wenn der Browser das versteht
    if manifest.join("static/icon.svg").is_file() {
        let res = client
            .get("/static/icon.svg")
            .header(Header::new("Accept-Encoding", "gzip"))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::Ok);
        assert_eq!(res.content_type(), Some(ContentType::SVG));
        assert_eq!(res.headers().get_one("Content-Encoding"), Some("gzip"));
        let gz = res.into_bytes().await.unwrap();
        assert_eq!(&gz[..2], &[0x1f, 0x8b]);
        let plain = client.get("/static/icon.svg").dispatch().await;
        assert!(plain.headers().get_one("Content-Encoding").is_none());
        assert!(plain.into_string().await.unwrap().contains("<svg"));
    }

    // Web-App-Manifest (zum Startbildschirm hinzufügen)
    let res = client.get("/static/manifest.webmanifest").dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(
        res.content_type(),
        Some(ContentType::new("application", "manifest+json"))
    );
    let body: serde_json::Value = serde_json::from_str(&res.into_string().await.unwrap()).unwrap();
    assert_eq!(body["name"], "nrzNews");
    assert_eq!(body["display"], "standalone");

    // Pfade aus static/, die nicht eingebettet werden
    assert_eq!(
        client
            .get("/static/fonts/LICENSE")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );
    assert_eq!(
        client
            .get("/static/../Cargo.toml")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );

    let res = client.get("/robots.txt").dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert!(res.into_string().await.unwrap().contains("Disallow: /"));
    client.terminate().await;
}

#[rocket::async_test]
async fn locales_are_served_from_the_binary() {
    let (client, _dir) = common::client().await;
    let codes = nrznews::routes::assets::locale_codes();
    for code in &codes {
        let url = format!("/static/locales/{code}.json?v=1.0.0");
        let res = client.get(url.clone()).dispatch().await;
        assert_eq!(res.status(), Status::Ok, "{url}");
        assert_eq!(res.content_type(), Some(ContentType::JSON), "{url}");
        let catalog: Value = res.into_json().await.unwrap();
        assert!(catalog.is_object(), "{url}");

        // gzip, wenn der Browser es versteht
        let res = client
            .get(url.clone())
            .header(Header::new("Accept-Encoding", "gzip, deflate, br"))
            .dispatch()
            .await;
        assert_eq!(
            res.headers().get_one("Content-Encoding"),
            Some("gzip"),
            "{url}"
        );
        assert_eq!(
            res.headers().get_one("Vary"),
            Some("Accept-Encoding"),
            "{url}"
        );
    }
    // Unbekannte Sprache und die Textliste selbst gibt es nicht
    assert_eq!(
        client
            .get("/static/locales/xx.json")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );
    assert_eq!(
        client
            .get("/static/locales/strings.json")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );
    client.terminate().await;
}

#[rocket::async_test]
async fn embedded_app_files_are_served_with_caching_and_gzip() {
    if !nrznews::routes::spa::available() {
        // Ohne `cargo xtask ui` vor dem Übersetzen gibt es nichts zu prüfen
        return;
    }
    let (client, _dir) = common::client().await;
    let res = client
        .get("/")
        .header(Header::new("Accept-Encoding", "gzip"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Content-Encoding"), Some("gzip"));
    assert_eq!(res.headers().get_one("Cache-Control"), Some("no-cache"));
    drop(res);

    if let Some(asset) = nrznews::routes::spa::FILES
        .iter()
        .find(|f| f.path.starts_with("assets/"))
    {
        let res = client.get(format!("/{}", asset.path)).dispatch().await;
        assert_eq!(res.status(), Status::Ok);
        assert_eq!(
            res.headers().get_one("Cache-Control"),
            Some("public, max-age=31536000, immutable")
        );
        if asset.path.ends_with(".wasm") {
            assert_eq!(
                res.content_type(),
                Some(ContentType::new("application", "wasm"))
            );
        }
    }
    client.terminate().await;
}

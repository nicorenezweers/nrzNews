//! Winziger HTTP-Server für die Tests (tokio, ohne weitere Abhängigkeiten): liefert
//! festgelegte Antworten je Pfad, beantwortet `If-None-Match` mit `304` und merkt sich jede
//! Anfrage. Eingebunden mit `#[path = "fixtures/server.rs"] mod server;`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct Reply {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    etag: Option<String>,
}

/// Eine aufgezeichnete Anfrage: Pfad und Kopfzeilen (klein geschrieben).
#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub head: String,
}

#[derive(Clone)]
pub struct TestServer {
    /// `http://127.0.0.1:<port>` (ohne `/` am Ende)
    pub url: String,
    replies: Arc<Mutex<HashMap<String, Reply>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl TestServer {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let replies: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let (r, s) = (replies.clone(), seen.clone());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (r, s) = (r.clone(), s.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let n = match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    let path = head
                        .lines()
                        .next()
                        .and_then(|l| l.split(' ').nth(1))
                        .unwrap_or("/")
                        .to_string();
                    let lower = head.to_lowercase();
                    s.lock().unwrap().push(Seen {
                        path: path.clone(),
                        head: lower.clone(),
                    });
                    let reply = r.lock().unwrap().get(&path).cloned();
                    let out: Vec<u8> = match reply {
                        None => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                        Some(rep) => {
                            let not_modified = rep.etag.as_ref().is_some_and(|e| {
                                lower
                                    .lines()
                                    .any(|l| l.strip_prefix("if-none-match:").is_some_and(|v| v.trim() == e.to_lowercase()))
                            });
                            if not_modified {
                                b"HTTP/1.1 304 Not Modified\r\nconnection: close\r\n\r\n".to_vec()
                            } else {
                                let mut head = format!(
                                    "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n",
                                    rep.status,
                                    rep.content_type,
                                    rep.body.len()
                                );
                                if let Some(e) = &rep.etag {
                                    head.push_str(&format!("etag: {e}\r\n"));
                                }
                                head.push_str("\r\n");
                                let mut out = head.into_bytes();
                                out.extend_from_slice(&rep.body);
                                out
                            }
                        }
                    };
                    let _ = sock.write_all(&out).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        TestServer { url, replies, seen }
    }

    /// Antwort für einen Pfad festlegen; `{base}` im Körper wird zur Adresse des Servers.
    pub fn set(&self, path: &str, content_type: &str, body: &str) {
        self.set_full(path, 200, content_type, body, None);
    }

    pub fn set_full(
        &self,
        path: &str,
        status: u16,
        content_type: &str,
        body: &str,
        etag: Option<&str>,
    ) {
        self.replies.lock().unwrap().insert(
            path.to_string(),
            Reply {
                status,
                content_type: content_type.to_string(),
                body: body.replace("{base}", &self.url).into_bytes(),
                etag: etag.map(String::from),
            },
        );
    }

    pub fn remove(&self, path: &str) {
        self.replies.lock().unwrap().remove(path);
    }

    /// Anfragen an einen Pfad.
    pub fn requests(&self, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }

    pub fn hits(&self, path: &str) -> usize {
        self.requests(path).len()
    }
}

/// Inhalt einer Datei aus `tests/fixtures`.
pub fn fixture(name: &str) -> String {
    match name {
        "page.html" => return PAGE_HTML.to_string(),
        "plain.html" => return PLAIN_HTML.to_string(),
        _ => {}
    }
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Ein kleiner RSS-Feed mit den gegebenen Einträgen `(guid, titel, text, pubDate-Unix)`.
pub fn rss(title: &str, items: &[(&str, &str, &str, i64)]) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>{title}</title>\
         <link>{{base}}/</link><description>Test</description>"
    );
    for (guid, t, text, ts) in items {
        let date = time::OffsetDateTime::from_unix_timestamp(*ts)
            .unwrap()
            .format(&time::format_description::well_known::Rfc2822)
            .unwrap();
        out.push_str(&format!(
            "<item><guid>{guid}</guid><title>{t}</title><link>{{base}}/p/{guid}</link>\
             <pubDate>{date}</pubDate><description><![CDATA[<p>{text}</p>]]></description></item>"
        ));
    }
    out.push_str("</channel></rss>");
    out
}

/// HTML-Seite mit Feed-Links im `<head>` (für das Finden von Feeds).
const PAGE_HTML: &str = r#"<!doctype html>
<html lang="de">
<head>
  <meta charset="utf-8">
  <title>Testseite</title>
  <link rel="stylesheet" href="/style.css">
  <link rel="alternate" type="application/rss+xml" title="Testseite RSS" href="/feed.xml">
  <link rel="alternate" type="application/atom+xml" href="atom.xml">
  <link rel="icon" href="/icon-32.png" sizes="32x32">
</head>
<body><h1>Willkommen</h1><p>Kein Feed hier, nur Links im Kopf.</p></body>
</html>
"#;

/// HTML-Seite ohne Feed-Links.
const PLAIN_HTML: &str = r#"<!doctype html>
<html><head><title>Ohne Links</title></head><body><p>Hier steht kein Feed-Link.</p></body></html>
"#;

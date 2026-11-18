//! Feeds und Icon einer Webseite finden. Wer „example.com“ eintippt, will nicht wissen, ob
//! der Feed unter `/feed`, `/rss.xml` oder `/index.xml` liegt:
//! 1. Ist die Adresse selbst ein Feed, ist es dieser.
//! 2. Sonst die `<link rel="alternate">` der Seite (RSS, Atom, JSON Feed) – abgerufen und
//!    geprüft, Feeds mit Einträgen zuerst.
//! 3. Sonst Links im Text, die nach einem Feed aussehen (`<a href="/news/rss">`).
//! 4. Sonst die üblichen Pfade ausprobieren.
//!
//! Geprüft wird jeweils gleichzeitig, damit es auch bei langsamen Servern schnell geht.

use super::fetch::Fetcher;
use super::parse;
use super::sanitize::resolve;
use nrznews_types::DiscoveredFeed;
use std::sync::LazyLock;
use url::Url;

/// Übliche Orte für Feeds, wenn die Seite keinen verlinkt.
const PROBES: &[&str] = &[
    "feed",
    "rss",
    "rss.xml",
    "atom.xml",
    "feed.xml",
    "index.xml",
    "feed.json",
    // Reddit & Co.: `/r/rust/.rss`
    ".rss",
];

/// Adresse aus der Eingabe: ohne Schema gilt `https://`, `feed://` wird zu `https://`.
pub fn normalize_url(input: &str) -> Result<Url, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Bitte eine Adresse angeben.".into());
    }
    if s.len() > 2000 {
        return Err("Die Adresse ist zu lang.".into());
    }
    let s = s
        .strip_prefix("feed://")
        .map(|r| format!("https://{r}"))
        .or_else(|| s.strip_prefix("feed:").map(String::from))
        .unwrap_or_else(|| s.to_string());
    let s = if s.contains("://") {
        s
    } else {
        format!("https://{s}")
    };
    let url = Url::parse(&s).map_err(|_| "Das ist keine gültige Adresse.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Nur Adressen mit http:// oder https:// gehen.".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("Das ist keine gültige Adresse.".into());
    }
    Ok(url)
}

/// Feeds zu einer Adresse (siehe Modulbeschreibung). Der erste ist der wahrscheinlichste.
pub async fn discover(fetcher: &Fetcher, input: &str) -> Result<Vec<DiscoveredFeed>, String> {
    let url = normalize_url(input)?;
    let page = fetcher.get(url.as_str()).await?;
    let now = crate::db::now();

    // 1. Selbst ein Feed?
    if let Ok(feed) = parse::parse(&page.body, url.as_str(), now) {
        return Ok(vec![DiscoveredFeed {
            url: url.to_string(),
            title: feed.title,
            kind: feed.kind.into(),
        }]);
    }

    // 2. Verlinkte Feeds der Seite – geprüft, damit z. B. ein Atom-Feed, den der Parser
    //    nicht versteht (leer), nicht vor dem RSS-Feed derselben Seite landet
    let base = Url::parse(&page.url).unwrap_or_else(|_| url.clone());
    let html = String::from_utf8_lossy(&page.body);
    let linked = feed_links(&html, &base);
    let found = verify(fetcher, linked.iter().map(|f| f.url.clone()).take(6), now).await;
    if !found.is_empty() {
        return Ok(found);
    }
    if !linked.is_empty() {
        // Verlinkt, aber gerade nicht abrufbar: trotzdem anbieten
        return Ok(linked);
    }

    // 3. Links im Text, die nach einem Feed aussehen (`<a href="/rss">`)
    let found = verify(fetcher, anchor_links(&html, &base).into_iter().take(8), now).await;
    if !found.is_empty() {
        return Ok(found);
    }

    // 4. Übliche Pfade – an der Seite selbst und an der Wurzel der Domain
    let mut candidates: Vec<String> = Vec::new();
    let mut dir = base.clone();
    if !dir.path().ends_with('/') {
        let path = format!("{}/", dir.path());
        dir.set_path(&path);
    }
    for b in [dir, base.join("/").unwrap_or_else(|_| base.clone())] {
        for p in PROBES {
            if let Ok(c) = b.join(p) {
                let c = c.to_string();
                if c != url.as_str() && !candidates.contains(&c) {
                    candidates.push(c);
                }
            }
        }
    }
    let found = verify(fetcher, candidates.into_iter(), now).await;
    if found.is_empty() {
        return Err("Unter dieser Adresse habe ich keinen Feed gefunden.".into());
    }
    Ok(found)
}

/// Adressen gleichzeitig abrufen und nur behalten, was sich als Feed lesen lässt – Feeds
/// mit Einträgen zuerst, sonst in der gegebenen Reihenfolge. Titel laut Feed.
async fn verify(
    fetcher: &Fetcher,
    urls: impl Iterator<Item = String>,
    now: i64,
) -> Vec<DiscoveredFeed> {
    let checks = urls.map(|u| async move {
        let page = fetcher.get(&u).await.ok()?;
        let feed = parse::parse(&page.body, &u, now).ok()?;
        Some((
            feed.entries.is_empty(),
            DiscoveredFeed {
                url: u,
                title: feed.title,
                kind: feed.kind.into(),
            },
        ))
    });
    let mut found: Vec<(bool, DiscoveredFeed)> = futures::future::join_all(checks)
        .await
        .into_iter()
        .flatten()
        .collect();
    // stabil: leere Feeds nach hinten
    found.sort_by_key(|(empty, _)| *empty);
    found.into_iter().map(|(_, f)| f).collect()
}

static LINK: LazyLock<scraper::Selector> =
    LazyLock::new(|| scraper::Selector::parse("link[href]").expect("Selektor"));
static TITLE: LazyLock<scraper::Selector> =
    LazyLock::new(|| scraper::Selector::parse("title").expect("Selektor"));

fn rel_has(rel: &str, token: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|t| t.eq_ignore_ascii_case(token))
}

/// `<link rel="alternate" type="application/rss+xml" …>` usw. einer HTML-Seite.
pub fn feed_links(html: &str, base: &Url) -> Vec<DiscoveredFeed> {
    let doc = scraper::Html::parse_document(html);
    let page_title = doc
        .select(&TITLE)
        .next()
        .map(|t| super::sanitize::collapse(&t.text().collect::<String>()))
        .unwrap_or_default();
    let mut out: Vec<DiscoveredFeed> = Vec::new();
    for link in doc.select(&LINK) {
        let el = link.value();
        if !rel_has(el.attr("rel").unwrap_or_default(), "alternate") {
            continue;
        }
        let kind_attr = el
            .attr("type")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let href = el.attr("href").unwrap_or_default();
        let title = el
            .attr("title")
            .map(super::sanitize::collapse)
            .unwrap_or_default();
        let kind = match kind_attr.as_str() {
            "application/rss+xml" | "application/rdf+xml" | "application/xml" | "text/xml" => "rss",
            "application/atom+xml" => "atom",
            "application/feed+json" => "json",
            // WordPress verlinkt seine REST-API ebenfalls als JSON – nur echte Feeds nehmen
            "application/json"
                if href.to_ascii_lowercase().contains("feed")
                    || title.to_ascii_lowercase().contains("feed") =>
            {
                "json"
            }
            _ => continue,
        };
        let Some(url) = resolve(href, Some(base)) else {
            continue;
        };
        if out.iter().any(|f| f.url == url) {
            continue;
        }
        out.push(DiscoveredFeed {
            url,
            title: if title.is_empty() {
                page_title.clone()
            } else {
                title
            },
            kind: kind.into(),
        });
    }
    out
}

static ANCHOR: LazyLock<scraper::Selector> =
    LazyLock::new(|| scraper::Selector::parse("a[href]").expect("Selektor"));

/// Links im Text einer Seite, die nach einem Feed aussehen (`…/rss`, `…/feed`, `….xml`).
pub fn anchor_links(html: &str, base: &Url) -> Vec<String> {
    let doc = scraper::Html::parse_document(html);
    let mut out: Vec<String> = Vec::new();
    for a in doc.select(&ANCHOR) {
        let Some(url) = a.value().attr("href").and_then(|h| resolve(h, Some(base))) else {
            continue;
        };
        let path = Url::parse(&url)
            .map(|u| u.path().to_ascii_lowercase())
            .unwrap_or_default();
        let looks = path.contains("rss")
            || path.contains("atom")
            || path.ends_with("/feed")
            || path.ends_with("/feed/")
            || path.ends_with(".xml")
            || (path.ends_with(".json") && path.contains("feed"));
        if looks && !path.contains("sitemap") && !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// Icon aus dem HTML einer Seite: bevorzugt gross genug für scharfe Anzeige.
pub fn icon_link(html: &str, base: &Url) -> Option<String> {
    let doc = scraper::Html::parse_document(html);
    let mut best: Option<(u8, String)> = None;
    for link in doc.select(&LINK) {
        let el = link.value();
        let rel = el.attr("rel").unwrap_or_default();
        let sizes = el.attr("sizes").unwrap_or_default().to_ascii_lowercase();
        let kind = el.attr("type").unwrap_or_default().to_ascii_lowercase();
        let large = sizes == "any"
            || kind.contains("svg")
            || sizes
                .split(['x', ' '])
                .filter_map(|n| n.parse::<u32>().ok())
                .any(|n| n >= 32);
        let score =
            if rel_has(rel, "apple-touch-icon") || rel_has(rel, "apple-touch-icon-precomposed") {
                3
            } else if rel_has(rel, "icon") {
                if large {
                    4
                } else {
                    2
                }
            } else {
                continue;
            };
        let Some(url) = el.attr("href").and_then(|h| resolve(h, Some(base))) else {
            continue;
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, url));
        }
    }
    best.map(|(_, u)| u)
}

/// Icon zu einem Feed: aus der Webseite, sonst das Icon laut Feed, sonst `/favicon.ico`
/// (nur wenn es dort wirklich ein Bild gibt). Leer = keins gefunden.
pub async fn find_icon(
    fetcher: &Fetcher,
    site_url: &str,
    feed_url: &str,
    feed_icon: &str,
) -> String {
    let site = Url::parse(site_url).or_else(|_| Url::parse(feed_url));
    let Ok(site) = site else {
        return feed_icon.to_string();
    };
    if !site_url.is_empty() {
        if let Ok(page) = fetcher.get(site.as_str()).await {
            let base = Url::parse(&page.url).unwrap_or_else(|_| site.clone());
            if page.content_type.is_empty() || page.content_type.contains("html") {
                if let Some(icon) = icon_link(&String::from_utf8_lossy(&page.body), &base) {
                    return icon;
                }
            }
        }
    }
    if !feed_icon.is_empty() {
        return feed_icon.to_string();
    }
    match site.join("/favicon.ico") {
        Ok(fav) if fetcher.is_image(fav.as_str()).await => fav.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_input() {
        assert_eq!(
            normalize_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(
            normalize_url(" feed://example.com/rss ").unwrap().as_str(),
            "https://example.com/rss"
        );
        assert_eq!(
            normalize_url("http://example.com/a").unwrap().as_str(),
            "http://example.com/a"
        );
        assert!(normalize_url("ftp://example.com").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn finds_links_and_icon() {
        let base = Url::parse("https://example.org/blog/").unwrap();
        let html = r#"<html><head><title>Mein Blog</title>
            <link rel="alternate" type="application/rss+xml" href="/feed.xml" title="RSS">
            <link rel="alternate" type="application/atom+xml" href="atom.xml">
            <link rel="alternate" type="application/json" href="/wp-json/wp/v2/pages/1">
            <link rel="icon" href="/favicon-16.png" sizes="16x16">
            <link rel="apple-touch-icon" href="/apple.png">
            </head></html>"#;
        let found = feed_links(html, &base);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].url, "https://example.org/feed.xml");
        assert_eq!(found[0].kind, "rss");
        assert_eq!(found[1].url, "https://example.org/blog/atom.xml");
        assert_eq!(found[1].title, "Mein Blog");
        assert_eq!(
            icon_link(html, &base).as_deref(),
            Some("https://example.org/apple.png")
        );
    }
}

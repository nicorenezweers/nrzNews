//! Feeds lesen (RSS 0.9x/1.0/2.0, Atom, JSON Feed – über feed-rs) und in das eigene Modell
//! bringen: ein Titel ohne Markup, absolute Adressen, gesäubertes HTML, reiner Text, ein
//! Vorschaubild und ein Datum, das nie in der Zukunft liegt.
//!
//! Feeds sind oft unvollständig, deshalb gibt es für fast alles eine Ersatzquelle:
//! * ID: `guid`/`id` → Link → SHA-256 aus Titel und Datum (stabil über Abrufe hinweg,
//!   sonst kämen Artikel ohne ID bei jedem Abruf neu).
//! * Inhalt: voller Inhalt → Zusammenfassung → `media:description` (YouTube).
//! * Datum: veröffentlicht → geändert → jetzt; Zukunft wird auf jetzt gekürzt, damit ein
//!   falsch datierter Artikel nicht wochenlang oben klebt.
//! * Bild: `media:thumbnail`/`media:content` → Bild-Enclosure → erstes `<img>` im Inhalt.
//!
//! Podcasts: Audio- und Video-Enclosures werden als `<audio controls>`/`<video controls>`
//! an den Inhalt gehängt, damit man die Folge direkt im Lesemodus hört.

use super::sanitize;
use sha2::{Digest, Sha256};
use url::Url;

/// Höchstens so viele Einträge je Abruf (manche Feeds liefern ihr ganzes Archiv).
const MAX_ENTRIES: usize = 500;

/// Ein gelesener Feed.
#[derive(Debug, Clone)]
pub struct ParsedFeed {
    /// "rss", "atom" oder "json"
    pub kind: &'static str,
    pub title: String,
    /// Webseite zum Feed (absolut, leer = unbekannt)
    pub site_url: String,
    pub description: String,
    /// Icon laut Feed (Atom `<icon>`, JSON Feed `favicon`); leer = keins
    pub icon_url: String,
    pub entries: Vec<ParsedEntry>,
}

/// Ein Artikel, fertig zum Speichern.
#[derive(Debug, Clone)]
pub struct ParsedEntry {
    pub guid: String,
    pub url: String,
    pub title: String,
    pub author: String,
    /// Gesäubert (ammonia)
    pub content_html: String,
    /// Reiner Text, Leerraum zusammengefasst
    pub content_text: String,
    pub image_url: String,
    pub published_at: i64,
}

/// Liest einen Feed. `feed_url` dient als Basis für relative Adressen, `now` als Datum für
/// Einträge ohne Datum und als Obergrenze.
pub fn parse(body: &[u8], feed_url: &str, now: i64) -> Result<ParsedFeed, String> {
    let parser = feed_rs::parser::Builder::new()
        .base_uri(Some(feed_url))
        // Eigene Ersatz-IDs (siehe oben) statt der zufälligen UUIDs von feed-rs
        .id_generator(|_, _, _| String::new())
        // Gesäubert wird selbst – mit eigener Liste und aufgelösten Adressen
        .sanitize_content(false)
        .build();
    let feed = parser
        .parse(body)
        .map_err(|_| "Das ist kein Feed (RSS, Atom oder JSON Feed).".to_string())?;
    let feed_base = Url::parse(feed_url).ok();

    let site_url = feed
        .links
        .iter()
        .filter(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .filter(|l| {
            l.media_type
                .as_deref()
                .is_none_or(|t| !(t.contains("xml") || t.contains("json")))
        })
        .filter_map(|l| sanitize::resolve(&l.href, feed_base.as_ref()))
        .find(|u| u != feed_url)
        .unwrap_or_default();
    let site_base = Url::parse(&site_url).ok().or_else(|| feed_base.clone());

    let title = feed
        .title
        .as_ref()
        .map(|t| sanitize::html_to_text(&t.content))
        .filter(|t| !t.is_empty())
        .or_else(|| {
            site_base
                .as_ref()
                .and_then(|u| u.host_str())
                .map(String::from)
        })
        .unwrap_or_else(|| feed_url.to_string());
    let description = feed
        .description
        .as_ref()
        .map(|d| truncate(&sanitize::html_to_text(&d.content), 1000))
        .unwrap_or_default();
    let icon_url = feed
        .icon
        .as_ref()
        .and_then(|i| sanitize::resolve(&i.uri, site_base.as_ref()))
        .unwrap_or_default();
    let feed_author = feed
        .authors
        .iter()
        .filter_map(|p| person_name(p.name.as_deref()))
        .next()
        .unwrap_or_default();

    // feed-rs verwirft `media:thumbnail`/`itunes:image` ohne `media:content` – selbst lesen,
    // aber nur, wenn die Einträge eindeutig zugeordnet werden können
    let thumbs = raw_thumbnails(body).filter(|t| t.len() == feed.entries.len());
    let entries = feed
        .entries
        .iter()
        .take(MAX_ENTRIES)
        .enumerate()
        .map(|(i, e)| {
            let thumb = thumbs.as_ref().and_then(|t| t[i].as_deref());
            entry(e, site_base.as_ref(), &feed_author, thumb, now)
        })
        .collect();

    Ok(ParsedFeed {
        kind: match feed.feed_type {
            feed_rs::model::FeedType::Atom => "atom",
            feed_rs::model::FeedType::JSON => "json",
            _ => "rss",
        },
        title: truncate(&title, 300),
        site_url,
        description,
        icon_url,
        entries,
    })
}

fn entry(
    e: &feed_rs::model::Entry,
    site: Option<&Url>,
    feed_author: &str,
    thumbnail: Option<&str>,
    now: i64,
) -> ParsedEntry {
    let is_media = |t: Option<&str>| {
        t.is_some_and(|t| {
            t.starts_with("audio/") || t.starts_with("video/") || t.starts_with("image/")
        })
    };
    let url = e
        .links
        .iter()
        .filter(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        // `<comments>` landet bei feed-rs ebenfalls als Link (mit Ziel „Kommentare“)
        .filter(|l| l.target.is_none())
        .filter(|l| !is_media(l.media_type.as_deref()))
        .find_map(|l| sanitize::resolve(&l.href, site))
        .or_else(|| {
            // RSS: <guid isPermaLink="true"> ist oft der einzige Link
            let id = e.id.trim();
            (id.starts_with("http://") || id.starts_with("https://"))
                .then(|| sanitize::resolve(id, site))
                .flatten()
        })
        .unwrap_or_default();
    // Basis für relative Adressen im Inhalt: xml:base, sonst der Artikel, sonst die Webseite
    let base = e
        .base
        .as_deref()
        .and_then(|b| Url::parse(b).ok())
        .or_else(|| Url::parse(&url).ok())
        .or_else(|| site.cloned());

    // Inhalt: voller Inhalt → Zusammenfassung → media:description
    let raw = e
        .content
        .as_ref()
        .and_then(|c| {
            let body = c.body.as_deref()?.trim();
            (!body.is_empty()).then(|| as_html(c.content_type.as_str(), body))
        })
        .or_else(|| {
            e.summary
                .as_ref()
                .filter(|s| !s.content.trim().is_empty())
                .map(|s| as_html(s.content_type.as_str(), &s.content))
        })
        .or_else(|| {
            e.media.iter().find_map(|m| {
                m.description
                    .as_ref()
                    .filter(|d| !d.content.trim().is_empty())
                    .map(|d| as_html(d.content_type.as_str(), &d.content))
            })
        })
        .unwrap_or_default();

    // Podcasts & Videos: Enclosures als Player anhängen (keine doppelten)
    let mut players = String::new();
    let mut seen: Vec<String> = Vec::new();
    let enclosures = e
        .media
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| {
            let url = c.url.as_ref()?.to_string();
            let kind = c
                .content_type
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default();
            Some((url, kind))
        })
        .chain(
            e.links
                .iter()
                .filter(|l| {
                    l.rel.as_deref() == Some("enclosure") || is_media(l.media_type.as_deref())
                })
                .map(|l| (l.href.clone(), l.media_type.clone().unwrap_or_default())),
        );
    for (href, kind) in enclosures {
        let Some(abs) = sanitize::resolve(&href, base.as_ref()) else {
            continue;
        };
        let tag = if kind.starts_with("audio/") || (kind.is_empty() && is_audio_file(&abs)) {
            "audio"
        } else if kind.starts_with("video/") || (kind.is_empty() && is_video_file(&abs)) {
            "video"
        } else {
            continue;
        };
        if seen.contains(&abs) || raw.contains(&abs) || raw.contains(&href) {
            continue;
        }
        players.push_str(&format!(
            "<p><{tag} controls preload=\"none\" src=\"{}\"></{tag}></p>",
            sanitize::escape(&abs)
        ));
        seen.push(abs);
    }
    let content_html = sanitize::clean_html(&format!("{raw}{players}"), base.as_ref());
    let content_text = sanitize::html_to_text(&content_html);

    let title = e
        .title
        .as_ref()
        .map(|t| sanitize::html_to_text(&t.content))
        .filter(|t| !t.is_empty())
        // Ohne Titel (Mikroblogs): Anfang des Textes
        .or_else(|| (!content_text.is_empty()).then(|| sanitize::excerpt(&content_text, 80)))
        .unwrap_or_else(|| "Ohne Titel".into());

    let author = {
        let names: Vec<String> = e
            .authors
            .iter()
            .filter_map(|p| person_name(p.name.as_deref()))
            .collect();
        if names.is_empty() {
            feed_author.to_string()
        } else {
            names.join(", ")
        }
    };

    let date = e.published.or(e.updated).map(|d| d.timestamp());
    let published_at = match date {
        Some(ts) if ts > 0 => ts.min(now),
        _ => now,
    };

    let guid = {
        let id = e.id.trim();
        if !id.is_empty() {
            id.to_string()
        } else if !url.is_empty() {
            url.clone()
        } else {
            // Ohne ID und Link: Titel und (ursprüngliches) Datum – nicht `now`, sonst wäre
            // der Eintrag bei jedem Abruf neu
            let mut h = Sha256::new();
            h.update(title.as_bytes());
            h.update(date.map(|d| d.to_string()).unwrap_or_default().as_bytes());
            h.update(
                content_text
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .as_bytes(),
            );
            hex::encode(h.finalize())
        }
    };
    // Sehr lange IDs (ganze Adressen mit Parametern) kurz halten
    let guid = if guid.len() > 512 {
        hex::encode(Sha256::digest(guid.as_bytes()))
    } else {
        guid
    };

    let image_url = thumbnail
        .and_then(|t| sanitize::resolve(t, base.as_ref()))
        .or_else(|| image(e, base.as_ref()))
        .or_else(|| sanitize::first_image(&content_html))
        .unwrap_or_default();

    ParsedEntry {
        guid,
        url,
        title: truncate(&title, 500),
        author: truncate(&author, 200),
        content_html,
        content_text,
        image_url,
        published_at,
    }
}

/// Vorschaubild aus Media RSS (Thumbnail, Bild-Inhalt) oder einem Bild-Enclosure.
fn image(e: &feed_rs::model::Entry, base: Option<&Url>) -> Option<String> {
    let thumbs = e
        .media
        .iter()
        .flat_map(|m| m.thumbnails.iter())
        .map(|t| t.image.uri.clone());
    let contents = e
        .media
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| {
            let url = c.url.as_ref()?.to_string();
            let kind = c
                .content_type
                .as_ref()
                .map(|t| t.to_string())
                .unwrap_or_default();
            (kind.starts_with("image/") || (kind.is_empty() && is_image_file(&url))).then_some(url)
        });
    let links = e
        .links
        .iter()
        .filter(|l| {
            l.media_type
                .as_deref()
                .is_some_and(|t| t.starts_with("image/"))
        })
        .map(|l| l.href.clone());
    thumbs
        .chain(contents)
        .chain(links)
        .find_map(|u| sanitize::resolve(&u, base))
}

/// Erstes `media:thumbnail` bzw. `itunes:image` je `<item>`/`<entry>` direkt aus dem XML, in
/// der Reihenfolge der Einträge. `None` bei JSON oder wenn sich nichts zuordnen lässt.
fn raw_thumbnails(body: &[u8]) -> Option<Vec<Option<String>>> {
    static ITEM: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?s)<(?:item|entry)[\s>].*?</(?:item|entry)>").expect("Regex")
    });
    static THUMB: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"<[A-Za-z0-9_]+:(?:thumbnail\b[^>]*?\burl|image\b[^>]*?\bhref)\s*=\s*["']([^"']+)["']"#,
        )
        .expect("Regex")
    });
    let text = String::from_utf8_lossy(body);
    if !text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
    {
        return None;
    }
    let found: Vec<Option<String>> = ITEM
        .find_iter(&text)
        .map(|item| {
            THUMB.captures(item.as_str()).map(|c| {
                c[1].replace("&amp;", "&")
                    .replace("&quot;", "\"")
                    .replace("&#38;", "&")
                    .trim()
                    .to_string()
            })
        })
        .collect();
    found.iter().any(Option::is_some).then_some(found)
}

/// Text (laut Typ) als HTML. Als „Text“ markiertes HTML kommt vor – dann gilt das Markup.
fn as_html(content_type: &str, body: &str) -> String {
    if content_type == "text/plain" && !looks_like_html(body) {
        sanitize::text_to_html(body)
    } else {
        body.to_string()
    }
}

/// Enthält der Text gängige HTML-Tags oder Entities? Unbekannte „Tags“ wie `<Klammern>` in
/// echtem Text zählen nicht.
fn looks_like_html(s: &str) -> bool {
    static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)</?(p|br|a|b|i|u|em|strong|img|div|span|ul|ol|li|h[1-6]|blockquote|pre|code|figure|table|hr)(\s[^<>]*)?/?>|&[a-z]+;|&#x?[0-9a-f]+;",
        )
        .expect("Regex")
    });
    TAG.is_match(s)
}

/// Name einer Person; "mail@example.org (Name)" (RSS) wird zu "Name".
fn person_name(name: Option<&str>) -> Option<String> {
    let name = sanitize::collapse(name?);
    let name = match (name.find('('), name.rfind(')')) {
        (Some(a), Some(b)) if a < b && name[..a].contains('@') => name[a + 1..b].trim().to_string(),
        // feed-rs trennt die Adresse schon ab und lässt "(Name)" stehen
        (Some(0), Some(b)) if b == name.len() - 1 => name[1..b].trim().to_string(),
        _ => name,
    };
    (!name.is_empty() && !name.contains('@')).then_some(name)
}

fn file_ext(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_audio_file(url: &str) -> bool {
    matches!(
        file_ext(url).as_str(),
        "mp3" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wav" | "flac"
    )
}

fn is_video_file(url: &str) -> bool {
    matches!(file_ext(url).as_str(), "mp4" | "m4v" | "webm" | "mov")
}

fn is_image_file(url: &str) -> bool {
    matches!(
        file_ext(url).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif"
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_names() {
        assert_eq!(
            person_name(Some("mail@example.org (Anna Muster)")).as_deref(),
            Some("Anna Muster")
        );
        assert_eq!(person_name(Some("  Bob  ")).as_deref(), Some("Bob"));
        assert_eq!(person_name(Some("nur@mail.org")), None);
    }

    #[test]
    fn html_detection() {
        assert!(looks_like_html("Hallo <b>Welt</b>"));
        assert!(looks_like_html("Tom &amp; Jerry"));
        assert!(!looks_like_html("a < b und c > d"));
        assert!(!looks_like_html("Zweiter Absatz <kein Tag>"));
        assert!(looks_like_html("Zeile<br/>Zeile"));
    }
}

//! HTML aus Feeds säubern (ammonia) und daraus reinen Text machen.
//!
//! Feeds sind fremde Inhalte: die Oberfläche zeigt sie per `dangerous_inner_html`, also darf
//! nur harmloses Markup durch – Textauszeichnung, Überschriften, Listen, Tabellen, Code,
//! Zitate, Bilder, Links, Audio und Video. Skripte, Styles, iframes und Formulare fliegen
//! raus. Relative Adressen werden gegen die Adresse des Artikels aufgelöst (sonst zeigten
//! sie auf nrzNews selbst), Links öffnen in einem neuen Tab ohne `opener`, Bilder laden erst
//! beim Scrollen, und 1×1-Zählpixel verschwinden.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use url::Url;

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "audio",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "picture",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// Wörter je Minute für die Lesezeit.
const WORDS_PER_MINUTE: usize = 200;

/// Säubert HTML eines Artikels. `base` ist die Adresse des Artikels (sonst der Webseite),
/// gegen die relative Links und Bilder aufgelöst werden; ohne `base` fallen sie weg.
pub fn clean_html(html: &str, base: Option<&Url>) -> String {
    let mut attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut allow = |tag: &'static str, list: &[&'static str]| {
        attrs.insert(tag, list.iter().copied().collect());
    };
    allow("a", &["href", "title"]);
    allow("img", &["src", "srcset", "alt", "title", "width", "height"]);
    allow("audio", &["src", "controls", "preload", "loop", "muted"]);
    allow(
        "video",
        &[
            "src",
            "controls",
            "preload",
            "poster",
            "width",
            "height",
            "loop",
            "muted",
            "playsinline",
        ],
    );
    allow("source", &["src", "srcset", "type", "media", "sizes"]);
    allow("abbr", &["title"]);
    allow("blockquote", &["cite"]);
    allow("q", &["cite"]);
    allow("del", &["cite", "datetime"]);
    allow("ins", &["cite", "datetime"]);
    allow("time", &["datetime"]);
    allow("td", &["colspan", "rowspan", "align"]);
    allow("th", &["colspan", "rowspan", "align", "scope"]);
    allow("ol", &["start", "reversed", "type"]);
    allow("li", &["value"]);
    allow("details", &["open"]);
    allow("col", &["span"]);
    allow("colgroup", &["span"]);

    let mut builder = ammonia::Builder::default();
    builder
        .tags(TAGS.iter().copied().collect())
        .tag_attributes(attrs)
        .generic_attributes(["lang", "dir"].into_iter().collect())
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .set_tag_attribute_value("img", "loading", "lazy")
        .set_tag_attribute_value("img", "decoding", "async")
        // Mitsamt Inhalt entfernen (sonst bliebe z. B. der Text eines Formulars stehen)
        .clean_content_tags(
            [
                "script", "style", "iframe", "object", "embed", "template", "form", "button",
                "select", "textarea", "svg", "math",
            ]
            .into_iter()
            .collect(),
        )
        .strip_comments(true);
    match base {
        Some(b) => {
            builder.url_relative(ammonia::UrlRelative::RewriteWithBase(b.clone()));
            // `srcset` kennt ammonia nicht als Adresse: selbst auflösen
            let b = b.clone();
            builder.attribute_filter(move |_element, attribute, value| {
                if attribute == "srcset" {
                    Some(Cow::Owned(resolve_srcset(value, Some(&b))))
                } else {
                    Some(Cow::Borrowed(value))
                }
            });
        }
        None => {
            builder.url_relative(ammonia::UrlRelative::Deny);
            builder.attribute_filter(|_element, attribute, value| {
                if attribute == "srcset" {
                    Some(Cow::Owned(resolve_srcset(value, None)))
                } else {
                    Some(Cow::Borrowed(value))
                }
            });
        }
    }
    let cleaned = builder.clean(html).to_string();
    drop_tracking_pixels(&cleaned).trim().to_string()
}

/// `srcset="a.jpg 1x, /b.jpg 2x"`: jede Adresse absolut machen, unsichere weglassen.
fn resolve_srcset(value: &str, base: Option<&Url>) -> String {
    value
        .split(',')
        .filter_map(|candidate| {
            let mut parts = candidate.split_whitespace();
            let url = parts.next()?;
            let descriptor: Vec<&str> = parts.collect();
            let absolute = resolve(url, base)?;
            Some(if descriptor.is_empty() {
                absolute
            } else {
                format!("{absolute} {}", descriptor.join(" "))
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Macht eine Adresse absolut (nur http/https); `None`, wenn das nicht geht.
pub fn resolve(href: &str, base: Option<&Url>) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    let url = match Url::parse(href) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(href).ok()?,
        Err(_) => return None,
    };
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// Bilder mit höchstens 1×1 Pixel (Zählpixel) entfernen. Läuft auf der Ausgabe von ammonia,
/// deren Attribute immer gleich geschrieben sind (`name="wert"`).
fn drop_tracking_pixels(html: &str) -> Cow<'_, str> {
    static IMG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<img\b[^>]*>").expect("Regex"));
    static DIM: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"\b(width|height)="\s*(\d+)(?:px)?\s*""#).expect("Regex")
    });
    IMG.replace_all(html, |caps: &regex::Captures| {
        let tag = &caps[0];
        let dims: Vec<u32> = DIM
            .captures_iter(tag)
            .filter_map(|c| c[2].parse().ok())
            .collect();
        let pixel = !dims.is_empty() && dims.iter().all(|d| *d <= 1);
        if pixel || is_tracker(tag) {
            String::new()
        } else {
            tag.to_string()
        }
    })
}

/// Bekannte Zähldienste in Feeds (Feedburner, WordPress-Statistik, …).
fn is_tracker(img_tag: &str) -> bool {
    const HOSTS: &[&str] = &[
        "feeds.feedburner.com/~r/",
        "feeds.feedburner.com/~ff/",
        "pixel.wp.com/",
        "stats.wordpress.com/",
        "pixel.quantserve.com/",
        "www.google-analytics.com/",
        "feedsportal.com/c/",
    ];
    HOSTS.iter().any(|h| img_tag.contains(h))
}

/// Elemente, nach denen im Text ein Abstand steht (sonst klebten Absätze aneinander).
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "img"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "td"
            | "th"
            | "tr"
            | "ul"
            | "summary"
            | "details"
            | "audio"
            | "video"
    )
}

/// Reiner Text aus HTML: Entities aufgelöst, Tags weg, Leerraum zusammengefasst. Inhalte
/// von `script`, `style` und `template` zählen nicht.
pub fn html_to_text(html: &str) -> String {
    if !html.contains('<') && !html.contains('&') {
        return collapse(html);
    }
    enum Step<N> {
        Visit(N),
        Space,
    }
    let doc = scraper::Html::parse_fragment(html);
    let mut out = String::with_capacity(html.len() / 2);
    let mut stack = vec![Step::Visit(doc.tree.root())];
    while let Some(step) = stack.pop() {
        let node = match step {
            Step::Visit(n) => n,
            Step::Space => {
                out.push(' ');
                continue;
            }
        };
        match node.value() {
            scraper::Node::Text(t) => out.push_str(t),
            scraper::Node::Element(e) => {
                let name = e.name();
                if matches!(name, "script" | "style" | "template" | "noscript") {
                    continue;
                }
                if is_block(name) {
                    out.push(' ');
                    stack.push(Step::Space);
                }
                stack.extend(node.children().rev().map(Step::Visit));
            }
            _ => stack.extend(node.children().rev().map(Step::Visit)),
        }
    }
    collapse(&out)
}

/// Leerraum (auch geschützte Leerzeichen) zu einem Leerzeichen zusammenfassen.
pub fn collapse(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c == '\u{a0}')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lesezeit in Minuten (~200 Wörter je Minute, mindestens 1).
pub fn reading_minutes(text: &str) -> u32 {
    let words = text.split_whitespace().count();
    words.div_ceil(WORDS_PER_MINUTE).max(1) as u32
}

/// Auszug für die Liste: höchstens `max` Zeichen, an einer Wortgrenze gekürzt, mit „…“.
pub fn excerpt(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let short = match cut.rfind(char::is_whitespace) {
        // Nur an der Wortgrenze kürzen, wenn dabei nicht zu viel verloren geht
        Some(pos) if pos > cut.len() / 2 => &cut[..pos],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        short.trim_end_matches(|c: char| c.is_whitespace() || ",;:.-–(".contains(c))
    )
}

/// Erstes Bild im (gesäuberten) HTML – als Vorschaubild für die Liste.
pub fn first_image(html: &str) -> Option<String> {
    if !html.contains("<img") {
        return None;
    }
    static IMG: LazyLock<scraper::Selector> =
        LazyLock::new(|| scraper::Selector::parse("img[src]").expect("Selektor"));
    let doc = scraper::Html::parse_fragment(html);
    doc.select(&IMG)
        .filter_map(|img| img.value().attr("src"))
        .map(str::trim)
        .find(|src| src.starts_with("http://") || src.starts_with("https://"))
        .map(String::from)
}

/// Text als HTML-Absätze (für Feeds, die nur reinen Text liefern).
pub fn text_to_html(text: &str) -> String {
    let esc = escape(text);
    esc.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", p.replace('\n', "<br>")))
        .collect()
}

/// Für HTML-Text und -Attribute maskieren.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_word_boundaries() {
        assert_eq!(
            html_to_text(
                "<p>Erster&nbsp;Absatz</p><p>Zweiter <b>fe</b>tter</p><script>x()</script>"
            ),
            "Erster Absatz Zweiter fetter"
        );
        assert_eq!(html_to_text("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(html_to_text("  a \n b "), "a b");
    }

    #[test]
    fn excerpt_on_word_boundary() {
        assert_eq!(excerpt("kurz", 300), "kurz");
        let e = excerpt("eins zwei drei vier fünf sechs", 12);
        assert_eq!(e, "eins zwei…");
    }

    #[test]
    fn reading_time() {
        assert_eq!(reading_minutes(""), 1);
        assert_eq!(reading_minutes(&"wort ".repeat(450)), 3);
    }

    #[test]
    fn srcset_is_resolved() {
        let base = Url::parse("https://example.org/blog/post").ok();
        assert_eq!(
            resolve_srcset("a.jpg 1x, /b.jpg 2x, javascript:alert(1) 3x", base.as_ref()),
            "https://example.org/blog/a.jpg 1x, https://example.org/b.jpg 2x"
        );
    }
}

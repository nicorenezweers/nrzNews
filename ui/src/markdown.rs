//! Kleines, sicheres Markdown → RSX für Texte der KI (Zusammenfassung, Überblick, Dieter).
//! Nie als HTML eingefügt: Überschriften, Aufzählungen (auch nummeriert), Zitate, Code,
//! **fett**, *kursiv*, `Code`, Links (nur http/https/mailto) und Verweise `[1]`, `[2, 3]` auf
//! Quellen – die führen zum Artikel im Lesemodus.

use crate::Route;
use dioxus::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Code(String),
    Link(Vec<Inline>, String),
    /// Verweise auf Quellen (1-basiert)
    Cite(Vec<usize>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    Bullets(Vec<Vec<Inline>>),
    Numbered(u32, Vec<Vec<Inline>>),
    Quote(Vec<Inline>),
    Code(String),
    Rule,
}

// ------------------------------------------------------------------ Zeilen

fn bullet(line: &str) -> Option<&str> {
    ["- ", "* ", "• ", "+ "].iter().find_map(|p| line.strip_prefix(p))
}

fn numbered(line: &str) -> Option<(u32, &str)> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let rest = &line[digits..];
    let text = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((line[..digits].parse().ok()?, text))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) {
        let rest = &line[level..];
        if let Some(text) = rest.strip_prefix(' ') {
            return Some((level as u8, text.trim().trim_end_matches('#').trim()));
        }
    }
    None
}

fn is_rule(line: &str) -> bool {
    let l: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    l.len() >= 3 && (l.chars().all(|c| c == '-') || l.chars().all(|c| c == '*') || l.chars().all(|c| c == '_'))
}

/// Text in Blöcke zerlegen.
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    // Offene Liste: (nummeriert?, Start, Einträge)
    let mut list: Option<(bool, u32, Vec<String>)> = None;
    let mut lines = text.lines().peekable();

    fn flush(blocks: &mut Vec<Block>, para: &mut Vec<String>, quote: &mut Vec<String>, list: &mut Option<(bool, u32, Vec<String>)>) {
        if !para.is_empty() {
            blocks.push(Block::Para(inline(&para.join(" "))));
            para.clear();
        }
        if !quote.is_empty() {
            blocks.push(Block::Quote(inline(&quote.join(" "))));
            quote.clear();
        }
        if let Some((num, start, items)) = list.take() {
            let items = items.iter().map(|i| inline(i)).collect();
            blocks.push(if num { Block::Numbered(start, items) } else { Block::Bullets(items) });
        }
    }

    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.starts_with("```") {
            flush(&mut blocks, &mut para, &mut quote, &mut list);
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim().starts_with("```") {
                    break;
                }
                code.push(l);
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        if line.is_empty() {
            flush(&mut blocks, &mut para, &mut quote, &mut list);
            continue;
        }
        if let Some((level, text)) = heading(line) {
            flush(&mut blocks, &mut para, &mut quote, &mut list);
            blocks.push(Block::Heading(level, inline(text)));
            continue;
        }
        if is_rule(line) {
            flush(&mut blocks, &mut para, &mut quote, &mut list);
            blocks.push(Block::Rule);
            continue;
        }
        if let Some(q) = line.strip_prefix('>') {
            if !para.is_empty() || list.is_some() {
                flush(&mut blocks, &mut para, &mut quote, &mut list);
            }
            quote.push(q.trim().to_string());
            continue;
        }
        let item = bullet(line).map(|t| (false, 1, t)).or_else(|| numbered(line).map(|(n, t)| (true, n, t)));
        if let Some((num, start, text)) = item {
            match &mut list {
                Some((is_num, _, items)) if *is_num == num => items.push(text.trim().to_string()),
                _ => {
                    flush(&mut blocks, &mut para, &mut quote, &mut list);
                    list = Some((num, start, vec![text.trim().to_string()]));
                }
            }
            continue;
        }
        // Eingerückte Fortsetzung eines Listeneintrags
        if let Some((_, _, items)) = &mut list {
            if raw.starts_with(' ') || raw.starts_with('\t') {
                if let Some(last) = items.last_mut() {
                    last.push(' ');
                    last.push_str(line);
                    continue;
                }
            }
        }
        if !quote.is_empty() || list.is_some() {
            flush(&mut blocks, &mut para, &mut quote, &mut list);
        }
        para.push(line.to_string());
    }
    flush(&mut blocks, &mut para, &mut quote, &mut list);
    blocks
}

// ------------------------------------------------------------------ Inline

fn find(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    if pat.is_empty() || chars.len() < pat.len() {
        return None;
    }
    (from..=chars.len() - pat.len()).find(|&i| chars[i..i + pat.len()] == *pat)
}

/// `[1]`, `[1, 2]`, `[1,2,3]` → Nummern
fn citation(inner: &str) -> Option<Vec<usize>> {
    let nums: Vec<usize> = inner
        .split([',', ';'])
        .map(|p| p.trim())
        .map(|p| if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() && p.len() <= 3 { p.parse().ok() } else { None })
        .collect::<Option<Vec<usize>>>()?;
    (!nums.is_empty() && nums.iter().all(|n| *n > 0)).then_some(nums)
}

fn safe_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")
}

/// Zeile in Textteile zerlegen. Unvollständige Auszeichnung bleibt als Text stehen.
pub fn inline(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Inline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let push_text = |buf: &mut String, out: &mut Vec<Inline>| {
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(buf)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        // `Code`
        if c == '`' {
            if let Some(end) = find(&chars, i + 1, &['`']) {
                push_text(&mut buf, &mut out);
                out.push(Inline::Code(chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        }
        // **fett** / __fett__
        if (c == '*' || c == '_') && chars.get(i + 1) == Some(&c) {
            if let Some(end) = find(&chars, i + 2, &[c, c]) {
                if end > i + 2 {
                    push_text(&mut buf, &mut out);
                    let inner: String = chars[i + 2..end].iter().collect();
                    out.push(Inline::Bold(inline(&inner)));
                    i = end + 2;
                    continue;
                }
            }
        }
        // *kursiv* / _kursiv_ (Unterstrich nur an Wortgrenzen, nicht in snake_case)
        if c == '*' || c == '_' {
            let prev_ok = c == '*' || i == 0 || !chars[i - 1].is_alphanumeric();
            // Nicht leer und kein doppeltes Zeichen (ein offenes ** bleibt Text)
            let next_ok = chars.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != c);
            if prev_ok && next_ok {
                let mut j = i + 2;
                let mut end = None;
                while j < chars.len() {
                    if chars[j] == c && !chars[j - 1].is_whitespace() && (c == '*' || chars.get(j + 1).is_none_or(|n| !n.is_alphanumeric())) {
                        end = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = end {
                    push_text(&mut buf, &mut out);
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push(Inline::Italic(inline(&inner)));
                    i = end + 1;
                    continue;
                }
            }
        }
        // [1] Verweis oder [Text](https://…)
        if c == '[' {
            if let Some(close) = find(&chars, i + 1, &[']']) {
                let inner: String = chars[i + 1..close].iter().collect();
                if let Some(nums) = citation(&inner) {
                    push_text(&mut buf, &mut out);
                    out.push(Inline::Cite(nums));
                    i = close + 1;
                    continue;
                }
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = find(&chars, close + 2, &[')']) {
                        let url: String = chars[close + 2..end].iter().collect();
                        if safe_url(&url) {
                            push_text(&mut buf, &mut out);
                            out.push(Inline::Link(inline(&inner), url.trim().to_string()));
                            i = end + 1;
                            continue;
                        }
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    push_text(&mut buf, &mut out);
    out
}

// ------------------------------------------------------------------ Darstellung

fn render_inline(parts: &[Inline], sources: &[i64]) -> Element {
    rsx! {
        for p in parts.iter() {
            match p {
                Inline::Text(t) => rsx! { "{t}" },
                Inline::Bold(inner) => rsx! { strong { class: "font-semibold text-foreground", {render_inline(inner, sources)} } },
                Inline::Italic(inner) => rsx! { em { {render_inline(inner, sources)} } },
                Inline::Code(c) => rsx! { code { "{c}" } },
                Inline::Link(inner, url) => rsx! {
                    a { href: "{url}", target: "_blank", rel: "noopener noreferrer", {render_inline(inner, sources)} }
                },
                Inline::Cite(nums) => rsx! {
                    span { class: "whitespace-nowrap",
                        for n in nums.iter().copied() {
                            match sources.get(n - 1).copied() {
                                Some(id) => rsx! {
                                    Link {
                                        
                                        to: Route::All { article: Some(id) },
                                        class: "mx-0.5 inline-grid h-[1.15rem] min-w-[1.15rem] place-items-center rounded-md bg-muted px-1 align-[0.12em] text-[0.68rem] font-semibold tabular-nums text-muted-foreground no-underline transition-colors hover:bg-primary hover:text-primary-foreground",
                                        "{n}"
                                    }
                                },
                                None => rsx! { span { class: "text-muted-foreground", "[{n}]" } },
                            }
                        }
                    }
                },
            }
        }
    }
}

/// Markdown als RSX. `sources` sind die Artikel-IDs der Quellen in Reihenfolge ([1] = erste).
pub fn render(text: &str, sources: &[i64]) -> Element {
    let blocks = parse(text);
    rsx! {
        for b in blocks.iter() {
            match b {
                Block::Heading(level, parts) => match level {
                    1 | 2 => rsx! { h2 { {render_inline(parts, sources)} } },
                    3 => rsx! { h3 { {render_inline(parts, sources)} } },
                    _ => rsx! { h4 { {render_inline(parts, sources)} } },
                },
                Block::Para(parts) => rsx! { p { {render_inline(parts, sources)} } },
                Block::Bullets(items) => rsx! {
                    ul { 
                        for it in items.iter() { li { {render_inline(it, sources)} } }
                    }
                },
                Block::Numbered(start, items) => rsx! {
                    ol { start: "{start}",
                        for it in items.iter() { li { {render_inline(it, sources)} } }
                    }
                },
                Block::Quote(parts) => rsx! { blockquote { {render_inline(parts, sources)} } },
                Block::Code(code) => rsx! { pre { code { "{code}" } } },
                Block::Rule => rsx! { hr {} },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn inline_marks() {
        assert_eq!(inline("a **b** c"), vec![t("a "), Inline::Bold(vec![t("b")]), t(" c")]);
        assert_eq!(inline("*kursiv* und `x_y`"), vec![Inline::Italic(vec![t("kursiv")]), t(" und "), Inline::Code("x_y".into())]);
        assert_eq!(inline("snake_case_name"), vec![t("snake_case_name")]);
        assert_eq!(inline("2 * 3 = 6"), vec![t("2 * 3 = 6")]);
        assert_eq!(inline("offen **fett"), vec![t("offen **fett")]);
    }

    #[test]
    fn citations_and_links() {
        assert_eq!(inline("Neu [1][2]."), vec![t("Neu "), Inline::Cite(vec![1]), Inline::Cite(vec![2]), t(".")]);
        assert_eq!(inline("x [1, 3]"), vec![t("x "), Inline::Cite(vec![1, 3])]);
        assert_eq!(inline("[Rust](https://rust-lang.org)"), vec![Inline::Link(vec![t("Rust")], "https://rust-lang.org".into())]);
        // Gefährliche Links bleiben Text
        assert_eq!(inline("[x](javascript:alert(1))"), vec![t("[x](javascript:alert(1))")]);
        assert_eq!(inline("[nur Text]"), vec![t("[nur Text]")]);
    }

    #[test]
    fn blocks() {
        let b = parse("## Thema\n\nErster Satz\nzweiter Satz.\n\n- eins\n- zwei\n  weiter\n1. a\n2. b\n\n> Zitat\n\n---");
        assert_eq!(b[0], Block::Heading(2, vec![t("Thema")]));
        assert_eq!(b[1], Block::Para(vec![t("Erster Satz zweiter Satz.")]));
        assert_eq!(b[2], Block::Bullets(vec![vec![t("eins")], vec![t("zwei weiter")]]));
        assert_eq!(b[3], Block::Numbered(1, vec![vec![t("a")], vec![t("b")]]));
        assert_eq!(b[4], Block::Quote(vec![t("Zitat")]));
        assert_eq!(b[5], Block::Rule);
        assert_eq!(parse("```\nfn main() {}\n```"), vec![Block::Code("fn main() {}".into())]);
    }
}

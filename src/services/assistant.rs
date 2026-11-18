//! KI-Funktionen über den Artikeln eines Benutzers:
//!
//! * **Zusammenfassung** eines Artikels: 3–6 Stichpunkte und ein Fazit in einem Satz. Wird am
//!   Artikel gespeichert (`ai_summary`), damit ein zweites Öffnen nichts kostet.
//! * **Überblick** (Briefing): die neuen Artikel der letzten Stunden, nach Themen gruppiert,
//!   jede Aussage mit Verweis `[n]` auf den Artikel.
//! * **Dieter**, der Assistent (wie in nrzPaper, ohne Vektor-Datenbank):
//!   1. Die KI macht aus der Frage (und dem Verlauf) Suchbegriffe.
//!   2. Die Volltextsuche findet passende Artikel; aus jedem werden die Textstellen um die
//!      Treffer genommen.
//!   3. Die KI antwortet nur anhand dieser Auszüge und verweist mit `[n]` auf die Artikel –
//!      die Oberfläche verlinkt sie.
//!
//! Verweise werden nach der Antwort in der Reihenfolge ihres ersten Auftretens neu nummeriert,
//! damit `[1]` immer die erste Quelle unter dem Text ist.

use crate::ai::{self, AiConfig};
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::services::articles::{fts_query, fts_query_any};
use nrznews_types::{AiSummary, AssistantAnswer, Briefing, BriefingInput, ChatMessage, Source};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// So viel Artikeltext geht höchstens in eine Zusammenfassung.
const SUMMARY_CHARS: usize = 12_000;
/// Überblick: höchstens so viele (neueste) Artikel, je so viele Zeichen Text.
const BRIEFING_MAX: i64 = 80;
const BRIEFING_CHARS: usize = 400;
/// Dieter: höchstens so viele Artikel, so viel Text insgesamt.
const MAX_ARTICLES: usize = 25;
const BUDGET: usize = 30_000;

fn date(ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(ts)
        .map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day()))
        .unwrap_or_default()
}

fn ensure_ready(ai: &AiConfig) -> AppResult<()> {
    match ai.missing() {
        Some(why) => Err(AppError::BadRequest(why)),
        None => Ok(()),
    }
}

/// Antwort der KI aufräumen: Codeblock-Zäune weg, Leerraum am Rand weg.
fn tidy(text: &str) -> String {
    let t = text.trim();
    let t = t
        .strip_prefix("```markdown")
        .or_else(|| t.strip_prefix("```md"))
        .or_else(|| t.strip_prefix("```"))
        .map(|r| r.trim_end().trim_end_matches("```"))
        .unwrap_or(t);
    t.trim().to_string()
}

// ------------------------------------------------------------------ Zusammenfassung

/// Zusammenfassung eines Artikels – gespeichert, ausser `refresh` verlangt eine neue.
pub async fn summary(
    pool: &SqlitePool,
    ai: &AiConfig,
    user_id: i64,
    article_id: i64,
    refresh: bool,
) -> AppResult<AiSummary> {
    /// Titel, Text, Feed, gespeicherte Zusammenfassung und deren Modell
    type Row = (String, String, String, Option<String>, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT a.title, a.content_text,
                CASE WHEN f.title <> '' THEN f.title ELSE f.original_title END,
                a.ai_summary, a.ai_summary_model
         FROM articles a JOIN feeds f ON f.id = a.feed_id
         WHERE a.id = ? AND a.user_id = ?",
    )
    .bind(article_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let (title, text, feed, cached, cached_model) = row.ok_or(AppError::NotFound)?;
    if let Some(s) = cached.filter(|s| !s.trim().is_empty() && !refresh) {
        return Ok(AiSummary {
            summary: s,
            model: cached_model.unwrap_or_default(),
            cached: true,
        });
    }
    ensure_ready(ai)?;
    if text.split_whitespace().count() < 5 {
        return Err(AppError::bad(
            "Der Artikel hat zu wenig Text für eine Zusammenfassung – öffne ihn im Original.",
        ));
    }
    let lang = ai::language_instruction(&ai.language, "in der Sprache des Artikels");
    let system = format!(
        "Du fasst Artikel aus Feeds für einen Leser zusammen. Schreibe {lang}:\n\
         - 3 bis 6 knappe Stichpunkte (Markdown, jeder beginnt mit „- “) mit den wichtigsten \
           Fakten, Zahlen und Namen;\n\
         - danach eine Leerzeile und ein Fazit in einem einzigen Satz, eingeleitet mit dem \
           fettgedruckten Wort für „Fazit“ in dieser Sprache (z. B. **Fazit:** oder **Takeaway:**).\n\
         Nur was im Artikel steht – nichts erfinden, keine Einleitung, keine Überschrift."
    );
    let body: String = text.chars().take(SUMMARY_CHARS).collect();
    let prompt = format!("Titel: {title}\nQuelle: {feed}\n\n{body}");
    let (answer, model) = ai::complete(ai, &system, &prompt)
        .await
        .map_err(AppError::BadRequest)?;
    let summary = tidy(&answer);
    sqlx::query(
        "UPDATE articles SET ai_summary = ?, ai_summary_model = ? WHERE id = ? AND user_id = ?",
    )
    .bind(&summary)
    .bind(&model)
    .bind(article_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(AiSummary {
        summary,
        model,
        cached: false,
    })
}

// ------------------------------------------------------------------ Verweise

/// Ein Artikel als mögliche Quelle.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Candidate {
    id: i64,
    title: String,
    feed_title: String,
    published_at: i64,
    text: String,
}

impl Candidate {
    fn source(&self) -> Source {
        Source {
            article_id: self.id,
            title: self.title.clone(),
            feed_title: self.feed_title.clone(),
            published_at: self.published_at,
        }
    }
}

/// Verweise `[3]`, `[2, 5]`, `[2][5]` in der Reihenfolge ihres Auftretens neu nummerieren
/// (1, 2, …) und die zugehörigen Quellen liefern. Verweise ausserhalb der Liste fallen weg.
fn renumber(text: &str, candidates: &[Candidate]) -> (String, Vec<Source>) {
    static CITE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([ \t]*)((?:\[\s*\d+(?:\s*[,;]\s*\d+)*\s*\])+)").expect("Regex")
    });
    let mut order: Vec<usize> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    let replaced = CITE.replace_all(text, |caps: &regex::Captures| {
        let mut out = String::new();
        // Aufeinanderfolgende Verweise ([2][5], [2, 5]) gemeinsam behandeln
        let numbers = caps[2]
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|n| n.parse::<usize>().ok());
        for n in numbers {
            if n == 0 || n > candidates.len() {
                continue;
            }
            let new = *index.entry(n).or_insert_with(|| {
                order.push(n);
                order.len()
            });
            out.push_str(&format!("[{new}]"));
        }
        // Ungültige Verweise samt Leerzeichen davor entfernen („auch [7].“ → „auch.“)
        if out.is_empty() {
            out
        } else {
            format!("{}{out}", &caps[1])
        }
    });
    let sources = order.iter().map(|n| candidates[n - 1].source()).collect();
    (replaced.trim().to_string(), sources)
}

// ------------------------------------------------------------------ Überblick

/// KI-Überblick über die (ungelesenen) Artikel der letzten Stunden. `user_language` gilt,
/// wenn die Sprache auf „auto“ steht (Artikel in mehreren Sprachen).
pub async fn briefing(
    pool: &SqlitePool,
    ai: &AiConfig,
    user_id: i64,
    user_language: &str,
    input: &BriefingInput,
) -> AppResult<Briefing> {
    ensure_ready(ai)?;
    let hours = input.hours.unwrap_or(24).clamp(1, 168);
    let unread_only = input.unread_only.unwrap_or(true);
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT a.id, a.title,
                CASE WHEN f.title <> '' THEN f.title ELSE f.original_title END AS feed_title,
                a.published_at, substr(a.content_text, 1, ",
    );
    qb.push(BRIEFING_CHARS as i64 + 50)
        .push(") AS text FROM articles a JOIN feeds f ON f.id = a.feed_id WHERE a.user_id = ")
        .push_bind(user_id)
        .push(" AND a.published_at >= ")
        .push_bind(now() - i64::from(hours) * 3600);
    if unread_only {
        qb.push(" AND a.read = 0");
    }
    if let Some(feed) = input.feed_id {
        qb.push(" AND a.feed_id = ").push_bind(feed);
    }
    if let Some(folder) = input.folder_id {
        qb.push(" AND f.folder_id = ").push_bind(folder);
    }
    qb.push(" ORDER BY a.published_at DESC, a.id DESC LIMIT ")
        .push_bind(BRIEFING_MAX);
    let articles: Vec<Candidate> = qb.build_query_as().fetch_all(pool).await?;
    let generated_at = now();
    if articles.is_empty() {
        return Ok(Briefing {
            text: format!(
                "In den letzten {hours} Stunden sind keine {}Artikel eingetroffen.",
                if unread_only { "ungelesenen " } else { "" }
            ),
            sources: Vec::new(),
            model: String::new(),
            generated_at,
            article_count: 0,
        });
    }
    let fallback = if user_language == "en" { "en" } else { "de" };
    let lang_code = if ai.language == "auto" || ai.language.is_empty() {
        fallback
    } else {
        ai.language.as_str()
    };
    let lang = ai::language_instruction(lang_code, "auf Deutsch");
    let system = format!(
        "Du bist Redakteur und schreibst einen kurzen Überblick über neue Artikel aus den \
         Feeds eines Lesers. Schreibe {lang} in Markdown:\n\
         - Gruppiere nach Themen: je Thema eine Überschrift „## Thema“ und darunter kurze \
           Stichpunkte („- “), Wichtiges zuerst.\n\
         - Belege jeden Stichpunkt mit der Nummer des Artikels in eckigen Klammern, z. B. [3] \
           oder [2][5]. Ähnliche Artikel fasse zusammen.\n\
         - Unwichtiges (Werbung, Gewinnspiele, Stellenanzeigen) weglassen.\n\
         - Nur was in den Artikeln steht – nichts erfinden. Keine Einleitung, kein Schlusswort."
    );
    let mut prompt = format!("{} Artikel der letzten {hours} Stunden:\n", articles.len());
    for (i, a) in articles.iter().enumerate() {
        let text = crate::feeds::sanitize::excerpt(&a.text, BRIEFING_CHARS);
        prompt.push_str(&format!(
            "\n[{}] {} — {} ({})\n{}\n",
            i + 1,
            a.title,
            a.feed_title,
            date(a.published_at),
            text
        ));
    }
    let (answer, model) = ai::complete(ai, &system, &prompt)
        .await
        .map_err(AppError::BadRequest)?;
    let (text, sources) = renumber(&tidy(&answer), &articles);
    Ok(Briefing {
        text,
        sources,
        model,
        generated_at,
        article_count: articles.len() as u32,
    })
}

// ------------------------------------------------------------------ Dieter

fn transcript(messages: &[ChatMessage]) -> String {
    let start = messages.len().saturating_sub(8);
    messages[start..]
        .iter()
        .map(|m| {
            let who = if m.role == "assistant" {
                "Dieter"
            } else {
                "Nutzer"
            };
            format!(
                "{who}: {}",
                m.content.chars().take(2000).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Grober Wortstamm: übliche Endungen ab (Wahlen → Wahl, Regierungen → Regierung), damit die
/// Präfixsuche auch andere Formen findet. Kurze Wörter bleiben, wie sie sind.
pub fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    if w.chars().count() < 6 {
        return w;
    }
    for end in ["ungen", "en", "er", "es", "e", "n", "s"] {
        if let Some(s) = w.strip_suffix(end) {
            if s.chars().count() >= 4 {
                return if end == "ungen" {
                    format!("{s}ung")
                } else {
                    s.to_string()
                };
            }
        }
    }
    w
}

/// Wörter der Suchanfragen (für Textstellen und die breitere Suche), samt Wortstämmen.
fn words(queries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = queries
        .iter()
        .flat_map(|q| q.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().count() >= 3)
        .flat_map(|w| [w.to_lowercase(), stem(w)])
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Textstellen um die Treffer (je ±300 Zeichen), zusammengeführt, höchstens `budget` Zeichen.
/// Ohne Treffer: der Anfang des Textes.
pub fn snippets(content: &str, words: &[String], budget: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= budget {
        return content.to_string();
    }
    let lower: Vec<char> = content.to_lowercase().chars().collect();
    // Kleinbuchstaben können länger werden (selten) – dann nur den Anfang nehmen
    if lower.len() != chars.len() {
        return chars[..budget].iter().collect();
    }
    let radius = 300usize;
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let wc: Vec<char> = w.chars().collect();
        if wc.is_empty() {
            continue;
        }
        let mut i = 0;
        while i + wc.len() <= lower.len() {
            if lower[i..i + wc.len()] == wc[..] {
                ranges.push((
                    i.saturating_sub(radius),
                    (i + wc.len() + radius).min(chars.len()),
                ));
                i += wc.len() + radius;
            } else {
                i += 1;
            }
        }
    }
    if ranges.is_empty() {
        return chars[..budget].iter().collect();
    }
    ranges.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }
    let mut out = String::new();
    let mut used = 0usize;
    for (a, b) in merged {
        if used >= budget {
            break;
        }
        if !out.is_empty() {
            out.push_str(" … ");
        }
        let take = (b - a).min(budget - used);
        out.extend(chars[a..a + take].iter());
        used += take;
    }
    out
}

/// Schritt 1: Suchbegriffe. Fällt die KI aus, gelten die Wörter der Frage.
async fn queries(ai: &AiConfig, messages: &[ChatMessage], question: &str) -> Vec<String> {
    let system = "Du hilfst, in den Artikeln aus den Feeds (Nachrichten, Blogs, Podcasts) eines \
        Lesers zu suchen. Erzeuge aus der letzten Frage des Nutzers (mit dem Verlauf als Kontext) \
        1 bis 5 kurze Suchanfragen für eine Volltextsuche: je 1–3 Stichwörter, auch Synonyme, \
        Namen und die Übersetzung ins Englische bzw. Deutsche. Keine Füllwörter. \
        Antworte nur mit JSON: {\"queries\": [\"…\"]}";
    let fallback = || {
        vec![question
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")]
    };
    match ai::complete(ai, system, &transcript(messages)).await {
        Ok((text, _)) => {
            let qs: Vec<String> = ai::extract_json(&text)
                .and_then(|v| v["queries"].as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|q| {
                    q.as_str()
                        .map(|s| s.trim().chars().take(80).collect::<String>())
                })
                .filter(|q| !q.is_empty())
                .take(5)
                .collect();
            if qs.is_empty() {
                fallback()
            } else {
                qs
            }
        }
        Err(_) => fallback(),
    }
}

/// Volltextsuche über die Artikel des Benutzers, nach Relevanz (bm25).
async fn search(
    pool: &SqlitePool,
    user_id: i64,
    matcher: &str,
    limit: i64,
) -> AppResult<Vec<Candidate>> {
    Ok(sqlx::query_as(
        "SELECT a.id, a.title,
                CASE WHEN f.title <> '' THEN f.title ELSE f.original_title END AS feed_title,
                a.published_at, a.content_text AS text
         FROM articles_fts
         JOIN articles a ON a.id = articles_fts.rowid
         JOIN feeds f ON f.id = a.feed_id
         WHERE articles_fts MATCH ? AND a.user_id = ?
         ORDER BY rank LIMIT ?",
    )
    .bind(matcher)
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// Dieter beantwortet eine Frage zu den Artikeln des Benutzers – mit Quellen.
pub async fn ask(
    pool: &SqlitePool,
    ai: &AiConfig,
    user_id: i64,
    messages: &[ChatMessage],
) -> AppResult<AssistantAnswer> {
    ensure_ready(ai)?;
    let question = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.trim().to_string())
        .unwrap_or_default();
    if question.is_empty() {
        return Err(AppError::bad("Bitte eine Frage stellen."));
    }
    if question.chars().count() > 2000 {
        return Err(AppError::bad(
            "Die Frage ist zu lang (höchstens 2000 Zeichen).",
        ));
    }

    // 1 + 2: suchen – erst alle Wörter einer Anfrage, dann jedes Wort (und sein Stamm) einzeln
    let qs = queries(ai, messages, &question).await;
    let ws = words(&qs);
    let mut seen = HashSet::new();
    let mut found: Vec<Candidate> = Vec::new();
    let mut matchers: Vec<String> = qs.iter().filter_map(|q| fts_query(q)).collect();
    matchers.extend(fts_query_any(&ws.join(" ")));
    for m in &matchers {
        if found.len() >= MAX_ARTICLES {
            break;
        }
        for c in search(pool, user_id, m, 15).await? {
            if found.len() < MAX_ARTICLES && seen.insert(c.id) {
                found.push(c);
            }
        }
    }
    // Nichts gefunden („Was gibt es Neues?“): die neuesten Artikel als Grundlage
    if found.is_empty() {
        found = sqlx::query_as(
            "SELECT a.id, a.title,
                    CASE WHEN f.title <> '' THEN f.title ELSE f.original_title END AS feed_title,
                    a.published_at, substr(a.content_text, 1, 1500) AS text
             FROM articles a JOIN feeds f ON f.id = a.feed_id
             WHERE a.user_id = ? ORDER BY a.published_at DESC, a.id DESC LIMIT 15",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    }

    // Auszüge: Budget je Artikel, Textstellen um die Treffer
    let per_article = if found.is_empty() {
        0
    } else {
        (BUDGET / found.len()).clamp(400, 4000)
    };
    let mut context = String::new();
    for (i, a) in found.iter().enumerate() {
        context.push_str(&format!(
            "\n[{}] {} — {} ({})\n{}\n",
            i + 1,
            a.title,
            a.feed_title,
            date(a.published_at),
            snippets(&a.text, &ws, per_article)
        ));
    }

    // 3: antworten
    let system = "Du bist Dieter, der freundliche Assistent von nrzNews, einem RSS-Reader. \
        Du beantwortest Fragen ausschliesslich anhand der Artikel unten aus den Feeds des Nutzers – \
        nichts erfinden. Steht die Antwort nicht darin, sag das ehrlich und schlage vor, wonach man \
        suchen könnte. Antworte in der Sprache der Frage, knapp, konkret und freundlich \
        (Markdown erlaubt, Listen mit „- “). Belege Aussagen mit der Nummer des Artikels in eckigen \
        Klammern, z. B. [2] oder [1][4] – nur Nummern aus der Liste unten.";
    let prompt = format!(
        "Heute: {}\n\nVerlauf:\n{}\n\nArtikel ({} gefunden):\n{}",
        date(now()),
        transcript(messages),
        found.len(),
        if found.is_empty() {
            "(keine Artikel)".to_string()
        } else {
            context
        }
    );
    let (text, model) = ai::complete(ai, system, &prompt)
        .await
        .map_err(AppError::BadRequest)?;
    let (answer, sources) = renumber(&tidy(&text), &found);
    Ok(AssistantAnswer {
        answer: if answer.is_empty() {
            "Dazu habe ich in deinen Artikeln nichts gefunden.".into()
        } else {
            answer
        },
        sources,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64) -> Candidate {
        Candidate {
            id,
            title: format!("T{id}"),
            feed_title: "F".into(),
            published_at: 0,
            text: String::new(),
        }
    }

    #[test]
    fn citations_are_renumbered() {
        let c: Vec<Candidate> = (1..=5).map(cand).collect();
        let (text, sources) = renumber("A [3]. B [5, 3]. C [9][1].", &c);
        assert_eq!(text, "A [1]. B [2][1]. C [3].");
        let ids: Vec<i64> = sources.iter().map(|s| s.article_id).collect();
        assert_eq!(ids, vec![3, 5, 1]);
    }

    #[test]
    fn stems() {
        assert_eq!(stem("Regierungen"), "regierung");
        assert_eq!(stem("Wahlen"), "wahl");
        assert_eq!(stem("Bank"), "bank");
    }

    #[test]
    fn snippets_around_matches() {
        let text = format!(
            "{}Bundesrat beschliesst Gesetz{}",
            "x".repeat(5000),
            "y".repeat(5000)
        );
        let s = snippets(&text, &["bundesrat".into()], 800);
        assert!(s.contains("Bundesrat beschliesst"));
        assert!(s.chars().count() <= 800);
        assert_eq!(snippets("kurz", &["x".into()], 100), "kurz");
    }

    #[test]
    fn tidy_fences() {
        assert_eq!(tidy("```markdown\n- a\n```"), "- a");
        assert_eq!(tidy("  - b  "), "- b");
    }
}

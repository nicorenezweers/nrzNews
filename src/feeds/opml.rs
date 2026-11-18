//! OPML: die Abo-Liste, die jeder Feedreader versteht. Beim Export wird jeder Ordner ein
//! `<outline>` mit seinen Feeds darin; beim Import gilt der Titel eines Outlines der obersten
//! Ebene als Ordner – tiefere Verschachtelung landet im selben Ordner (nrzNews kennt nur
//! eine Ebene).

/// Ein Feed aus einer OPML-Datei bzw. für den Export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlFeed {
    pub title: String,
    pub xml_url: String,
    pub html_url: String,
    /// Ordner (leer = ohne Ordner)
    pub folder: String,
}

/// Liest die Feeds einer OPML-Datei.
pub fn parse(text: &str) -> Result<Vec<OpmlFeed>, String> {
    // Manche Programme schreiben vor die Deklaration ein BOM oder Leerzeilen
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err("Die OPML-Datei ist leer.".into());
    }
    let doc = opml::OPML::from_str(text)
        .map_err(|e| format!("Die OPML-Datei liess sich nicht lesen ({e})."))?;
    let mut out = Vec::new();
    for top in &doc.body.outlines {
        if top.xml_url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            push(&mut out, top, "");
        } else {
            let folder = label(top);
            collect(&mut out, &top.outlines, &folder);
        }
    }
    Ok(out)
}

fn collect(out: &mut Vec<OpmlFeed>, outlines: &[opml::Outline], folder: &str) {
    for o in outlines {
        if o.xml_url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            push(out, o, folder);
        }
        collect(out, &o.outlines, folder);
    }
}

fn push(out: &mut Vec<OpmlFeed>, o: &opml::Outline, folder: &str) {
    let xml_url = o.xml_url.as_deref().unwrap_or_default().trim().to_string();
    if out.iter().any(|f| f.xml_url == xml_url) {
        return;
    }
    out.push(OpmlFeed {
        title: label(o),
        xml_url,
        html_url: o.html_url.as_deref().unwrap_or_default().trim().to_string(),
        folder: folder.to_string(),
    });
}

/// Angezeigter Name: `title`, sonst `text`.
fn label(o: &opml::Outline) -> String {
    let t = o.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
    super::sanitize::collapse(t.unwrap_or(o.text.trim()))
}

/// OPML 2.0 mit allen Feeds; Feeds ohne Ordner stehen auf der obersten Ebene.
pub fn export(feeds: &[OpmlFeed], owner: &str) -> Result<String, String> {
    let mut doc = opml::OPML::default();
    let now = time::OffsetDateTime::now_utc();
    doc.head = Some(opml::Head {
        title: Some("nrzNews-Abos".into()),
        date_created: now
            .format(&time::format_description::well_known::Rfc2822)
            .ok(),
        owner_name: (!owner.is_empty()).then(|| owner.to_string()),
        ..opml::Head::default()
    });
    let outline = |f: &OpmlFeed| opml::Outline {
        text: f.title.clone(),
        title: Some(f.title.clone()),
        r#type: Some("rss".into()),
        xml_url: Some(f.xml_url.clone()),
        html_url: (!f.html_url.is_empty()).then(|| f.html_url.clone()),
        ..opml::Outline::default()
    };
    let mut folders: Vec<opml::Outline> = Vec::new();
    for f in feeds.iter().filter(|f| !f.folder.is_empty()) {
        match folders.iter_mut().find(|o| o.text == f.folder) {
            Some(folder) => folder.outlines.push(outline(f)),
            None => folders.push(opml::Outline {
                text: f.folder.clone(),
                title: Some(f.folder.clone()),
                outlines: vec![outline(f)],
                ..opml::Outline::default()
            }),
        }
    }
    doc.body.outlines = folders;
    doc.body
        .outlines
        .extend(feeds.iter().filter(|f| f.folder.is_empty()).map(outline));
    let xml = doc
        .to_string()
        .map_err(|e| format!("OPML konnte nicht geschrieben werden: {e}"))?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{xml}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let feeds = vec![
            OpmlFeed {
                title: "Rust & Co".into(),
                xml_url: "https://blog.rust-lang.org/feed.xml".into(),
                html_url: "https://blog.rust-lang.org/".into(),
                folder: "Technik".into(),
            },
            OpmlFeed {
                title: "Ohne Ordner".into(),
                xml_url: "https://example.org/feed".into(),
                html_url: String::new(),
                folder: String::new(),
            },
        ];
        let xml = export(&feeds, "alice").unwrap();
        assert!(xml.starts_with("<?xml"));
        assert_eq!(parse(&xml).unwrap(), feeds);
    }
}

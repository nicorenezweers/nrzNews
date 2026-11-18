//! Artikelliste (mittlere Spalte) für alle Ansichten – alle, heute, gemerkt, Feed, Ordner,
//! Suche – und rechts daneben der Lesemodus. Auf dem Handy jeweils nur eine Spalte.
//! Der geöffnete Artikel steht als `?article=` in der Adresse.
//!
//! Tastatur: j/k blättern, o/Enter Original, s merken, m gelesen, r abrufen, / Suche,
//! Esc schliessen, Shift+A alle als gelesen, ? Übersicht.

use crate::api;
use crate::components::common::*;
use crate::components::feed_dialog::{FeedDialog, FolderDialog, delete_folder, unsubscribe};
use crate::components::feed_icon::FeedIcon;
use crate::components::overlay::*;
use crate::components::shortcuts::SHORTCUTS_OPEN;
use crate::i18n::{tr, trf};
use crate::pages::reader::{Reader, ReaderEmpty};
use crate::state::{self, ADD_FEED_OPEN, LIST_BASELINE, OVERVIEW, REFRESH, UNREAD_ONLY};
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::*;

/// Artikel je Seite (weitere per „Mehr laden“ bzw. beim Scrollen).
const PAGE: u32 = 40;

/// Der Lesemodus wurde aus der Liste geöffnet (neuer Eintrag im Verlauf) – Schliessen geht
/// dann zurück, statt einen weiteren Eintrag anzulegen.
static FROM_LIST: GlobalSignal<bool> = Signal::global(|| false);

/// Lesemodus schliessen: zurück zur Liste.
pub fn close_reader(sel: &Sel) {
    let nav = navigator();
    if *FROM_LIST.peek() {
        *FROM_LIST.write() = false;
        nav.go_back();
    } else {
        nav.replace(sel.route(None));
    }
}

// ------------------------------------------------------------------ Auswahl

/// Was die Liste zeigt.
#[derive(Clone, PartialEq, Debug)]
pub enum Sel {
    All,
    Today,
    Starred,
    Feed(i64),
    Folder(i64),
    Search(String),
}

impl Sel {
    /// Adresse dieser Auswahl, optional mit geöffnetem Artikel.
    pub fn route(&self, article: Option<i64>) -> Route {
        match self {
            Sel::All => Route::All { article },
            Sel::Today => Route::Today { article },
            Sel::Starred => Route::Starred { article },
            Sel::Feed(id) => Route::FeedView { id: *id, article },
            Sel::Folder(id) => Route::FolderView { id: *id, article },
            Sel::Search(q) => Route::Search { article, q: q.clone() },
        }
    }

    /// Schlüssel für Zähler und Neuladen (`all`, `today`, `feed:3`, …).
    pub fn key(&self) -> String {
        match self {
            Sel::All => "all".into(),
            Sel::Today => "today".into(),
            Sel::Starred => "starred".into(),
            Sel::Feed(id) => format!("feed:{id}"),
            Sel::Folder(id) => format!("folder:{id}"),
            Sel::Search(q) => format!("search:{q}"),
        }
    }

    fn url(&self, unread_only: bool, cursor: Option<&str>) -> String {
        let view = match self {
            Sel::Today => View::Today,
            Sel::Starred => View::Starred,
            _ if unread_only => View::Unread,
            _ => View::All,
        };
        let mut q = vec![format!("view={}", view.as_str()), format!("limit={PAGE}")];
        match self {
            Sel::Feed(id) => q.push(format!("feed={id}")),
            Sel::Folder(id) => q.push(format!("folder={id}")),
            Sel::Search(s) => q.push(format!("q={}", api::enc(s))),
            _ => {}
        }
        if let Some(c) = cursor {
            q.push(format!("cursor={}", api::enc(c)));
        }
        format!("/articles?{}", q.join("&"))
    }

    /// „Nur ungelesene“ lässt sich umschalten (bei „Gemerkt“ nicht).
    pub fn has_unread_toggle(&self) -> bool {
        !matches!(self, Sel::Starred)
    }

    /// „Heute“ liefert der Server ganz (letzte 24 h) – Ungelesene filtert die Oberfläche.
    fn filters_read(&self, unread_only: bool) -> bool {
        matches!(self, Sel::Today) && unread_only
    }

    /// „Alle als gelesen“ für diese Auswahl (bei der Suche nicht möglich).
    fn mark_input(&self, before: Option<i64>) -> Option<MarkReadInput> {
        let base = MarkReadInput { before, ..Default::default() };
        match self {
            Sel::All => Some(base),
            Sel::Today => Some(MarkReadInput { view: Some(View::Today), ..base }),
            Sel::Starred => Some(MarkReadInput { view: Some(View::Starred), ..base }),
            Sel::Feed(id) => Some(MarkReadInput { feed_id: Some(*id), ..base }),
            Sel::Folder(id) => Some(MarkReadInput { folder_id: Some(*id), ..base }),
            Sel::Search(_) => None,
        }
    }

    pub fn title(&self, ov: Option<&Overview>) -> String {
        match self {
            Sel::All => tr("Alle Artikel"),
            Sel::Today => tr("Heute"),
            Sel::Starred => tr("Gemerkt"),
            Sel::Feed(id) => ov.and_then(|o| o.feeds.iter().find(|f| f.id == *id)).map(|f| f.title.clone()).unwrap_or_else(|| tr("Feed")),
            Sel::Folder(id) => ov.and_then(|o| o.folders.iter().find(|f| f.id == *id)).map(|f| f.name.clone()).unwrap_or_else(|| tr("Ordner")),
            Sel::Search(q) => trf("Suche: {0}", &[q]),
        }
    }
}

// ------------------------------------------------------------------ Zustand der Spalten

/// Liste und geöffneter Artikel – geteilt zwischen Liste, Lesemodus und Tastatur.
#[derive(Clone, Copy, PartialEq)]
pub struct Pane {
    pub items: Signal<Vec<ArticleSummary>>,
    pub cursor: Signal<Option<String>>,
    pub loading: Signal<bool>,
    pub ready: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub generation: Signal<u64>,
    pub refreshing: Signal<bool>,
    /// Im Lesemodus geöffneter Artikel (ganzer Inhalt)
    pub open: Signal<Option<Article>>,
}

/// Was es zum Umschalten von gelesen/gemerkt braucht (inkl. Zähler).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ArticleRef {
    pub id: i64,
    pub feed_id: i64,
    pub published_at: i64,
    pub read: bool,
    pub starred: bool,
}

impl Pane {
    /// Stand eines Artikels: aus der Liste, sonst aus dem Lesemodus.
    pub fn get(&self, id: i64) -> Option<ArticleRef> {
        if let Some(a) = self.items.try_peek().ok()?.iter().find(|a| a.id == id) {
            return Some(ArticleRef { id, feed_id: a.feed_id, published_at: a.published_at, read: a.read, starred: a.starred });
        }
        self.open
            .try_peek()
            .ok()?
            .as_ref()
            .filter(|a| a.id == id)
            .map(|a| ArticleRef { id, feed_id: a.feed_id, published_at: a.published_at, read: a.read, starred: a.starred })
    }

    /// Link zum Original.
    pub fn url_of(&self, id: i64) -> Option<String> {
        if let Some(a) = self.items.peek().iter().find(|a| a.id == id) {
            return Some(a.url.clone());
        }
        self.open.peek().as_ref().filter(|a| a.id == id).map(|a| a.url.clone())
    }

    /// Neuen Stand in Liste und Lesemodus eintragen (still, falls die Spalten schon weg sind).
    fn apply(&self, id: i64, read: bool, starred: bool) {
        let mut items = self.items;
        if let Ok(mut list) = items.try_write() {
            for a in list.iter_mut().filter(|a| a.id == id) {
                a.read = read;
                a.starred = starred;
            }
        }
        let mut open = self.open;
        if let Ok(mut o) = open.try_write() {
            if let Some(a) = o.as_mut().filter(|a| a.id == id) {
                a.read = read;
                a.starred = starred;
            }
        }
    }
}

/// Gelesen/gemerkt setzen: sofort in der Oberfläche (samt Zählern), dann beim Server;
/// schlägt das fehl, wird alles zurückgenommen.
pub fn set_state(pane: Option<Pane>, r: ArticleRef, read: Option<bool>, starred: Option<bool>) {
    let new_read = read.unwrap_or(r.read);
    let new_starred = starred.unwrap_or(r.starred);
    if new_read == r.read && new_starred == r.starred {
        return;
    }
    if let Some(p) = pane {
        p.apply(r.id, new_read, new_starred);
    }
    let unread_delta = r.read as i64 - new_read as i64;
    let starred_delta = new_starred as i64 - r.starred as i64;
    state::adjust_counts(r.feed_id, r.published_at, unread_delta, starred_delta);
    let input = ArticleStateInput { read: (new_read != r.read).then_some(new_read), starred: (new_starred != r.starred).then_some(new_starred) };
    dioxus::core::spawn_forever(async move {
        match api::put::<ArticleState, _>(&format!("/articles/{}", r.id), &input).await {
            Ok(_) => state::sync_soon(),
            Err(e) => {
                if let Some(p) = pane {
                    p.apply(r.id, r.read, r.starred);
                }
                state::adjust_counts(r.feed_id, r.published_at, -unread_delta, -starred_delta);
                toast_error(e);
            }
        }
    });
}

/// Liste laden: neu (`more = false`) oder die nächste Seite anhängen.
fn load(pane: Pane, sel: Sel, unread_only: bool, more: bool, clear: bool) {
    let Pane { mut items, mut cursor, mut loading, mut ready, mut error, mut generation, .. } = pane;
    let next = if more {
        let c = cursor.peek().clone();
        if c.is_none() || *loading.peek() {
            return;
        }
        c
    } else {
        None
    };
    let current = if more {
        *generation.peek()
    } else {
        let g = *generation.peek() + 1;
        generation.set(g);
        g
    };
    if clear {
        items.set(Vec::new());
        ready.set(false);
        cursor.set(None);
    }
    loading.set(true);
    spawn(async move {
        let res = api::get::<ArticlePage>(&sel.url(unread_only, next.as_deref())).await;
        // Inzwischen etwas anderes gewählt → Antwort verwerfen
        if *generation.peek() != current {
            return;
        }
        loading.set(false);
        ready.set(true);
        match res {
            Ok(page) => {
                let mut list = page.items;
                if sel.filters_read(unread_only) {
                    list.retain(|a| !a.read);
                }
                if more {
                    let mut all = items.write();
                    for a in list {
                        if !all.iter().any(|x| x.id == a.id) {
                            all.push(a);
                        }
                    }
                } else {
                    items.set(list);
                    // Stand für „Neue Artikel“ merken (-1 = Überblick noch nicht da)
                    let key = sel.key();
                    let count = OVERVIEW.peek().as_ref().map(|o| state::unread_for_key(o, &key));
                    *LIST_BASELINE.write() = match count {
                        Some(Some(n)) => Some((key, n)),
                        Some(None) => None,
                        None => Some((key, -1)),
                    };
                }
                cursor.set(page.next_cursor);
                error.set(None);
            }
            Err(e) => error.set(Some(e.message)),
        }
    });
}

/// Feeds jetzt abrufen (ein Feed oder alle), danach Liste und Zähler neu.
fn refresh_feeds(pane: Pane, sel: Sel) {
    let mut refreshing = pane.refreshing;
    if *refreshing.peek() {
        return;
    }
    refreshing.set(true);
    spawn(async move {
        match &sel {
            Sel::Feed(id) => match api::action::<Feed>(&format!("/feeds/{id}/refresh")).await {
                Ok(f) => match f.last_error.filter(|e| !e.trim().is_empty()) {
                    Some(e) => toast_error(trf("Abruf fehlgeschlagen: {0}", &[&e])),
                    None => toast(tr("Feed abgerufen.")),
                },
                Err(e) => toast_error(e),
            },
            _ => match api::action::<RefreshResult>("/feeds/refresh").await {
                Ok(r) => {
                    let mut msg = match r.new_articles {
                        0 => tr("Keine neuen Artikel."),
                        1 => tr("1 neuer Artikel."),
                        n => trf("{0} neue Artikel.", &[&n]),
                    };
                    if r.errors == 1 {
                        msg = format!("{msg} {}", tr("1 Feed mit Fehlern."));
                    } else if r.errors > 1 {
                        msg = format!("{msg} {}", trf("{0} Feeds mit Fehlern.", &[&r.errors]));
                    }
                    toast(msg);
                }
                Err(e) => toast_error(e),
            },
        }
        refreshing.set(false);
        state::load_overview().await;
        state::refresh();
    });
}

/// Alle Artikel der Auswahl bis zum neuesten geladenen als gelesen markieren.
fn mark_all_read(pane: Pane, sel: Sel) {
    let before = pane.items.peek().iter().map(|a| a.published_at).max();
    let Some(input) = sel.mark_input(before) else { return };
    if before.is_none() {
        return;
    }
    let title = sel.title(OVERVIEW.peek().as_ref());
    spawn(async move {
        let ask = Ask::new(tr("Alle als gelesen markieren?"), trf("„{0}“: alle Artikel bis zum neuesten in der Liste.", &[&title])).confirm(tr("Als gelesen markieren"));
        if !confirm(ask).await {
            return;
        }
        let mut items = pane.items;
        let before = before.unwrap_or_default();
        let old: Vec<(i64, bool)> = items.peek().iter().map(|a| (a.id, a.read)).collect();
        for a in items.write().iter_mut().filter(|a| a.published_at <= before) {
            a.read = true;
        }
        match api::post::<MarkReadResult, _>("/articles/mark-read", &input).await {
            Ok(r) => {
                toast(if r.updated == 1 { tr("1 Artikel als gelesen markiert.") } else { trf("{0} Artikel als gelesen markiert.", &[&r.updated]) });
                state::load_overview().await;
                state::rebase();
            }
            Err(e) => {
                if let Ok(mut list) = items.try_write() {
                    for a in list.iter_mut() {
                        if let Some((_, read)) = old.iter().find(|(id, _)| *id == a.id) {
                            a.read = *read;
                        }
                    }
                }
                toast_error(e);
            }
        }
    });
}

// ------------------------------------------------------------------ Tastatur

const KEYS_JS: &str = r#"
if (window.__nrznewsKeys) window.removeEventListener('keydown', window.__nrznewsKeys);
window.__nrznewsKeys = (e) => {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target;
    if (t && t.closest && t.closest('input, textarea, select, [contenteditable], [role=dialog], [role=alertdialog]')) return;
    // Feststelltaste egal; Shift+a ergibt „A“ (auch bei Tastaturen, die „a“ melden)
    let k = e.key;
    if (k.length === 1) k = e.shiftKey ? (k === '/' ? '?' : k.toUpperCase()) : k.toLowerCase();
    const keys = ['j', 'k', 'n', 'p', 'o', 'v', 'Enter', 's', 'm', 'r', '/', 'Escape', '?', 'A'];
    if (!keys.includes(k)) return;
    if (k === 'Enter' && t && t.closest && t.closest('a, button')) return;
    e.preventDefault();
    dioxus.send(k);
};
window.addEventListener('keydown', window.__nrznewsKeys);
await new Promise(() => {});
"#;

/// Tastatur-Listener entfernen (ohne Dioxus – läuft beim Abbauen der Seite).
fn remove_key_listener() {
    use wasm_bindgen::JsCast;
    let Some(w) = web_sys::window() else { return };
    let key = wasm_bindgen::JsValue::from_str("__nrznewsKeys");
    if let Ok(f) = js_sys::Reflect::get(&w, &key) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = w.remove_event_listener_with_callback("keydown", f);
        }
        let _ = js_sys::Reflect::set(&w, &key, &wasm_bindgen::JsValue::NULL);
    }
}

/// Zum nächsten (`dir = 1`) oder vorherigen (`-1`) Artikel der Liste.
fn step(pane: Pane, sel: &Sel, article: Option<i64>, dir: i64) {
    let (id, near_end) = {
        let items = pane.items.peek();
        if items.is_empty() {
            return;
        }
        let pos = article.and_then(|id| items.iter().position(|a| a.id == id));
        let next = match pos {
            Some(i) => i as i64 + dir,
            None if dir > 0 => 0,
            None => return,
        };
        if next < 0 || next as usize >= items.len() {
            (None, true)
        } else {
            (Some(items[next as usize].id), next as usize + 4 >= items.len())
        }
    };
    if near_end && dir > 0 {
        load(pane, sel.clone(), state::unread_only(), true, false);
    }
    let Some(id) = id else { return };
    let nav = navigator();
    if article.is_some() {
        nav.replace(sel.route(Some(id)));
    } else {
        *FROM_LIST.write() = true;
        nav.push(sel.route(Some(id)));
    }
    util::scroll_into_view(&format!("row-{id}"));
}

fn on_key(key: &str, pane: Pane, sel: &Sel, article: Option<i64>) {
    match key {
        "j" | "n" => step(pane, sel, article, 1),
        "k" | "p" => step(pane, sel, article, -1),
        "o" | "v" | "Enter" => {
            if let Some(url) = article.and_then(|id| pane.url_of(id)) {
                util::open_tab(&url);
            }
        }
        "s" => {
            if let Some(r) = article.and_then(|id| pane.get(id)) {
                set_state(Some(pane), r, None, Some(!r.starred));
            }
        }
        "m" => {
            if let Some(r) = article.and_then(|id| pane.get(id)) {
                set_state(Some(pane), r, Some(!r.read), None);
            }
        }
        "r" => refresh_feeds(pane, sel.clone()),
        "/" => util::focus("search-input"),
        "Escape" => {
            if article.is_some() {
                close_reader(sel);
            }
        }
        "A" => mark_all_read(pane, sel.clone()),
        "?" => *SHORTCUTS_OPEN.write() = true,
        _ => {}
    }
}

/// Gesten vom Handy: Zeile wischen (gelesen/merken), Liste ziehen (abrufen), im Lesemodus
/// wischen (nächster/vorheriger Artikel).
fn on_gesture(g: &crate::gestures::Gesture, pane: Pane, sel: &Sel, article: Option<i64>) {
    match g.t.as_str() {
        "swipe" => {
            let Some(r) = g.id.and_then(|id| pane.get(id)) else { return };
            match g.action.as_deref() {
                Some("read") => set_state(Some(pane), r, Some(!r.read), None),
                Some("star") => set_state(Some(pane), r, None, Some(!r.starred)),
                _ => {}
            }
        }
        "pull" => refresh_feeds(pane, sel.clone()),
        "next" if article.is_some() => step(pane, sel, article, 1),
        "prev" if article.is_some() => step(pane, sel, article, -1),
        _ => {}
    }
}

// ------------------------------------------------------------------ Seiten

#[component]
pub fn All(article: Option<i64>) -> Element {
    rsx! { ArticlesPage { sel: Sel::All, article } }
}

#[component]
pub fn Today(article: Option<i64>) -> Element {
    rsx! { ArticlesPage { sel: Sel::Today, article } }
}

#[component]
pub fn Starred(article: Option<i64>) -> Element {
    rsx! { ArticlesPage { sel: Sel::Starred, article } }
}

#[component]
pub fn FeedView(id: i64, article: Option<i64>) -> Element {
    rsx! { ArticlesPage { sel: Sel::Feed(id), article } }
}

#[component]
pub fn FolderView(id: i64, article: Option<i64>) -> Element {
    rsx! { ArticlesPage { sel: Sel::Folder(id), article } }
}

#[component]
pub fn Search(article: Option<i64>, q: String) -> Element {
    rsx! { ArticlesPage { sel: Sel::Search(q), article } }
}

/// Liste und Lesemodus nebeneinander (Handy: eins von beiden).
#[component]
fn ArticlesPage(sel: Sel, article: Option<i64>) -> Element {
    let items = use_signal(Vec::<ArticleSummary>::new);
    let cursor = use_signal(|| None::<String>);
    let loading = use_signal(|| false);
    let ready = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let generation = use_signal(|| 0u64);
    let refreshing = use_signal(|| false);
    let open = use_signal(|| None::<Article>);
    let pane = use_context_provider(|| Pane { items, cursor, loading, ready, error, generation, refreshing, open });

    // Aktuelle Auswahl für die Tastatur (die läuft in einer eigenen Schleife)
    let mut current = use_signal(|| (sel.clone(), article));
    use_effect(use_reactive!(|sel, article| current.set((sel, article))));

    // Laden bei anderer Auswahl, anderem Filter oder nach Änderungen (REFRESH)
    let unread_only = state::unread_only();
    let load_key = (sel.clone(), unread_only, REFRESH());
    let mut last = use_signal(String::new);
    use_effect(use_reactive!(|load_key| {
        let (s, u, _) = load_key;
        let k = format!("{}|{u}", s.key());
        let clear = *last.peek() != k;
        last.set(k);
        load(pane, s, u, false, clear);
    }));

    use_future(move || async move {
        let mut eval = document::eval(KEYS_JS);
        while let Ok(key) = eval.recv::<String>().await {
            let (sel, article) = current.peek().clone();
            on_key(&key, pane, &sel, article);
        }
    });
    use_drop(remove_key_listener);
    // Handy: Wischen und Ziehen (gestures.rs)
    use_future(move || {
        crate::gestures::listen("list", move |g| {
            let (sel, article) = current.peek().clone();
            on_gesture(&g, pane, &sel, article);
        })
    });

    let preview = article.and_then(|id| items.read().iter().find(|a| a.id == id).cloned());
    let list_class = if article.is_some() { "hidden md:flex" } else { "flex" };
    let reader_class = if article.is_some() { "flex" } else { "hidden md:flex" };
    rsx! {
        div { class: "flex h-full min-h-0 w-full",
            section { class: "{list_class} min-h-0 w-full flex-col border-r md:w-[21rem] md:shrink-0 xl:w-[25rem]",
                ArticleList { sel: sel.clone(), article }
            }
            section { class: "{reader_class} min-h-0 min-w-0 flex-1 flex-col",
                if let Some(id) = article {
                    Reader { key: "{id}", id, sel: sel.clone(), preview }
                } else {
                    ReaderEmpty {}
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Liste

#[component]
fn ArticleList(sel: Sel, article: Option<i64>) -> Element {
    let pane = use_context::<Pane>();
    let nav = use_navigator();
    let ov = OVERVIEW.read().clone();
    let prefs = state::prefs();
    let unread_only = state::unread_only();
    let title = sel.title(ov.as_ref());
    let items = pane.items.read().clone();
    let ready = (pane.ready)();
    let loading = (pane.loading)();
    let refreshing = (pane.refreshing)();
    let has_more = pane.cursor.read().is_some();
    let error = pane.error.read().clone();
    let feed = match &sel {
        Sel::Feed(id) => ov.as_ref().and_then(|o| o.feeds.iter().find(|f| f.id == *id).cloned()),
        _ => None,
    };
    let folder = match &sel {
        Sel::Folder(id) => ov.as_ref().and_then(|o| o.folders.iter().find(|f| f.id == *id).cloned()),
        _ => None,
    };
    let no_feeds = ov.as_ref().is_some_and(|o| o.feeds.is_empty());
    let count = ov.as_ref().and_then(|o| state::unread_for_key(o, &sel.key()));
    // Seit dem Laden neue Artikel eingetroffen?
    let fresh = match (count, LIST_BASELINE.read().as_ref()) {
        (Some(c), Some((k, n))) => *k == sel.key() && *n >= 0 && c > *n,
        _ => false,
    };

    // Suchfeld (bei der Suche mit dem Begriff vorbelegt)
    let current_q = match &sel {
        Sel::Search(q) => q.clone(),
        _ => String::new(),
    };
    let mut text = use_signal(|| current_q.clone());
    use_effect(use_reactive!(|current_q| text.set(current_q)));

    let mut feed_dialog = use_signal(|| false);
    let mut folder_dialog = use_signal(|| false);
    let s_more = sel.clone();
    let load_more = move || load(pane, s_more.clone(), state::unread_only(), true, false);
    let load_more2 = load_more.clone();
    let s_refresh = sel.clone();
    let s_mark = sel.clone();
    let can_mark = sel.mark_input(None).is_some();

    rsx! {
        // Kopf
        header { class: "shrink-0 border-b px-3 pt-3 pb-3 sm:px-4",
            div { class: "flex items-center gap-1",
                MenuButton {}
                div { class: "flex min-w-0 flex-1 items-center gap-2 pl-1",
                    {sel_icon(&sel, feed.as_ref())}
                    h1 { class: "truncate text-[15px] font-semibold tracking-tight", title: "{title}", "{title}" }
                    if let Some(n) = count.filter(|n| *n > 0) {
                        span { class: "shrink-0 text-xs tabular-nums text-muted-foreground", "{n}" }
                    }
                }
                button { class: ICON_BTN, title: tr("Jetzt abrufen (r)"), disabled: refreshing,
                    onclick: move |_| refresh_feeds(pane, s_refresh.clone()),
                    span { class: if refreshing { "grid animate-spin" } else { "grid" }, icons::RefreshCw { class: "size-4" } }
                }
                if can_mark {
                    button { class: ICON_BTN, title: tr("Alle als gelesen markieren (Shift+A)"), disabled: items.is_empty(),
                        onclick: move |_| mark_all_read(pane, s_mark.clone()),
                        icons::CheckCheck {}
                    }
                }
                if let Some(f) = feed.clone() {
                    Dropdown { align: Align::End, class: "w-56",
                        trigger_class: ICON_BTN,
                        trigger: rsx! { icons::EllipsisVertical { class: "size-4" } },
                        if !f.site_url.is_empty() {
                            MenuItem { onclick: { let u = f.site_url.clone(); move |_| util::open_tab(&u) }, icons::Globe {} {tr("Webseite öffnen")} }
                        }
                        MenuItem { onclick: move |_| feed_dialog.set(true), icons::Pencil {} {tr("Umbenennen oder verschieben")} }
                        MenuSeparator {}
                        MenuItem { destructive: true,
                            onclick: {
                                let f = f.clone();
                                move |_| {
                                    let f = f.clone();
                                    spawn(async move {
                                        if unsubscribe(&f).await {
                                            navigator().replace(Route::All { article: None });
                                        }
                                    });
                                }
                            },
                            icons::Trash2 {}
                            {tr("Abbestellen")}
                        }
                    }
                }
                if let Some(f) = folder.clone() {
                    Dropdown { align: Align::End, class: "w-52",
                        trigger_class: ICON_BTN,
                        trigger: rsx! { icons::EllipsisVertical { class: "size-4" } },
                        MenuItem { onclick: move |_| folder_dialog.set(true), icons::Pencil {} {tr("Umbenennen")} }
                        MenuSeparator {}
                        MenuItem { destructive: true,
                            onclick: {
                                let f = f.clone();
                                move |_| {
                                    let f = f.clone();
                                    spawn(async move {
                                        if delete_folder(&f).await {
                                            navigator().replace(Route::All { article: None });
                                        }
                                    });
                                }
                            },
                            icons::Trash2 {}
                            {tr("Ordner löschen")}
                        }
                    }
                }
            }
            div { class: "mt-3 flex items-center gap-2",
                form { class: "relative min-w-0 flex-1",
                    onsubmit: move |e| {
                        e.prevent_default();
                        // `&` würde die Adresse zerteilen
                        let q = text().replace('&', " ").trim().to_string();
                        if q.is_empty() {
                            nav.push(Route::All { article: None });
                        } else {
                            nav.push(Route::Search { article: None, q });
                        }
                    },
                    icons::Search { class: "pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" }
                    input {
                        id: "search-input",
                        class: input_class("h-8 px-8"),
                        r#type: "search",
                        enterkeyhint: "search",
                        placeholder: tr("Suchen …"),
                        value: "{text}",
                        oninput: move |e| text.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Escape {
                                document::eval("document.activeElement && document.activeElement.blur()");
                            }
                        },
                    }
                    if matches!(sel, Sel::Search(_)) {
                        button {
                            r#type: "button",
                            class: "absolute top-1/2 right-1.5 grid size-5 -translate-y-1/2 place-items-center rounded text-muted-foreground hover:text-foreground",
                            title: tr("Suche löschen"),
                            onclick: move |_| { nav.push(Route::All { article: None }); },
                            icons::X { class: "size-3.5" }
                        }
                    }
                }
                if sel.has_unread_toggle() {
                    Segmented {
                        class: "h-8 shrink-0 text-xs [&>button]:px-2.5 [&>button]:text-xs",
                        options: vec![("unread".to_string(), tr("Ungelesen")), ("all".to_string(), tr("Alle"))],
                        value: if unread_only { "unread" } else { "all" },
                        onchange: move |v: String| *UNREAD_ONLY.write() = Some(v == "unread"),
                    }
                }
            }
            if let Some(e) = feed.as_ref().and_then(|f| f.last_error.clone()).filter(|e| !e.trim().is_empty()) {
                div { class: "mt-3 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs",
                    icons::CircleAlert { class: "mt-px size-3.5 shrink-0 text-destructive" }
                    span { class: "min-w-0 break-words", {trf("Letzter Abruf fehlgeschlagen: {0}", &[&e])} }
                }
            }
        }

        // Liste
        div {
            id: "article-list",
            class: "relative min-h-0 flex-1 overflow-y-auto overscroll-contain",
            onscroll: move |e| {
                let d = e.data();
                if d.scroll_top() + d.client_height() as f64 >= d.scroll_height() as f64 - 600.0 {
                    load_more();
                }
            },
            // Ziehen zum Aktualisieren (Höhe setzt gestures.rs beim Ziehen)
            div { id: "ptr", class: if refreshing { "ptr h-12" } else { "ptr h-0" },
                if refreshing {
                    icons::LoaderCircle { class: "size-5 animate-spin" }
                } else {
                    span { class: "ptr-arrow grid", icons::ArrowDown { class: "size-5" } }
                }
            }
            if fresh {
                div { class: "sticky top-2 z-10 flex justify-center",
                    button {
                        class: "inline-flex items-center gap-1.5 rounded-full border bg-popover px-3 py-1 text-xs font-medium shadow-lg animate-in hover:bg-accent",
                        onclick: move |_| state::refresh(),
                        icons::ArrowUpRight { class: "size-3.5 -rotate-45" }
                        {tr("Neue Artikel")}
                    }
                }
            }
            if !ready {
                ListSkeleton { compact: prefs.compact_list }
            } else if let Some(e) = error {
                div { class: "p-4",
                    Callout { text: e, error: true,
                        button { class: "{btn_sm(BTN_OUTLINE)} justify-self-start", onclick: move |_| state::refresh(), {tr("Nochmals versuchen")} }
                    }
                }
            } else if items.is_empty() {
                ListEmpty { sel: sel.clone(), unread_only, no_feeds }
            } else {
                div { class: "animate-fade-in",
                    for a in items.iter().cloned() {
                        ArticleRow {
                            key: "{a.id}",
                            active: article == Some(a.id),
                            replace: article.is_some(),
                            compact: prefs.compact_list,
                            show_feed: !matches!(sel, Sel::Feed(_)),
                            to: sel.route(Some(a.id)),
                            a,
                        }
                    }
                }
                div { class: "flex justify-center px-4 pt-6 pb-24 lg:pb-6",
                    if loading {
                        icons::LoaderCircle { class: "size-4 animate-spin text-muted-foreground" }
                    } else if has_more {
                        button { class: btn_sm(BTN_OUTLINE), onclick: move |_| load_more2(), icons::ChevronDown { class: "size-4" } {tr("Mehr laden")} }
                    } else {
                        span { class: "text-xs text-muted-foreground", {tr("Das war alles.")} }
                    }
                }
            }
        }
        FeedDialog { open: feed_dialog, feed }
        FolderDialog { open: folder_dialog, folder }
    }
}

fn sel_icon(sel: &Sel, feed: Option<&Feed>) -> Element {
    let c = "size-4 shrink-0 text-muted-foreground";
    match sel {
        Sel::All => rsx! { icons::Inbox { class: c } },
        Sel::Today => rsx! { icons::Sun { class: c } },
        Sel::Starred => rsx! { icons::Star { class: c } },
        Sel::Folder(_) => rsx! { icons::FolderOpen { class: c } },
        Sel::Search(_) => rsx! { icons::Search { class: c } },
        Sel::Feed(_) => match feed {
            Some(f) => rsx! { FeedIcon { src: f.icon_url.clone(), title: f.title.clone() } },
            None => rsx! { icons::Rss { class: c } },
        },
    }
}

#[component]
fn ArticleRow(a: ArticleSummary, active: bool, replace: bool, compact: bool, show_feed: bool, to: Route) -> Element {
    let state = if active { "bg-accent/70" } else { "hover:bg-accent/30" };
    let title_class = if a.read { "text-muted-foreground" } else { "font-semibold text-foreground" };
    let title = if a.title.trim().is_empty() { tr("(ohne Titel)") } else { a.title.clone() };
    let pad = if compact { "py-2.5" } else { "py-3.5" };
    rsx! {
        // Auf dem Handy wischbar: nach rechts gelesen/ungelesen, nach links merken (gestures.rs)
        div { class: "swipe-row border-b border-border/60", "data-swipe-id": "{a.id}",
            div { class: "swipe-under", "aria-hidden": "true",
                span {
                    if a.read { icons::CircleDot { class: "size-5" } } else { icons::CircleCheck { class: "size-5" } }
                    if a.read { {tr("Ungelesen")} } else { {tr("Gelesen")} }
                }
                span {
                    if a.starred { {tr("Nicht merken")} } else { {tr("Merken")} }
                    icons::Star { class: if a.starred { "size-5" } else { "size-5 fill-current" } }
                }
            }
            Link {
                to: to.clone(),
                id: "row-{a.id}",
                // Ist schon ein Artikel offen, ersetzt der nächste ihn im Verlauf (wie j/k)
                onclick_only: true,
                onclick: move |_| {
                    let nav = navigator();
                    if replace {
                        nav.replace(to.clone());
                    } else {
                        *FROM_LIST.write() = true;
                        nav.push(to.clone());
                    }
                },
                class: "swipe-fg group flex gap-3 px-4 outline-none transition-colors select-none focus-visible:bg-accent/40 md:select-auto {pad} {state}",
                // Ungelesen-Punkt
                span { class: if a.read { "mt-[0.45rem] size-1.5 shrink-0 rounded-full" } else { "mt-[0.45rem] size-1.5 shrink-0 rounded-full bg-foreground" } }
                div { class: "min-w-0 flex-1",
                    div { class: "flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground",
                        if show_feed {
                            FeedIcon { src: a.feed_icon.clone(), title: a.feed_title.clone(), class: "size-3.5 text-[8px]" }
                            span { class: "min-w-0 truncate", "{a.feed_title}" }
                            span { class: "shrink-0 opacity-60", "·" }
                        }
                        span { class: "shrink-0 whitespace-nowrap", title: "{util::datetime(a.published_at)}", "{util::relative(a.published_at)}" }
                        if a.starred {
                            icons::Star { class: "ml-auto size-3.5 shrink-0 fill-current text-foreground" }
                        }
                    }
                    p { class: "mt-1 line-clamp-2 text-[0.94rem] leading-snug {title_class}", "{title}" }
                    if !compact && !a.excerpt.trim().is_empty() {
                        p { class: "mt-1 line-clamp-2 text-[0.82rem] leading-relaxed text-muted-foreground/80", "{a.excerpt}" }
                    }
                }
                if !compact && !a.image_url.is_empty() {
                    Thumb { src: a.image_url.clone() }
                }
            }
        }
    }
}

/// Vorschaubild in der Liste; verschwindet, wenn es nicht lädt.
#[component]
fn Thumb(src: String) -> Element {
    let mut failed = use_signal(|| false);
    if failed() {
        return rsx! {};
    }
    rsx! {
        img {
            class: "mt-1 size-16 shrink-0 rounded-md bg-muted object-cover",
            src: "{src}",
            alt: "",
            loading: "lazy",
            decoding: "async",
            referrerpolicy: "no-referrer",
            onerror: move |_| failed.set(true),
        }
    }
}

#[component]
fn ListSkeleton(compact: bool) -> Element {
    rsx! {
        div { class: "divide-y divide-border/60",
            for i in 0..8 {
                div { key: "{i}", class: "flex gap-3 px-4 py-3.5",
                    span { class: "mt-1.5 size-1.5 shrink-0 rounded-full bg-muted" }
                    div { class: "min-w-0 flex-1 space-y-2",
                        div { class: "h-3 w-1/3 animate-pulse rounded bg-muted" }
                        div { class: "h-3.5 w-11/12 animate-pulse rounded bg-muted" }
                        if !compact {
                            div { class: "h-3 w-3/4 animate-pulse rounded bg-muted/70" }
                        }
                    }
                    if !compact && i % 3 == 0 {
                        div { class: "mt-1 size-16 shrink-0 animate-pulse rounded-md bg-muted" }
                    }
                }
            }
        }
    }
}

#[component]
fn ListEmpty(sel: Sel, unread_only: bool, no_feeds: bool) -> Element {
    if no_feeds {
        return rsx! {
            EmptyState { title: tr("Noch keine Feeds"), description: tr("Füge die Adresse einer Webseite oder eines Feeds hinzu – oder importiere deine Abos als OPML in den Einstellungen."), icon: rsx! { icons::Rss {} },
                button { class: btn(BTN_PRIMARY), onclick: move |_| *ADD_FEED_OPEN.write() = true, icons::Plus { class: "size-4" } {tr("Feed hinzufügen")} }
            }
        };
    }
    match sel {
        Sel::Search(q) => rsx! {
            EmptyState { title: tr("Keine Treffer"), description: trf("Für „{0}“ wurde nichts gefunden.", &[&q]), icon: rsx! { icons::Search {} } }
        },
        Sel::Starred => rsx! {
            EmptyState { title: tr("Noch nichts gemerkt"), description: tr("Mit dem Stern (oder s) merkst du dir Artikel für später."), icon: rsx! { icons::Star {} } }
        },
        _ if unread_only && sel.has_unread_toggle() => rsx! {
            EmptyState { title: tr("Alles gelesen"), description: tr("Keine ungelesenen Artikel. Zeit für einen Kaffee."), icon: rsx! { icons::CheckCheck {} },
                button { class: btn_sm(BTN_OUTLINE), onclick: move |_| *UNREAD_ONLY.write() = Some(false), {tr("Alle anzeigen")} }
            }
        },
        _ => rsx! {
            EmptyState { title: tr("Keine Artikel"), description: tr("Hier ist noch nichts eingetroffen."), icon: rsx! { icons::Inbox {} } }
        },
    }
}

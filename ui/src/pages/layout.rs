//! Rahmen: Anmeldung prüfen (AppLayout), dann Seitenleiste mit Ansichten, Ordnern und Feeds,
//! unten Einstellungen und Profilmenü. Auf schmalen Bildschirmen ist die Seitenleiste ein Sheet.

use crate::api;
use crate::components::add_feed::AddFeedDialog;
use crate::components::common::*;
use crate::components::feed_icon::FeedIcon;
use crate::components::overlay::*;
use crate::components::shortcuts::{SHORTCUTS_OPEN, ShortcutsHelp};
use crate::i18n::{self, tr};
use crate::state::{self, ADD_FEED_OPEN, LIST_BASELINE, ME, OVERVIEW, SIDEBAR_OPEN};
use crate::util;
use crate::Route;
use dioxus::prelude::*;
use nrznews_types::{AuthStatus, Feed, Folder};
use std::collections::BTreeSet;

/// Zugeklappte Ordner (im Browser gemerkt).
static COLLAPSED: GlobalSignal<BTreeSet<i64>> = Signal::global(|| {
    util::storage_get("nrznews-collapsed")
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default()
});

fn toggle_folder(id: i64) {
    let mut c = COLLAPSED.write();
    if !c.remove(&id) {
        c.insert(id);
    }
    util::storage_set("nrznews-collapsed", &c.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
}

/// Prüft die Anmeldung; ohne Konto zur Ersteinrichtung, ohne Anmeldung zur Anmeldeseite.
#[component]
pub fn AppLayout() -> Element {
    let nav = use_navigator();
    let mut status = use_resource(|| async move {
        let s = api::get::<AuthStatus>("/auth/status").await?;
        if let Some(user) = s.user.clone() {
            i18n::set_language(&user.preferences.language, true).await;
            *ME.write() = Some(user);
        }
        Ok::<AuthStatus, api::ApiError>(s)
    });
    match &*status.read() {
        None => rsx! { Splash {} },
        Some(Err(e)) => rsx! {
            div { class: "grid min-h-dvh place-items-center p-6",
                EmptyState { title: tr("Server nicht erreichbar"), description: e.message.clone(), icon: rsx! { icons::CircleAlert {} },
                    button { class: btn(BTN_OUTLINE), onclick: move |_| status.restart(), icons::RotateCw { class: "size-4" } {tr("Nochmals versuchen")} }
                }
            }
        },
        Some(Ok(s)) if s.setup_required => {
            nav.replace(Route::Setup {});
            rsx! { Splash {} }
        }
        Some(Ok(s)) if !s.authenticated || s.user.is_none() => {
            nav.replace(Route::Login { next: util::current_path() });
            rsx! { Splash {} }
        }
        Some(Ok(_)) => rsx! { Shell {} },
    }
}

#[component]
fn Splash() -> Element {
    rsx! {
        div { class: "grid min-h-dvh place-items-center",
            img { src: asset!("/assets/icon.png"), class: "size-10 animate-pulse rounded-xl opacity-80", alt: "" }
        }
    }
}

/// Angemeldet: Seitenleiste, Inhalt, Dieter, Dialoge; Zähler alle 60 s auffrischen.
#[component]
fn Shell() -> Element {
    let route = use_route::<Route>();
    // Seitenwechsel schliesst die Seitenleiste auf dem Handy. Ohne geöffneten Artikel bleibt von
    // `?article=` ein leeres „?“ in der Adresse stehen – das kommt weg.
    use_effect(use_reactive!(|route| {
        let _ = route;
        *SIDEBAR_OPEN.write() = false;
        document::eval("if (location.href.endsWith('?')) history.replaceState(history.state, '', location.pathname + location.hash)");
    }));
    use_future(|| async {
        state::load_overview().await;
        loop {
            // Noch kein Stand für „Neue Artikel“ (Liste vor dem Überblick geladen) → jetzt merken
            if LIST_BASELINE.peek().as_ref().is_some_and(|(_, n)| *n < 0) {
                state::rebase();
            }
            gloo_timers::future::TimeoutFuture::new(60_000).await;
            state::load_overview().await;
        }
    });
    // Seitenleiste auf dem Handy: nach links wischen schliesst sie
    use_future(|| crate::gestures::listen("shell", |g| {
        if g.t == "close-sheet" {
            *SIDEBAR_OPEN.write() = false;
        }
    }));
    let unread = OVERVIEW.read().as_ref().map(|o| o.unread).unwrap_or(0);
    let title = if unread > 0 { format!("({unread}) nrzNews") } else { "nrzNews".to_string() };
    let reading = open_article(&route).is_some();
    rsx! {
        document::Title { "{title}" }
        div { class: "safe-x flex h-dvh flex-col overflow-hidden",
            div { class: "flex min-h-0 flex-1",
                // Seitenleiste (Desktop)
                aside { class: "hidden w-64 shrink-0 border-r border-sidenav-border bg-sidenav lg:flex lg:flex-col",
                    Sidebar {}
                }
                main { class: "min-h-0 min-w-0 flex-1", Outlet::<Route> {} }
            }
            // Tab-Leiste (Handy/Tablet); beim Lesen auf dem Handy hat der Lesemodus eine eigene
            TabBar { reading }
        }
        // Seitenleiste (Handy/Tablet) als Sheet
        if SIDEBAR_OPEN() {
            div { class: "fixed inset-0 z-50 bg-black/60 backdrop-blur-[2px] animate-fade-in lg:hidden", onclick: move |_| *SIDEBAR_OPEN.write() = false }
            aside {
                id: "mobile-sheet",
                class: "safe-bottom fixed inset-y-0 left-0 z-50 flex w-[18rem] max-w-[85vw] flex-col border-r border-sidenav-border bg-sidenav pl-[env(safe-area-inset-left)] shadow-xl animate-slide-in lg:hidden",
                Sidebar {}
            }
        }
        crate::components::assistant::Assistant {}
        AddFeedDialog {}
        ShortcutsHelp {}
    }
}

/// Im Lesemodus geöffneter Artikel (`?article=`).
pub fn open_article(route: &Route) -> Option<i64> {
    match route {
        Route::All { article }
        | Route::Today { article }
        | Route::Starred { article }
        | Route::FeedView { article, .. }
        | Route::FolderView { article, .. }
        | Route::Search { article, .. } => *article,
        _ => None,
    }
}

/// Untere Leiste auf Handy und Tablet: die wichtigsten Ansichten mit dem Daumen erreichbar,
/// „Feeds“ öffnet die Seitenleiste. Nochmals auf den aktiven Reiter tippen scrollt nach oben.
#[component]
fn TabBar(reading: bool) -> Element {
    let route = use_route::<Route>();
    let (unread, today) = OVERVIEW.read().as_ref().map(|o| (o.unread, o.today)).unwrap_or_default();
    let feeds_active = SIDEBAR_OPEN() || matches!(route, Route::FeedView { .. } | Route::FolderView { .. } | Route::Settings { .. });
    let vis = if reading { "hidden md:block" } else { "block" };
    let feeds_class = if feeds_active { "text-foreground" } else { "text-muted-foreground" };
    rsx! {
        nav { class: "{vis} safe-bottom shrink-0 border-t border-sidenav-border bg-sidenav lg:hidden",
            div { class: "mx-auto grid h-14 max-w-xl grid-cols-5",
                Tab { to: Route::Briefing {}, label: tr("Überblick"), active: matches!(route, Route::Briefing {}), icon: rsx! { icons::Sparkles {} } }
                Tab { to: Route::All { article: None }, label: tr("Alle"), count: unread, active: matches!(route, Route::All { .. } | Route::Search { .. }), icon: rsx! { icons::Inbox {} } }
                Tab { to: Route::Today { article: None }, label: tr("Heute"), count: today, active: matches!(route, Route::Today { .. }), icon: rsx! { icons::Sun {} } }
                Tab { to: Route::Starred { article: None }, label: tr("Gemerkt"), active: matches!(route, Route::Starred { .. }), icon: rsx! { icons::Star {} } }
                button {
                    class: "flex flex-col items-center justify-center gap-1 text-[10.5px] font-medium transition-colors active:scale-95 [&_svg]:size-[1.35rem] {feeds_class}",
                    "aria-label": tr("Feeds"),
                    onclick: move |_| {
                        let open = *SIDEBAR_OPEN.peek();
                        *SIDEBAR_OPEN.write() = !open;
                    },
                    icons::Rss {}
                    span { {tr("Feeds")} }
                }
            }
        }
    }
}

#[component]
fn Tab(to: Route, #[props(into)] label: String, icon: Element, active: bool, #[props(default)] count: i64) -> Element {
    let c = if active { "text-foreground" } else { "text-muted-foreground" };
    let badge = if count > 99 { "99+".to_string() } else { count.to_string() };
    rsx! {
        Link {
            to,
            class: "flex flex-col items-center justify-center gap-1 text-[10.5px] font-medium transition-colors active:scale-95 [&_svg]:size-[1.35rem] {c}",
            "aria-current": if active { "page" } else { "false" },
            onclick: move |_| {
                *SIDEBAR_OPEN.write() = false;
                if active {
                    document::eval("document.getElementById('article-list')?.scrollTo({ top: 0, behavior: 'smooth' }); document.getElementById('briefing-scroll')?.scrollTo({ top: 0, behavior: 'smooth' })");
                }
            },
            span { class: "relative",
                {icon}
                if count > 0 {
                    span { class: "absolute -top-1.5 left-[calc(100%-0.5rem)] min-w-[1.1rem] rounded-full bg-foreground px-1 text-center text-[9.5px] leading-[1.1rem] font-semibold tabular-nums text-background", "{badge}" }
                }
            }
            span { "{label}" }
        }
    }
}

#[component]
fn NavItem(to: Route, #[props(into)] label: String, icon: Element, active: bool, #[props(default)] count: i64) -> Element {
    let c = if active {
        "bg-sidenav-accent font-medium text-sidenav-accent-foreground"
    } else {
        "text-muted-foreground hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground"
    };
    rsx! {
        Link { to, class: "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors [&_svg]:size-4 [&_svg]:shrink-0 {c}",
            {icon}
            span { class: "flex-1 truncate", "{label}" }
            Count { n: count, strong: active }
        }
    }
}

#[component]
fn Sidebar() -> Element {
    let route = use_route::<Route>();
    let ov = OVERVIEW.read().clone();
    let me = ME.read().clone();
    let (unread, today, starred) = ov.as_ref().map(|o| (o.unread, o.today, o.starred)).unwrap_or_default();
    let folders: Vec<Folder> = ov.as_ref().map(|o| o.folders.clone()).unwrap_or_default();
    let feeds: Vec<Feed> = ov.as_ref().map(|o| o.feeds.clone()).unwrap_or_default();
    let loose: Vec<Feed> = feeds.iter().filter(|f| f.folder_id.is_none_or(|id| !folders.iter().any(|x| x.id == id))).cloned().collect();
    let collapsed = COLLAPSED.read().clone();
    let active_feed = match &route {
        Route::FeedView { id, .. } => Some(*id),
        _ => None,
    };
    let active_folder = match &route {
        Route::FolderView { id, .. } => Some(*id),
        _ => None,
    };

    rsx! {
        // Logo
        div { class: "flex h-14 shrink-0 items-center gap-2.5 px-4",
            Link { to: Route::All { article: None }, class: "flex items-center gap-2.5",
                img { src: asset!("/assets/icon.png"), class: "size-7 rounded-lg", alt: "" }
                span { class: "text-[15px] font-semibold tracking-tight", "nrzNews" }
            }
        }

        nav { class: "flex flex-col gap-0.5 px-3",
            NavItem { to: Route::Briefing {}, label: tr("Überblick"), active: matches!(route, Route::Briefing {}), icon: rsx! { icons::Sparkles {} } }
            NavItem { to: Route::All { article: None }, label: tr("Alle Artikel"), count: unread, active: matches!(route, Route::All { .. }), icon: rsx! { icons::Inbox {} } }
            NavItem { to: Route::Today { article: None }, label: tr("Heute"), count: today, active: matches!(route, Route::Today { .. }), icon: rsx! { icons::Sun {} } }
            NavItem { to: Route::Starred { article: None }, label: tr("Gemerkt"), count: starred, active: matches!(route, Route::Starred { .. }), icon: rsx! { icons::Star {} } }
        }

        // Feeds
        div { class: "mt-6 flex items-center justify-between pr-3 pl-5",
            span { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", {tr("Feeds")} }
            button {
                class: "grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-sidenav-accent hover:text-foreground",
                title: tr("Feed hinzufügen"),
                onclick: move |_| *ADD_FEED_OPEN.write() = true,
                icons::Plus { class: "size-4" }
            }
        }
        div { class: "mt-1 min-h-0 flex-1 overflow-y-auto px-3 pb-3",
            if ov.is_none() {
                div { class: "grid gap-2 px-2.5 py-2",
                    for w in ["w-3/4", "w-1/2", "w-2/3", "w-3/5"] {
                        div { class: "h-3.5 animate-pulse rounded bg-muted {w}" }
                    }
                }
            } else if feeds.is_empty() {
                div { class: "rounded-lg border border-dashed px-3 py-4 text-center",
                    p { class: "text-sm text-muted-foreground", {tr("Noch keine Feeds.")} }
                    button { class: "{btn_sm(BTN_OUTLINE)} mt-3", onclick: move |_| *ADD_FEED_OPEN.write() = true,
                        icons::Plus { class: "size-4" }
                        {tr("Feed hinzufügen")}
                    }
                }
            }
            for folder in folders.iter().cloned() {
                FolderNode {
                    key: "{folder.id}",
                    feeds: feeds.iter().filter(|f| f.folder_id == Some(folder.id)).cloned().collect::<Vec<_>>(),
                    open: !collapsed.contains(&folder.id),
                    active: active_folder == Some(folder.id),
                    active_feed,
                    folder,
                }
            }
            for f in loose {
                FeedRow { key: "{f.id}", active: active_feed == Some(f.id), feed: f, indent: false }
            }
            if !feeds.is_empty() {
                button {
                    class: "mt-2 flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm text-muted-foreground hover:bg-sidenav-accent/60 hover:text-foreground",
                    onclick: move |_| *ADD_FEED_OPEN.write() = true,
                    icons::Plus { class: "size-4" }
                    {tr("Feed hinzufügen")}
                }
            }
        }

        // Unten: Profil und Einstellungen
        div { class: "flex items-center gap-1 border-t border-sidenav-border p-3",
            div { class: "min-w-0 flex-1",
                if let Some(u) = me {
                    ProfileMenu { name: u.name().to_string(), admin: u.is_admin() }
                }
            }
            Link {
                to: Route::Settings { section: "account".into() },
                class: if matches!(route, Route::Settings { .. }) { "grid size-9 shrink-0 place-items-center rounded-md bg-sidenav-accent text-foreground" } else { "grid size-9 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-sidenav-accent hover:text-foreground" },
                title: tr("Einstellungen"),
                icons::Settings { class: "size-4" }
            }
        }
    }
}

#[component]
fn FolderNode(folder: Folder, feeds: Vec<Feed>, open: bool, active: bool, active_feed: Option<i64>) -> Element {
    let c = if active {
        "bg-sidenav-accent font-medium text-sidenav-accent-foreground"
    } else {
        "text-muted-foreground hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground"
    };
    let id = folder.id;
    let errors = feeds.iter().filter(|f| f.last_error.as_ref().is_some_and(|e| !e.is_empty())).count();
    rsx! {
        div { class: "mt-0.5",
            div { class: "group flex items-center rounded-md transition-colors {c}",
                button {
                    class: "grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:text-foreground",
                    title: if open { tr("Zuklappen") } else { tr("Aufklappen") },
                    "aria-expanded": "{open}",
                    onclick: move |_| toggle_folder(id),
                    span { class: if open { "grid rotate-90 transition-transform" } else { "grid transition-transform" }, icons::ChevronRight { class: "size-3.5" } }
                }
                Link { to: Route::FolderView { id, article: None }, class: "flex min-w-0 flex-1 items-center gap-2 py-1.5 pr-2.5 text-sm",
                    if open {
                        icons::FolderOpen { class: "size-4 shrink-0" }
                    } else {
                        icons::Folder { class: "size-4 shrink-0" }
                    }
                    span { class: "min-w-0 flex-1 truncate", "{folder.name}" }
                    if errors > 0 && !open {
                        span { class: "size-1.5 shrink-0 rounded-full bg-destructive", title: tr("Ein Feed in diesem Ordner hat Probleme beim Abrufen.") }
                    }
                    Count { n: folder.unread, strong: active }
                }
            }
            if open {
                div { class: "mt-0.5 flex flex-col gap-0.5",
                    for f in feeds {
                        FeedRow { key: "{f.id}", active: active_feed == Some(f.id), feed: f, indent: true }
                    }
                }
            }
        }
    }
}

#[component]
fn FeedRow(feed: Feed, active: bool, indent: bool) -> Element {
    let c = if active {
        "bg-sidenav-accent font-medium text-sidenav-accent-foreground"
    } else if feed.unread > 0 {
        "text-sidenav-foreground/90 hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground"
    } else {
        "text-muted-foreground hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground"
    };
    let pad = if indent { "pl-7" } else { "pl-2.5" };
    let error = feed.last_error.clone().filter(|e| !e.trim().is_empty());
    rsx! {
        Link { to: Route::FeedView { id: feed.id, article: None }, class: "flex items-center gap-2.5 rounded-md py-1.5 pr-2.5 text-sm transition-colors {pad} {c}", title: "{feed.title}",
            FeedIcon { src: feed.icon_url.clone(), title: feed.title.clone() }
            span { class: "min-w-0 flex-1 truncate", "{feed.title}" }
            if let Some(e) = error {
                span { class: "size-1.5 shrink-0 rounded-full bg-destructive", title: "{e}" }
            }
            Count { n: feed.unread, strong: active }
        }
    }
}

#[component]
fn ProfileMenu(name: String, admin: bool) -> Element {
    let nav = use_navigator();
    let go = move |section: &'static str| move |_| {
        nav.push(Route::Settings { section: section.into() });
    };
    rsx! {
        Dropdown { side: Side::Top, class: "w-56",
            trigger_class: "flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-sidenav-accent",
            trigger: rsx! {
                span { class: "grid size-8 shrink-0 place-items-center rounded-full bg-secondary text-sm font-semibold", "{util::initial(&name)}" }
                span { class: "min-w-0 flex-1",
                    span { class: "block truncate text-sm font-medium", "{name}" }
                    span { class: "block text-xs text-muted-foreground",
                        if admin { {tr("Administrator")} } else { {tr("Konto")} }
                    }
                }
                icons::ChevronsUpDown { class: "size-4 shrink-0 text-muted-foreground" }
            },
            MenuLabel { {tr("Einstellungen")} }
            MenuItem { onclick: go("account"), icons::User {} {tr("Konto")} }
            MenuItem { onclick: go("reading"), icons::BookOpen {} {tr("Lesen")} }
            MenuItem { onclick: go("feeds"), icons::Rss {} {tr("Feeds & Ordner")} }
            if admin {
                MenuSeparator {}
                MenuLabel { {tr("Instanz")} }
                MenuItem { onclick: go("users"), icons::Users {} {tr("Benutzer")} }
                MenuItem { onclick: go("instance"), icons::ShieldCheck {} {tr("Registrierung & Abruf")} }
                MenuItem { onclick: go("ai"), icons::Sparkles {} {tr("KI & Modelle")} }
            }
            MenuSeparator {}
            MenuItem { onclick: move |_| *SHORTCUTS_OPEN.write() = true, icons::Keyboard {} {tr("Tastenkürzel")} }
            MenuItem {
                onclick: move |_| {
                    spawn(async move {
                        let _: api::ApiResult<serde_json::Value> = api::action("/auth/logout").await;
                        *ME.write() = None;
                        if let Some(w) = web_sys::window() {
                            let _ = w.location().assign("/login");
                        }
                    });
                },
                icons::LogOut {}
                {tr("Abmelden")}
            }
        }
    }
}

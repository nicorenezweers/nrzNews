//! nrzNews – Oberfläche (Dioxus 0.7, WebAssembly) mit Komponenten von rust-ui.
//! Spricht nur mit der JSON-API unter `/api/v1` (Sitzungs-Cookie); die Typen kommen aus
//! `nrznews-types`, damit Server und Oberfläche nie auseinanderlaufen.

mod api;
mod components;
mod gestures;
mod i18n;
mod markdown;
mod pages;
mod state;
mod ui;
mod util;

use components::overlay::{ConfirmHost, Toaster};
use dioxus::prelude::*;
use pages::articles::{All, FeedView, FolderView, Search, Starred, Today};
use pages::auth::{Login, Register, Setup};
use pages::briefing::Briefing;
use pages::layout::AppLayout;
use pages::not_found::NotFound;
use pages::settings::Settings;

static CSS: Asset = asset!("/assets/tailwind.css");
static ICON: Asset = asset!("/assets/icon.png");
static FAVICON: Asset = asset!("/assets/favicon-32.png");

/// Adressen der App. Der geöffnete Artikel steht als `?article=` in der Adresse, damit
/// Neuladen und Zurück funktionieren.
#[derive(Routable, Clone, PartialEq, Debug)]
#[rustfmt::skip]
pub enum Route {
    #[route("/login?:next")]
    Login { next: String },
    #[route("/setup")]
    Setup {},
    #[route("/register")]
    Register {},
    #[layout(AppLayout)]
        #[route("/?:article")]
        All { article: Option<i64> },
        #[route("/today?:article")]
        Today { article: Option<i64> },
        #[route("/starred?:article")]
        Starred { article: Option<i64> },
        #[route("/feeds/:id?:article")]
        FeedView { id: i64, article: Option<i64> },
        #[route("/folders/:id?:article")]
        FolderView { id: i64, article: Option<i64> },
        #[route("/search?:article&:q")]
        Search { article: Option<i64>, q: String },
        #[route("/briefing")]
        Briefing {},
        #[redirect("/settings", || Route::Settings { section: "account".into() })]
        #[route("/settings/:section")]
        Settings { section: String },
    #[end_layout]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut ready = use_signal(|| false);
    use_future(move || async move {
        i18n::init().await;
        ready.set(true);
    });
    // Handy: bis an die Ränder zeichnen (Notch, Home-Leiste – die Abstände kommen aus
    // env(safe-area-inset-*)), nicht per Doppeltipp/Pinch auf die Oberfläche zoomen.
    use_hook(|| {
        document::eval(
            "let m = document.querySelector('meta[name=viewport]');\
             if (!m) { m = document.createElement('meta'); m.name = 'viewport'; document.head.appendChild(m); }\
             m.content = 'width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content';",
        );
    });
    rsx! {
        document::Stylesheet { href: CSS }
        document::Link { rel: "icon", r#type: "image/png", href: FAVICON }
        document::Link { rel: "apple-touch-icon", href: ICON }
        document::Link { rel: "manifest", href: "/static/manifest.webmanifest" }
        document::Meta { name: "theme-color", content: "#121212" }
        document::Meta { name: "mobile-web-app-capable", content: "yes" }
        document::Meta { name: "apple-mobile-web-app-capable", content: "yes" }
        document::Meta { name: "apple-mobile-web-app-status-bar-style", content: "black" }
        document::Meta { name: "apple-mobile-web-app-title", content: "nrzNews" }
        if ready() {
            Router::<Route> {}
        }
        ConfirmHost {}
        Toaster {}
    }
}

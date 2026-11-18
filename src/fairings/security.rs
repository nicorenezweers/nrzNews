//! Sicherheits-Header für jede Antwort. Die Content-Security-Policy erlaubt Skripte, Stile
//! und Schrift nur vom eigenen Server: Oberfläche und Schrift sind eingebettet (siehe
//! routes::spa und routes::assets), Skripte nur als Dateien – keine Inline-Skripte.
//! WebAssembly braucht 'wasm-unsafe-eval', Dioxus' `eval` 'unsafe-eval'.
//!
//! Ein RSS-Reader zeigt Bilder, Audio und Video fremder Seiten (aus den Artikeln), darum
//! `img-src`/`media-src` auch für http(s). Damit die Bild-Server dabei nicht erfahren, wer
//! was liest, schickt der Browser nie einen Referrer (`Referrer-Policy: no-referrer`).

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::{Request, Response};

pub struct SecurityHeaders;

pub const CSP: &str = "default-src 'self'; \
    script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; \
    font-src 'self'; \
    img-src 'self' data: blob: https: http:; \
    media-src 'self' https: http:; \
    worker-src 'self' blob:; \
    connect-src 'self'; \
    object-src 'none'; \
    frame-ancestors 'none'; \
    base-uri 'self'; \
    form-action 'self'";

#[rocket::async_trait]
impl Fairing for SecurityHeaders {
    fn info(&self) -> Info {
        Info {
            name: "Sicherheits-Header",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _req: &'r Request<'_>, res: &mut Response<'r>) {
        for (name, value) in [
            ("Content-Security-Policy", CSP),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
            (
                "Permissions-Policy",
                "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
            ),
            ("Cross-Origin-Opener-Policy", "same-origin"),
        ] {
            if !res.headers().contains(name) {
                res.set_header(Header::new(name, value));
            }
        }
    }
}

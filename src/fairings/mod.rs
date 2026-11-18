//! Fairings: Sicherheits-Header, Anfrage-Protokoll, Aufräumen abgelaufener Sitzungen und
//! sauberes Schliessen der Datenbank beim Beenden. (Das Abrufen der Feeds im Hintergrund
//! ist `feeds::refresher::Refresher`.)

mod close_database;
mod request_log;
mod security;
mod session_cleanup;

pub use close_database::CloseDatabase;
pub use request_log::RequestLog;
pub use security::SecurityHeaders;
pub use session_cleanup::SessionCleanup;

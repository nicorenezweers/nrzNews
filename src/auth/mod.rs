//! Anmeldung: Argon2id-Passwörter, Sitzungen in der Datenbank, die Bremse gegen
//! Passwort-Raten und die Request Guards, mit denen Routen Anmeldung und Rechte verlangen.

pub mod guards;
pub mod password;
pub mod session;
pub mod throttle;

pub use guards::{AdminUser, CurrentUser};

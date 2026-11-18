//! Passwörter als Argon2id-Hash (PHC-String). Hashen und Prüfen laufen auf einem
//! Blocking-Thread, damit sie die Server-Threads nicht aufhalten.

use crate::error::{AppError, AppResult};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use nrznews_types::MIN_PASSWORD_LEN;
use std::sync::LazyLock;

/// Längere Passwörter bringen nichts mehr, kosten aber Rechenzeit beim Hashen.
pub const MAX_LENGTH: usize = 1024;

/// Hash eines zufälligen Passworts – wird bei unbekannten Benutzernamen geprüft,
/// damit die Anmeldung immer gleich lange dauert.
pub static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(&rand::random::<[u8; 16]>(), &salt)
        .map(|h| h.to_string())
        .unwrap_or_default()
});

/// Mindestlänge (gemeinsam mit der Oberfläche, siehe `nrznews_types`) und eine Obergrenze.
pub fn check_strength(password: &str) -> AppResult<()> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(AppError::bad(format!(
            "Passwort: mindestens {MIN_PASSWORD_LEN} Zeichen."
        )));
    }
    if len > MAX_LENGTH {
        return Err(AppError::bad(format!(
            "Passwort: höchstens {MAX_LENGTH} Zeichen."
        )));
    }
    Ok(())
}

pub async fn hash(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
    })
    .await
    .map_err(|e| AppError::Config(format!("Hash-Thread: {e}")))?
    .map_err(|e| AppError::Config(format!("Argon2: {e}")))
}

pub async fn verify(password: String, hash: String) -> bool {
    // Überlange Eingaben gar nicht erst hashen (kein Rechenzeit-Angriff über die Anmeldung)
    if password.len() > MAX_LENGTH * 4 {
        return false;
    }
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn hash_and_verify() {
        let h = super::hash("korrekt-pferd".into()).await.unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(super::verify("korrekt-pferd".into(), h.clone()).await);
        assert!(!super::verify("falsch".into(), h).await);
        assert!(!super::verify("x".into(), "kein hash".into()).await);
    }

    #[test]
    fn strength() {
        assert!(super::check_strength("kurz").is_err());
        assert!(super::check_strength("lang-genug").is_ok());
        assert!(super::check_strength(&"x".repeat(super::MAX_LENGTH + 1)).is_err());
    }
}

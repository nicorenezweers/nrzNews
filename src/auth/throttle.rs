//! Bremse gegen Passwort-Raten: fehlgeschlagene Anmeldungen je Benutzername und IP-Adresse
//! im Speicher zählen. Nach [`MAX_FAILURES`] Fehlversuchen in [`WINDOW_SECS`] antwortet die
//! Anmeldung mit 429, bis das Zeitfenster abgelaufen ist. Eine erfolgreiche Anmeldung setzt
//! den Zähler zurück.
//!
//! Die IP kommt ohne Proxy aus `X-Real-IP`, wenn vorhanden (Rockets `ip_header`) – die kann
//! ein Angreifer selbst setzen. Darum gilt zusätzlich eine Grenze je Benutzername über alle
//! Adressen ([`MAX_FAILURES_PER_NAME`]).
//!
//! Bewusst nur im Speicher: Ein Neustart hebt die Sperre auf, aber es braucht keine Tabelle.
//! Ein Angreifer kann niemanden dauerhaft aussperren – höchstens für das Zeitfenster, und
//! je Konto erst nach deutlich mehr Versuchen als je Adresse.

use crate::db::now;
use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::sync::Mutex;

pub const MAX_FAILURES: usize = 10;
/// Fehlversuche je Benutzername über alle IP-Adressen zusammen.
pub const MAX_FAILURES_PER_NAME: usize = 50;
pub const WINDOW_SECS: i64 = 15 * 60;
/// Obergrenze für die Tabelle – gegen Speicherverbrauch durch erfundene Namen.
const MAX_ENTRIES: usize = 10_000;
/// Platzhalter-IP für den Zähler je Benutzername.
const ANY_IP: &str = "*";

/// Fehlversuche je (Benutzername klein geschrieben, IP-Adresse); als Rocket-State verwaltet.
#[derive(Default)]
pub struct LoginThrottle {
    failures: Mutex<HashMap<(String, String), Vec<i64>>>,
}

/// Benutzernamen sind höchstens 40 Zeichen lang; längere Eingaben werden gekürzt, damit
/// niemand mit riesigen Namen Speicher füllt.
fn key(username: &str, ip: &str) -> (String, String) {
    let name: String = username.trim().chars().take(64).collect();
    (name.to_lowercase(), ip.chars().take(64).collect())
}

impl LoginThrottle {
    /// Vor dem Prüfen des Passworts: noch erlaubt?
    pub fn check(&self, username: &str, ip: &str) -> AppResult<()> {
        let ts = now();
        let cutoff = ts - WINDOW_SECS;
        let Ok(mut map) = self.failures.lock() else {
            return Ok(());
        };
        for (k, limit) in [
            (key(username, ip), MAX_FAILURES),
            (key(username, ANY_IP), MAX_FAILURES_PER_NAME),
        ] {
            let Some(times) = map.get_mut(&k) else {
                continue;
            };
            times.retain(|t| *t > cutoff);
            if times.len() >= limit {
                let oldest = times.iter().min().copied().unwrap_or(ts);
                let minutes = ((oldest + WINDOW_SECS - ts + 59) / 60).max(1);
                let wait = if minutes == 1 {
                    "einer Minute".to_string()
                } else {
                    format!("{minutes} Minuten")
                };
                return Err(AppError::TooManyRequests(format!(
                    "Zu viele fehlgeschlagene Anmeldungen. Bitte in {wait} erneut versuchen."
                )));
            }
        }
        Ok(())
    }

    /// Falsches Passwort (oder unbekannter Name) merken.
    pub fn failed(&self, username: &str, ip: &str) {
        let ts = now();
        let Ok(mut map) = self.failures.lock() else {
            return;
        };
        if map.len() >= MAX_ENTRIES {
            let cutoff = ts - WINDOW_SECS;
            map.retain(|_, times| times.iter().any(|t| *t > cutoff));
        }
        for k in [key(username, ip), key(username, ANY_IP)] {
            if map.len() < MAX_ENTRIES || map.contains_key(&k) {
                map.entry(k).or_default().push(ts);
            }
        }
    }

    /// Erfolgreich angemeldet: Zähler zurücksetzen.
    pub fn succeeded(&self, username: &str, ip: &str) {
        if let Ok(mut map) = self.failures.lock() {
            map.remove(&key(username, ip));
            map.remove(&key(username, ANY_IP));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_failures_per_name_and_ip() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            assert!(t.check("Alice", "1.2.3.4").is_ok());
            t.failed("alice", "1.2.3.4");
        }
        assert!(matches!(
            t.check("ALICE", "1.2.3.4"),
            Err(AppError::TooManyRequests(_))
        ));
        // Andere IP und anderer Name sind nicht betroffen
        assert!(t.check("alice", "5.6.7.8").is_ok());
        assert!(t.check("bob", "1.2.3.4").is_ok());
        t.succeeded("alice", "1.2.3.4");
        assert!(t.check("alice", "1.2.3.4").is_ok());
    }

    #[test]
    fn blocks_a_name_attacked_from_many_addresses() {
        let t = LoginThrottle::default();
        for i in 0..MAX_FAILURES_PER_NAME {
            let ip = format!("10.0.{}.{}", i / 250, i % 250);
            assert!(t.check("alice", &ip).is_ok());
            t.failed("alice", &ip);
        }
        assert!(t.check("alice", "192.168.1.1").is_err());
        assert!(t.check("bob", "192.168.1.1").is_ok());
    }

    #[test]
    fn long_names_are_truncated() {
        let t = LoginThrottle::default();
        t.failed(&"x".repeat(100_000), "1.2.3.4");
        let map = t.failures.lock().unwrap();
        assert!(map.keys().all(|(name, _)| name.len() <= 64));
    }
}

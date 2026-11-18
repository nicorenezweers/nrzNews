//! Feeds abrufen, lesen (RSS, Atom, JSON Feed), auf Webseiten finden, OPML, HTML säubern.
//!
//! * [`fetch`] – ein gemeinsamer HTTP-Client, bedingtes Abrufen, Grössenlimit
//! * [`parse`] – feed-rs → eigenes Modell (IDs, Daten, Bilder, Podcasts)
//! * [`sanitize`] – HTML säubern (ammonia), reiner Text, Lesezeit, Auszug
//! * [`discover`] – Feeds und Icon einer Webseite finden
//! * [`opml`] – Abos als OPML lesen und schreiben
//! * [`refresher`] – Abrufen im Hintergrund, Speichern der Artikel, Aufräumen

pub mod discover;
pub mod fetch;
pub mod opml;
pub mod parse;
pub mod refresher;
pub mod sanitize;

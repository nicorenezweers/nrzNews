-- nrzRSS: Benutzer, Sitzungen, Einstellungen, Ordner, Feeds, Artikel (+ Volltextindex).
-- Feeds gehören je einem Benutzer (wie bei Miniflux): jeder hat seine eigenen Abos,
-- Gelesen/Gemerkt steht direkt am Artikel. Zeitpunkte sind Unix-Sekunden (UTC).

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    display_name  TEXT    NOT NULL DEFAULT '',
    password_hash TEXT    NOT NULL,
    role          TEXT    NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
    disabled      INTEGER NOT NULL DEFAULT 0,
    -- JSON (nrzrss_types::Preferences)
    preferences   TEXT    NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    last_login_at INTEGER
);

CREATE TABLE sessions (
    -- SHA-256 (hex) des Tokens im Cookie; das Token selbst steht nirgends
    token_hash  TEXT    PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    user_agent  TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX sessions_user ON sessions (user_id);

-- Instanzweite Einstellungen als JSON (registration, ai, cookie_secret, …)
CREATE TABLE app_settings (
    key        TEXT    PRIMARY KEY,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE folders (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (user_id, name)
);

CREATE TABLE feeds (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    folder_id       INTEGER REFERENCES folders (id) ON DELETE SET NULL,
    -- Angezeigter Name (vom Benutzer änderbar)
    title           TEXT    NOT NULL DEFAULT '',
    -- Titel laut Feed (für "Name zurücksetzen")
    original_title  TEXT    NOT NULL DEFAULT '',
    feed_url        TEXT    NOT NULL,
    site_url        TEXT    NOT NULL DEFAULT '',
    description     TEXT    NOT NULL DEFAULT '',
    icon_url        TEXT    NOT NULL DEFAULT '',
    -- Bedingtes Abrufen (304 Not Modified)
    etag            TEXT,
    last_modified   TEXT,
    last_fetched_at INTEGER,
    next_fetch_at   INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    error_count     INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    UNIQUE (user_id, feed_url)
);
CREATE INDEX feeds_user ON feeds (user_id);
CREATE INDEX feeds_next_fetch ON feeds (next_fetch_at);

CREATE TABLE articles (
    id               INTEGER PRIMARY KEY,
    feed_id          INTEGER NOT NULL REFERENCES feeds (id) ON DELETE CASCADE,
    -- doppelt zum Feed, damit Listen ohne Join filtern können
    user_id          INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- ID des Eintrags laut Feed (sonst Link, sonst Hash aus Titel und Datum)
    guid             TEXT    NOT NULL,
    url              TEXT    NOT NULL DEFAULT '',
    title            TEXT    NOT NULL DEFAULT '',
    author           TEXT    NOT NULL DEFAULT '',
    -- gesäubert mit ammonia
    content_html     TEXT    NOT NULL DEFAULT '',
    -- reiner Text: Suche, Auszug, KI
    content_text     TEXT    NOT NULL DEFAULT '',
    image_url        TEXT    NOT NULL DEFAULT '',
    published_at     INTEGER NOT NULL,
    fetched_at       INTEGER NOT NULL,
    read             INTEGER NOT NULL DEFAULT 0,
    read_at          INTEGER,
    starred          INTEGER NOT NULL DEFAULT 0,
    starred_at       INTEGER,
    ai_summary       TEXT,
    ai_summary_model TEXT,
    UNIQUE (feed_id, guid)
);
CREATE INDEX articles_user_published ON articles (user_id, published_at DESC, id DESC);
CREATE INDEX articles_user_unread ON articles (user_id, read, published_at DESC);
CREATE INDEX articles_feed_published ON articles (feed_id, published_at DESC, id DESC);
CREATE INDEX articles_user_starred ON articles (user_id, published_at DESC) WHERE starred = 1;

-- Volltextsuche über Titel und Text, Umlaute/Akzente gefaltet
CREATE VIRTUAL TABLE articles_fts USING fts5 (
    title,
    content_text,
    content = 'articles',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER articles_fts_insert AFTER INSERT ON articles BEGIN
    INSERT INTO articles_fts (rowid, title, content_text) VALUES (new.id, new.title, new.content_text);
END;

CREATE TRIGGER articles_fts_delete AFTER DELETE ON articles BEGIN
    INSERT INTO articles_fts (articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
END;

CREATE TRIGGER articles_fts_update AFTER UPDATE OF title, content_text ON articles BEGIN
    INSERT INTO articles_fts (articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
    INSERT INTO articles_fts (rowid, title, content_text) VALUES (new.id, new.title, new.content_text);
END;

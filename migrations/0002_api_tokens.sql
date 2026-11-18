-- Persönliche API-Tokens für den MCP-Server unter /mcp (z. B. für nrzDashboard). Das Token
-- selbst steht nirgends, nur sein SHA-256 (hex) – wie bei den Sitzungen. Diese Datei nach
-- dem Release nie mehr ändern (sqlx prüft die Prüfsumme), sondern neue Migrationen anlegen.
CREATE TABLE api_tokens (
    id           INTEGER PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL UNIQUE,
    -- Anfang des Tokens zum Wiedererkennen in der Liste (z. B. "nrznews_AbC1")
    prefix       TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

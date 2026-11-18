//! Migrationen laufen auf einer leeren Datenbank, die Volltextsuche (FTS5) folgt den Artikeln.

use nrznews::config::Config;

#[tokio::test]
async fn migrations_and_fts() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let pool = nrznews::db::connect(&config).await.unwrap();
    sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES ('a', 'x', 0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO feeds (user_id, feed_url, created_at) VALUES (1, 'https://example.org/feed', 0)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO articles (feed_id, user_id, guid, title, content_text, published_at, fetched_at)
                 VALUES (1, 1, 'g1', 'Grüne Äpfel', 'Rust und Rocket', 0, 0)")
        .execute(&pool).await.unwrap();
    let hits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM articles_fts WHERE articles_fts MATCH 'apfel*'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(hits, 1);
    pool.close().await;
}

/// SHA-384 der ersten Migration, wie sie seit 1.0.0 in jeder Datenbank steht.
const MIGRATION_1_CHECKSUM: &str = "6943ddaf4e3d03c76c9550ffeada81d26e522319cd670e95c55a49dd3a283d59a53853ecb350c17b8de2adbf4bf00df5";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Angewendete Migrationen dürfen sich nie ändern (auch keine Kommentare) – sonst startet der
/// Server mit bestehenden Datenbanken nicht mehr („previously applied but has been modified“).
#[test]
fn applied_migrations_are_unchanged() {
    let released = [
        (1, MIGRATION_1_CHECKSUM),
        // 1.2.0: API-Tokens
        (2, "d3f84a1759effb16bb5335959068d194dfae9b1d0f1de102e2594b71a384d86ff4fca72a0c4a7b1a17e51483d13c5d67"),
    ];
    let migrator = nrznews::db::migrations::migrator();
    for (version, checksum) in released {
        let m = migrator.iter().find(|m| m.version == version).unwrap();
        assert_eq!(
            hex(&m.checksum),
            checksum,
            "Migration {version} wurde geändert"
        );
    }
}

/// Datenbank von nrzRSS 1.0.0 (`nrzrss.db`) und eine, die nrzNews 1.1.0 mit falscher Prüfsumme
/// angelegt hat: beide öffnen sich.
#[tokio::test]
async fn databases_from_older_versions_open() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let pool = nrznews::db::connect(&config).await.unwrap();
    sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES ('alt', 'x', 0)")
        .execute(&pool)
        .await
        .unwrap();
    // Prüfsumme wie bei einer mit 1.1.0 angelegten Datenbank
    sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1")
        .bind(
            (0..96)
                .step_by(2)
                .map(|i| u8::from_str_radix(&"ceb70adc51bf64ed2f01ebdb92e7fea220d30bcf1ed294c53f340e5c34529f74a43676cac4fdb094d0332436ceb6dbd4"[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        )
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    // Unter dem alten Namen
    std::fs::rename(dir.path().join("nrznews.db"), dir.path().join("nrzrss.db")).unwrap();
    assert_eq!(config.database_path(), dir.path().join("nrzrss.db"));

    let pool = nrznews::db::connect(&config).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT username FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "alt");
    let checksum: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(hex(&checksum), MIGRATION_1_CHECKSUM);
    pool.close().await;
}

//! Ordner eines Benutzers: eine Ebene, alphabetisch nach Position. Ein gelöschter Ordner
//! nimmt seine Feeds nicht mit – sie stehen danach „ohne Ordner“ (`ON DELETE SET NULL`).

use crate::db::now;
use crate::error::{AppError, AppResult};
use nrznews_types::Folder;
use sqlx::SqlitePool;

/// Ordner mit Zahl der ungelesenen Artikel (als Makro, damit `concat!` statische SQL ergibt).
macro_rules! select {
    () => {
        "SELECT f.id, f.name, f.position,
            (SELECT count(*) FROM articles a JOIN feeds d ON d.id = a.feed_id
             WHERE d.folder_id = f.id AND a.read = 0) AS unread
         FROM folders f"
    };
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    name: String,
    position: i64,
    unread: i64,
}

impl From<Row> for Folder {
    fn from(r: Row) -> Self {
        Folder {
            id: r.id,
            name: r.name,
            position: r.position,
            unread: r.unread,
        }
    }
}

/// Name prüfen: nicht leer, nicht zu lang, Leerraum zusammengefasst.
fn clean_name(name: &str) -> AppResult<String> {
    let name = crate::feeds::sanitize::collapse(name);
    if name.is_empty() {
        return Err(AppError::bad("Bitte einen Namen für den Ordner angeben."));
    }
    if name.chars().count() > 100 {
        return Err(AppError::bad(
            "Der Name ist zu lang (höchstens 100 Zeichen).",
        ));
    }
    Ok(name)
}

pub async fn list(pool: &SqlitePool, user_id: i64) -> AppResult<Vec<Folder>> {
    let rows: Vec<Row> = sqlx::query_as(concat!(
        select!(),
        " WHERE f.user_id = ? ORDER BY f.position, f.name COLLATE NOCASE, f.id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Folder::from).collect())
}

pub async fn get(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<Folder> {
    let row: Option<Row> = sqlx::query_as(concat!(select!(), " WHERE f.user_id = ? AND f.id = ?"))
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(Folder::from).ok_or(AppError::NotFound)
}

/// Gehört der Ordner dem Benutzer? (für Feeds, die in einen Ordner sollen)
pub async fn ensure_owned(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<()> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM folders WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    found
        .map(|_| ())
        .ok_or_else(|| AppError::bad("Diesen Ordner gibt es nicht."))
}

pub async fn create(pool: &SqlitePool, user_id: i64, name: &str) -> AppResult<Folder> {
    let name = clean_name(name)?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (user_id, name, position, created_at)
         VALUES (?, ?, (SELECT coalesce(max(position), -1) + 1 FROM folders WHERE user_id = ?), ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(&name)
    .bind(user_id)
    .bind(now())
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::unique(e, "Diesen Ordner gibt es schon."))?;
    get(pool, user_id, id).await
}

pub async fn rename(pool: &SqlitePool, user_id: i64, id: i64, name: &str) -> AppResult<Folder> {
    let name = clean_name(name)?;
    let r = sqlx::query("UPDATE folders SET name = ? WHERE id = ? AND user_id = ?")
        .bind(&name)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::unique(e, "Diesen Ordner gibt es schon."))?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    get(pool, user_id, id).await
}

pub async fn delete(pool: &SqlitePool, user_id: i64, id: i64) -> AppResult<()> {
    let r = sqlx::query("DELETE FROM folders WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Ordner mit diesem Namen (ohne Gross/Klein) – oder neu anlegen. `true` = neu angelegt.
pub async fn find_or_create(pool: &SqlitePool, user_id: i64, name: &str) -> AppResult<(i64, bool)> {
    let name = clean_name(name)?;
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM folders WHERE user_id = ? AND name = ? COLLATE NOCASE ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(&name)
    .fetch_optional(pool)
    .await?;
    if let Some(id) = found {
        return Ok((id, false));
    }
    Ok((create(pool, user_id, &name).await?.id, true))
}

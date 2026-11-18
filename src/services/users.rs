//! Benutzerkonten: anlegen, anmelden, eigene Einstellungen, Verwaltung durch Admins.
//!
//! Die API-Struktur [`User`] kennt den Passwort-Hash nicht; intern liest [`UserRow`] die
//! ganze Zeile und gibt nach aussen nur `User` weiter – so kann der Hash nie versehentlich
//! in einer Antwort landen.

use crate::auth::{password, session};
use crate::config::Config;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::services::settings;
use nrznews_types::{Preferences, Role, UpdateMeInput, UpdateUserInput, User};
use sqlx::SqlitePool;
use std::time::Duration;

/// `SELECT … FROM users <rest>` als feste Zeichenkette (sqlx verlangt statisches SQL).
macro_rules! select_users {
    ($rest:literal) => {
        concat!(
            "SELECT id, username, display_name, password_hash, role, disabled, preferences, \
             created_at, last_login_at FROM users ",
            $rest
        )
    };
}

/// Höchstlänge des Anzeigenamens.
const MAX_DISPLAY_NAME: usize = 80;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i64,
    username: String,
    display_name: String,
    password_hash: String,
    role: String,
    disabled: bool,
    preferences: String,
    created_at: i64,
    last_login_at: Option<i64>,
}

impl UserRow {
    fn into_user(self) -> User {
        User {
            id: self.id,
            username: self.username,
            display_name: self.display_name,
            role: parse_role(&self.role),
            disabled: self.disabled,
            created_at: self.created_at,
            last_login_at: self.last_login_at,
            // Fehlende Felder bekommen ihre Vorgabe (`#[serde(default)]`); kaputtes JSON auch.
            preferences: serde_json::from_str(&self.preferences).unwrap_or_default(),
        }
    }
}

fn parse_role(s: &str) -> Role {
    if s == "admin" {
        Role::Admin
    } else {
        Role::Member
    }
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

async fn row(pool: &SqlitePool, id: i64) -> AppResult<Option<UserRow>> {
    Ok(sqlx::query_as(select_users!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

/// Benutzername vergleicht ohne Gross-/Kleinschreibung (`COLLATE NOCASE` in der Tabelle).
async fn row_by_username(pool: &SqlitePool, username: &str) -> AppResult<Option<UserRow>> {
    Ok(sqlx::query_as(select_users!("WHERE username = ?"))
        .bind(username.trim())
        .fetch_optional(pool)
        .await?)
}

pub async fn count(pool: &SqlitePool) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await?)
}

/// Aktive (nicht gesperrte) Administratoren – davon muss immer mindestens einer bleiben.
async fn active_admins(pool: &SqlitePool) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin' AND disabled = 0")
            .fetch_one(pool)
            .await?,
    )
}

pub async fn find(pool: &SqlitePool, id: i64) -> AppResult<Option<User>> {
    Ok(row(pool, id).await?.map(UserRow::into_user))
}

/// Benutzer mit dieser ID – oder 404.
pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<User> {
    find(pool, id).await?.ok_or(AppError::NotFound)
}

pub async fn list(pool: &SqlitePool) -> AppResult<Vec<User>> {
    let rows: Vec<UserRow> = sqlx::query_as(select_users!("ORDER BY created_at, id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(UserRow::into_user).collect())
}

/// Darf sich gerade jemand selbst registrieren? Das erste Konto immer, danach nur, wenn die
/// Registrierung offen ist (Einstellung des Admins, sonst `NRZNEWS_REGISTRATION_OPEN`).
pub async fn registration_allowed(pool: &SqlitePool, config: &Config) -> AppResult<bool> {
    if count(pool).await? == 0 {
        return Ok(true);
    }
    Ok(settings::instance(pool, config).await?.registration_open)
}

/// 2–40 Zeichen aus `a-z A-Z 0-9 . _ -` – taugt so auch in Pfaden und Protokollen.
pub fn validate_username(name: &str) -> AppResult<String> {
    let name = name.trim();
    let ok = (2..=40).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        return Err(AppError::bad(
            "Benutzername: 2–40 Zeichen, nur Buchstaben (ohne Umlaute), Zahlen, Punkt, Minus und Unterstrich.",
        ));
    }
    Ok(name.to_string())
}

fn validate_display_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.chars().count() > MAX_DISPLAY_NAME || name.chars().any(char::is_control) {
        return Err(AppError::bad(format!(
            "Anzeigename: höchstens {MAX_DISPLAY_NAME} Zeichen, ohne Steuerzeichen."
        )));
    }
    Ok(name.to_string())
}

pub fn validate_preferences(p: &Preferences) -> AppResult<()> {
    if !["de", "en"].contains(&p.language.as_str()) {
        return Err(AppError::bad("Sprache: „de“ oder „en“."));
    }
    if !["s", "m", "l"].contains(&p.reader_size.as_str()) {
        return Err(AppError::bad("Schriftgrösse: „s“, „m“ oder „l“."));
    }
    Ok(())
}

/// Legt ein Konto an. Benutzernamen sind eindeutig (ohne Gross-/Kleinschreibung) → 409.
pub async fn create(
    pool: &SqlitePool,
    username: &str,
    password_plain: &str,
    display_name: &str,
    role: Role,
) -> AppResult<User> {
    let username = validate_username(username)?;
    let display_name = validate_display_name(display_name)?;
    password::check_strength(password_plain)?;
    let hash = password::hash(password_plain.to_string()).await?;
    let preferences =
        serde_json::to_string(&Preferences::default()).unwrap_or_else(|_| "{}".into());
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, display_name, password_hash, role, preferences, created_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&username)
    .bind(&display_name)
    .bind(&hash)
    .bind(role_str(role))
    .bind(preferences)
    .bind(now())
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::unique(e, "Dieser Benutzername ist schon vergeben."))?;
    get(pool, id).await
}

/// Anmeldedaten prüfen. Auch bei unbekanntem Namen wird ein Hash geprüft, damit die
/// Antwortzeit nicht verrät, ob es das Konto gibt; die Meldung ist in beiden Fällen dieselbe.
/// Gesperrte Konten erfahren erst nach richtigem Passwort, dass sie gesperrt sind.
pub async fn authenticate(
    pool: &SqlitePool,
    username: &str,
    password_plain: &str,
) -> AppResult<User> {
    let row = row_by_username(pool, username).await?;
    let hash = row
        .as_ref()
        .map(|r| r.password_hash.clone())
        .unwrap_or_else(|| password::DUMMY_HASH.clone());
    let ok = password::verify(password_plain.to_string(), hash).await;
    match row {
        Some(r) if ok && r.disabled => Err(AppError::forbidden(
            "Dieses Konto ist gesperrt – bitte an einen Administrator wenden.",
        )),
        Some(r) if ok => Ok(r.into_user()),
        _ => Err(AppError::InvalidCredentials),
    }
}

/// Zeitpunkt der letzten Anmeldung festhalten.
pub async fn record_login(pool: &SqlitePool, id: i64) -> AppResult<User> {
    sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    get(pool, id).await
}

/// Eigenen Anzeigenamen und eigene Einstellungen ändern (nur gesetzte Felder).
pub async fn update_me(pool: &SqlitePool, id: i64, input: &UpdateMeInput) -> AppResult<User> {
    if let Some(name) = &input.display_name {
        let name = validate_display_name(name)?;
        sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(prefs) = &input.preferences {
        validate_preferences(prefs)?;
        let json = serde_json::to_string(prefs).map_err(|e| AppError::bad(e.to_string()))?;
        sqlx::query("UPDATE users SET preferences = ? WHERE id = ?")
            .bind(json)
            .bind(id)
            .execute(pool)
            .await?;
    }
    get(pool, id).await
}

/// Eigenes Passwort ändern – nur mit dem bisherigen. Die anderen Sitzungen beendet der
/// Aufrufer (er kennt das Token der aktuellen).
pub async fn change_password(
    pool: &SqlitePool,
    id: i64,
    current: &str,
    new: &str,
) -> AppResult<()> {
    let row = row(pool, id).await?.ok_or(AppError::NotFound)?;
    if !password::verify(current.to_string(), row.password_hash).await {
        // Bremst Durchprobieren mit einer gestohlenen Sitzung.
        tokio::time::sleep(Duration::from_millis(400)).await;
        return Err(AppError::bad("Das aktuelle Passwort stimmt nicht."));
    }
    password::check_strength(new)?;
    set_password(pool, id, new).await
}

async fn set_password(pool: &SqlitePool, id: i64, new: &str) -> AppResult<()> {
    let hash = password::hash(new.to_string()).await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Ein Admin ändert ein Konto (nur gesetzte Felder). Sich selbst kann er weder herabstufen
/// noch sperren, und es muss immer ein aktiver Admin bleiben. Ein neues Passwort oder eine
/// Sperre beendet alle Sitzungen des Kontos.
pub async fn update(
    pool: &SqlitePool,
    actor: &User,
    id: i64,
    input: &UpdateUserInput,
) -> AppResult<User> {
    let target = get(pool, id).await?;
    let is_self = target.id == actor.id;
    let demote = target.is_admin() && input.role == Some(Role::Member);
    let disable = !target.disabled && input.disabled == Some(true);
    // Leeres Passwortfeld im Formular = Passwort nicht ändern
    let new_password = input.password.as_deref().filter(|p| !p.is_empty());

    if is_self && demote {
        return Err(AppError::bad(
            "Du kannst dir die Administratorrechte nicht selbst entziehen.",
        ));
    }
    if is_self && disable {
        return Err(AppError::bad("Du kannst dein eigenes Konto nicht sperren."));
    }
    if is_self && new_password.is_some() {
        return Err(AppError::bad(
            "Das eigene Passwort bitte unter „Konto“ ändern (mit dem bisherigen Passwort).",
        ));
    }
    if target.is_admin()
        && !target.disabled
        && (demote || disable)
        && active_admins(pool).await? <= 1
    {
        return Err(AppError::Conflict(
            "Es muss mindestens ein aktiver Administrator bleiben.".into(),
        ));
    }

    let display_name = input
        .display_name
        .as_deref()
        .map(validate_display_name)
        .transpose()?;
    if let Some(p) = new_password {
        password::check_strength(p)?;
    }

    if let Some(name) = display_name {
        sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(role) = input.role {
        sqlx::query("UPDATE users SET role = ? WHERE id = ?")
            .bind(role_str(role))
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(disabled) = input.disabled {
        sqlx::query("UPDATE users SET disabled = ? WHERE id = ?")
            .bind(disabled)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(p) = new_password {
        set_password(pool, id, p).await?;
    }
    if new_password.is_some() || disable {
        session::destroy_all(pool, id).await?;
    }
    get(pool, id).await
}

/// Konto mit allen Daten löschen (Ordner, Feeds, Artikel, Sitzungen). Nicht sich selbst und
/// nicht den letzten aktiven Admin.
pub async fn delete(pool: &SqlitePool, actor: &User, id: i64) -> AppResult<()> {
    let target = get(pool, id).await?;
    if target.id == actor.id {
        return Err(AppError::bad("Du kannst dein eigenes Konto nicht löschen."));
    }
    if target.is_admin() && !target.disabled && active_admins(pool).await? <= 1 {
        return Err(AppError::Conflict(
            "Es muss mindestens ein aktiver Administrator bleiben.".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    // Artikel ausdrücklich zuerst: so laufen die Trigger des Volltextindex sicher mit.
    sqlx::query("DELETE FROM articles WHERE user_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // Der Rest folgt über ON DELETE CASCADE (Ordner, Feeds, Sitzungen).
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(user = %target.username, by = %actor.username, "Konto gelöscht");
    Ok(())
}

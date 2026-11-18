//! SQLite persistence, schema versioning, and the JSON import path.
//!
//! The database is the source of truth (`docs/spec/domain-model.md` §12).
//! Writes are transactional, migrations are forward-only, and a database from
//! a newer build is refused rather than opened with unknown columns. The
//! previous build stored one JSON document; [`Store::open`] imports it once,
//! keeps the JSON as a timestamped backup, and records the migration in
//! `audit_log` (`A-DATA-01`).
//!
//! Profiles and commitments are stored as their serialized domain value so a
//! round trip cannot lose a field. `rules`, `schedules`, and the usage tables
//! are projections written in the same transaction: they are queryable and
//! auditable, while the serialized value remains the input to the engine.
//! `challenges`, `saved_qr`, `breaks`, and `usage_hourly` from the target schema
//! are not created until the features that fill them exist; see
//! `docs/spec/implementation-notes.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;

use crate::app::AppKey;
use crate::domain::{QuickBlock, Target};
use crate::error::{Error, Result};
use crate::period::ClockSettings;
use crate::time::Timestamp;
use crate::usage::{BlockRecord, LaunchRecord, UsageInterval, UsageLedger};

/// Current database schema. Bump it and add a forward step in [`migrate`].
pub const SCHEMA_VERSION: i64 = 1;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A wrapper from the previous, JSON-based build.
#[derive(Debug, Deserialize)]
struct LegacyState {
    version: i64,
    state: crate::engine::State,
}

/// One opened database.
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

/// Outcome of opening a store, so callers can report a migration.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The loaded state.
    pub state: crate::engine::State,
    /// The schema version now in use.
    pub schema_version: i64,
    /// The JSON file that was imported, if this was a migration.
    pub imported_from: Option<PathBuf>,
}

impl Store {
    /// Open or create the database at `path`.
    ///
    /// `legacy_json` is the old state file. It is imported only when the
    /// database is new; after a successful import it is renamed to a
    /// timestamped backup so an old copy can never be imported twice.
    pub fn open(
        path: impl Into<PathBuf>,
        legacy_json: Option<&Path>,
        initial: crate::engine::State,
    ) -> Result<(Self, Opened)> {
        let path = path.into();
        create_private_parent(&path)?;
        let conn = Connection::open(&path).map_err(storage)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(storage)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(storage)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(storage)?;
        set_private(&path);

        let mut store = Self { conn, path };
        let version = store.current_version()?;
        if version > SCHEMA_VERSION {
            return Err(Error::Storage(format!(
                "database schema {version} is newer than this build supports ({SCHEMA_VERSION})"
            )));
        }
        let imported_from = if version == 0 {
            store.migrate_from_json(legacy_json, initial)?
        } else {
            None
        };
        if version > 0 && version < SCHEMA_VERSION {
            store.backup(version)?;
            let tx = store.conn.transaction().map_err(storage)?;
            migrate(&tx, version)?;
            tx.commit().map_err(storage)?;
        }
        let state = store.load()?;
        set_private(store.path());
        Ok((
            store,
            Opened {
                state,
                schema_version: SCHEMA_VERSION,
                imported_from,
            },
        ))
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write `state` in one transaction.
    pub fn save(&mut self, state: &crate::engine::State) -> Result<()> {
        let tx = self.conn.transaction().map_err(storage)?;
        write_state(&tx, state)?;
        tx.commit().map_err(storage)?;
        set_private(&self.path);
        Ok(())
    }

    fn current_version(&self) -> Result<i64> {
        let exists: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if exists == 0 {
            return Ok(0);
        }
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .map_err(storage)
    }

    fn backup(&self, version: i64) -> Result<()> {
        let stamp = Utc::now().timestamp();
        let name = self
            .path
            .with_extension(format!("v{version}-{stamp}.backup"));
        fs::copy(&self.path, &name).map_err(|error| {
            Error::Storage(format!(
                "could not back up {}: {error}",
                self.path.display()
            ))
        })?;
        Ok(())
    }

    fn migrate_from_json(
        &mut self,
        legacy_json: Option<&Path>,
        initial: crate::engine::State,
    ) -> Result<Option<PathBuf>> {
        let (state, imported) = match legacy_json {
            Some(path) if path.is_file() => match read_legacy(path) {
                Ok(state) => (state, Some(path.to_path_buf())),
                Err(error) => {
                    // The old build also preserved invalid files instead of
                    // silently starting empty. Keep that behaviour.
                    eprintln!("stayproductive: state file is invalid: {error}");
                    (initial, None)
                }
            },
            _ => (initial, None),
        };
        let tx = self.conn.transaction().map_err(storage)?;
        create_schema(&tx)?;
        write_state(&tx, &state)?;
        let now = format_ts(Utc::now());
        tx.execute(
            "INSERT INTO schema_version(version, applied_at, note) VALUES (1, ?1, ?2)",
            params![
                now,
                if imported.is_some() {
                    "initial SQLite schema imported from state.json"
                } else {
                    "initial SQLite schema"
                }
            ],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO audit_log(at, action, detail) VALUES (?1, 'migration', ?2)",
            params![
                now,
                format!(
                    "schema 0 -> 1 ({})",
                    if imported.is_some() {
                        "json import"
                    } else {
                        "new database"
                    }
                )
            ],
        )
        .map_err(storage)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(storage)?;
        tx.commit().map_err(storage)?;

        if let Some(path) = &imported {
            let stamp = Utc::now().timestamp();
            let backup = path.with_extension(format!("migrated-{stamp}.json"));
            fs::rename(path, &backup).map_err(|error| {
                Error::Storage(format!("could not keep {}: {error}", path.display()))
            })?;
            return Ok(Some(backup));
        }
        Ok(None)
    }

    fn load(&self) -> Result<crate::engine::State> {
        let settings = self
            .conn
            .query_row(
                "SELECT start_of_day, timezone FROM settings WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(storage)?
            .map(|(start, timezone)| parse_settings(&start, &timezone))
            .transpose()?
            .unwrap_or_default();

        let mut state = crate::engine::State::new(settings);

        let mut statement = self
            .conn
            .prepare("SELECT payload FROM profiles ORDER BY rowid")
            .map_err(storage)?;
        let profiles = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage)?;
        for payload in profiles {
            state
                .profiles
                .push(parse_payload(&payload.map_err(storage)?)?);
        }

        let mut statement = self
            .conn
            .prepare("SELECT payload FROM commitments ORDER BY rowid")
            .map_err(storage)?;
        let commitments = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage)?;
        for payload in commitments {
            state
                .commitments
                .push(parse_payload(&payload.map_err(storage)?)?);
        }

        let mut statement = self
            .conn
            .prepare("SELECT target, kind, until, created_at FROM quick_blocks ORDER BY rowid")
            .map_err(storage)?;
        let blocks = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(storage)?;
        for block in blocks {
            let (target, kind, until, created_at) = block.map_err(storage)?;
            state.quick_blocks.push(QuickBlock {
                target: parse_target(&kind, &target)?,
                until: parse_ts(&until)?,
                created_at: parse_ts(&created_at)?,
            });
        }

        let mut statement = self
            .conn
            .prepare("SELECT profile_id, until FROM extensions")
            .map_err(storage)?;
        let extensions = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage)?;
        for extension in extensions {
            let (profile, until) = extension.map_err(storage)?;
            state
                .extensions
                .insert(parse_id(&profile)?, parse_ts(&until)?);
        }

        state.manual_active = self
            .meta("manual_active")?
            .map(|id| parse_id(&id))
            .transpose()?;
        state.screen_locked = self
            .meta("screen_locked")?
            .map(|value| value == "1")
            .unwrap_or(false);

        let mut intervals = Vec::new();
        let mut statement = self
            .conn
            .prepare("SELECT app, start, end FROM usage_events ORDER BY rowid")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(storage)?;
        for row in rows {
            let (app, start, end) = row.map_err(storage)?;
            intervals.push(UsageInterval {
                app: parse_app(&app)?,
                start: parse_ts(&start)?,
                end: parse_ts(&end)?,
            });
        }

        let mut launches = Vec::new();
        let mut statement = self
            .conn
            .prepare("SELECT app, at FROM usage_launches ORDER BY rowid")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage)?;
        for row in rows {
            let (app, at) = row.map_err(storage)?;
            launches.push(LaunchRecord {
                app: parse_app(&app)?,
                at: parse_ts(&at)?,
            });
        }

        let mut pickups = Vec::new();
        let mut statement = self
            .conn
            .prepare("SELECT at FROM usage_pickups ORDER BY rowid")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage)?;
        for row in rows {
            pickups.push(parse_ts(&row.map_err(storage)?)?);
        }

        let mut blocks = Vec::new();
        let mut statement = self
            .conn
            .prepare(
                "SELECT at, target, kind, reason, until, profile FROM usage_blocks ORDER BY rowid",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(storage)?;
        for row in rows {
            let (at, target, kind, reason, until, profile) = row.map_err(storage)?;
            blocks.push(BlockRecord {
                at: parse_ts(&at)?,
                target: parse_target(&kind, &target)?,
                reason: serde_json::from_value(serde_json::Value::String(reason))
                    .map_err(|error| Error::Storage(error.to_string()))?,
                until: until.map(|value| parse_ts(&value)).transpose()?,
                profile: profile.map(|value| parse_id(&value)).transpose()?,
            });
        }

        let usage_meta = self
            .conn
            .query_row(
                "SELECT open_app, open_start, last_exit, last_heartbeat FROM usage_meta WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(storage)?;
        let (open, last_exit, last_heartbeat) = match usage_meta {
            Some((app, start, last_exit, heartbeat)) => {
                let open = match (app, start) {
                    (Some(app), Some(start)) => Some((parse_app(&app)?, parse_ts(&start)?)),
                    _ => None,
                };
                let last_exit: BTreeMap<AppKey, Timestamp> =
                    serde_json::from_str(&last_exit).map_err(json_error)?;
                (
                    open,
                    last_exit,
                    heartbeat.map(|value| parse_ts(&value)).transpose()?,
                )
            }
            None => (None, BTreeMap::new(), None),
        };
        state.usage = UsageLedger::from_persisted(
            intervals,
            open,
            launches,
            pickups,
            blocks,
            last_exit,
            last_heartbeat,
        );

        Ok(state)
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(storage)
    }
}

fn create_schema(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL,
            note TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            start_of_day TEXT NOT NULL,
            timezone TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL,
            priority INTEGER NOT NULL,
            activation TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS profiles_name ON profiles(name);
        CREATE TABLE IF NOT EXISTS rules (
            profile_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            target TEXT NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY (profile_id, kind, target)
        );
        CREATE TABLE IF NOT EXISTS schedules (
            profile_id TEXT PRIMARY KEY,
            payload TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS commitments (
            id TEXT PRIMARY KEY,
            state TEXT NOT NULL,
            starts_at TEXT NOT NULL,
            ends_at TEXT,
            updated_at TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS quick_blocks (
            target TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            until TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS extensions (
            profile_id TEXT PRIMARY KEY,
            until TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            start TEXT NOT NULL,
            end TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_events_start ON usage_events(start);
        CREATE TABLE IF NOT EXISTS usage_launches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_pickups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_blocks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            target TEXT NOT NULL,
            kind TEXT NOT NULL,
            reason TEXT NOT NULL,
            until TEXT,
            profile TEXT
        );
        CREATE TABLE IF NOT EXISTS usage_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            open_app TEXT,
            open_start TEXT,
            last_exit TEXT NOT NULL,
            last_heartbeat TEXT
        );
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            action TEXT NOT NULL,
            detail TEXT NOT NULL
        );
        "#,
    )
    .map_err(storage)
}

fn migrate(tx: &Transaction<'_>, from: i64) -> Result<()> {
    let mut version = from;
    while version < SCHEMA_VERSION {
        match version {
            // Each future step goes here, inside the caller's transaction.
            0 => create_schema(tx)?,
            other => {
                return Err(Error::Storage(format!(
                    "no migration step from schema {other}"
                )));
            }
        }
        version += 1;
        let now = format_ts(Utc::now());
        tx.execute(
            "INSERT INTO schema_version(version, applied_at, note) VALUES (?1, ?2, ?3)",
            params![version, now, format!("migrated from schema {from}")],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO audit_log(at, action, detail) VALUES (?1, 'migration', ?2)",
            params![now, format!("schema {from} -> {version}")],
        )
        .map_err(storage)?;
        tx.pragma_update(None, "user_version", version)
            .map_err(storage)?;
    }
    Ok(())
}

fn write_state(tx: &Transaction<'_>, state: &crate::engine::State) -> Result<()> {
    tx.execute("DELETE FROM settings", []).map_err(storage)?;
    tx.execute("DELETE FROM rules", []).map_err(storage)?;
    tx.execute("DELETE FROM schedules", []).map_err(storage)?;
    tx.execute("DELETE FROM profiles", []).map_err(storage)?;
    tx.execute("DELETE FROM commitments", []).map_err(storage)?;
    tx.execute("DELETE FROM quick_blocks", [])
        .map_err(storage)?;
    tx.execute("DELETE FROM extensions", []).map_err(storage)?;
    tx.execute("DELETE FROM usage_events", [])
        .map_err(storage)?;
    tx.execute("DELETE FROM usage_launches", [])
        .map_err(storage)?;
    tx.execute("DELETE FROM usage_pickups", [])
        .map_err(storage)?;
    tx.execute("DELETE FROM usage_blocks", [])
        .map_err(storage)?;
    tx.execute("DELETE FROM meta", []).map_err(storage)?;
    tx.execute("DELETE FROM usage_meta", []).map_err(storage)?;

    tx.execute(
        "INSERT INTO settings(id, start_of_day, timezone) VALUES (1, ?1, ?2)",
        params![
            state.settings.start_of_day.format("%H:%M:%S").to_string(),
            state.settings.timezone.name()
        ],
    )
    .map_err(storage)?;

    for profile in &state.profiles {
        tx.execute(
            "INSERT INTO profiles(id, name, enabled, priority, activation, updated_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                profile.id.to_string(),
                profile.name,
                i64::from(profile.enabled),
                profile.priority,
                serde_json::to_string(&profile.activation).map_err(json_error)?,
                format_ts(profile.updated_at),
                serde_json::to_string(profile).map_err(json_error)?,
            ],
        )
        .map_err(storage)?;
        for (app, entry) in &profile.rules.apps {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'app', ?2, ?3)",
                params![
                    profile.id.to_string(),
                    app.as_str(),
                    serde_json::to_string(entry).map_err(json_error)?
                ],
            )
            .map_err(storage)?;
        }
        for site in &profile.rules.sites {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'site', ?2, ?3)",
                params![
                    profile.id.to_string(),
                    site.pattern.to_string(),
                    serde_json::to_string(&site.action).map_err(json_error)?
                ],
            )
            .map_err(storage)?;
        }
        for keyword in &profile.rules.keywords {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'keyword', ?2, ?3)",
                params![
                    profile.id.to_string(),
                    keyword.text.as_str(),
                    serde_json::to_string(&keyword.scope).map_err(json_error)?
                ],
            )
            .map_err(storage)?;
        }
        for app in &profile.rules.allow_apps {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'allow_app', ?2, '{}')",
                params![profile.id.to_string(), app.as_str()],
            )
            .map_err(storage)?;
        }
        for site in &profile.rules.allow_sites {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'allow_site', ?2, '{}')",
                params![profile.id.to_string(), site.to_string()],
            )
            .map_err(storage)?;
        }
        if let Some(limit) = profile.rules.pickup_limit {
            tx.execute(
                "INSERT INTO rules(profile_id, kind, target, payload) VALUES (?1, 'pickup', '', ?2)",
                params![
                    profile.id.to_string(),
                    serde_json::to_string(&limit).map_err(json_error)?
                ],
            )
            .map_err(storage)?;
        }
        if let Some(schedule) = &profile.schedule {
            tx.execute(
                "INSERT INTO schedules(profile_id, payload) VALUES (?1, ?2)",
                params![
                    profile.id.to_string(),
                    serde_json::to_string(schedule).map_err(json_error)?
                ],
            )
            .map_err(storage)?;
        }
    }

    for commitment in &state.commitments {
        tx.execute(
            "INSERT INTO commitments(id, state, starts_at, ends_at, updated_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                commitment.id.to_string(),
                serde_json::to_string(&commitment.state).map_err(json_error)?,
                format_ts(commitment.starts_at),
                commitment.ends_at.map(format_ts),
                format_ts(commitment.updated_at),
                serde_json::to_string(commitment).map_err(json_error)?,
            ],
        )
        .map_err(storage)?;
    }

    for block in &state.quick_blocks {
        tx.execute(
            "INSERT INTO quick_blocks(target, kind, until, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                block.target.key(),
                target_kind(&block.target),
                format_ts(block.until),
                format_ts(block.created_at)
            ],
        )
        .map_err(storage)?;
    }

    for (profile, until) in &state.extensions {
        tx.execute(
            "INSERT INTO extensions(profile_id, until) VALUES (?1, ?2)",
            params![profile.to_string(), format_ts(*until)],
        )
        .map_err(storage)?;
    }

    for interval in state.usage.intervals() {
        tx.execute(
            "INSERT INTO usage_events(app, start, end) VALUES (?1, ?2, ?3)",
            params![
                interval.app.as_str(),
                format_ts(interval.start),
                format_ts(interval.end)
            ],
        )
        .map_err(storage)?;
    }
    for launch in state.usage.launches() {
        tx.execute(
            "INSERT INTO usage_launches(app, at) VALUES (?1, ?2)",
            params![launch.app.as_str(), format_ts(launch.at)],
        )
        .map_err(storage)?;
    }
    for pickup in state.usage.pickups() {
        tx.execute(
            "INSERT INTO usage_pickups(at) VALUES (?1)",
            params![format_ts(*pickup)],
        )
        .map_err(storage)?;
    }
    for block in state.usage.blocks() {
        tx.execute(
            "INSERT INTO usage_blocks(at, target, kind, reason, until, profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                format_ts(block.at),
                block.target.key(),
                target_kind(&block.target),
                serde_json::to_value(block.reason)
                    .map_err(json_error)?
                    .as_str()
                    .unwrap_or_default(),
                block.until.map(format_ts),
                block.profile.map(|profile| profile.to_string()),
            ],
        )
        .map_err(storage)?;
    }

    let last_exit = serde_json::to_string(state.usage.last_exits()).map_err(json_error)?;
    tx.execute(
        "INSERT INTO usage_meta(id, open_app, open_start, last_exit, last_heartbeat)
         VALUES (1, ?1, ?2, ?3, ?4)",
        params![
            state.usage.open_foreground().map(|(app, _)| app.as_str()),
            state
                .usage
                .open_foreground()
                .map(|(_, start)| format_ts(start)),
            last_exit,
            state.usage.last_heartbeat().map(format_ts),
        ],
    )
    .map_err(storage)?;

    if let Some(profile) = state.manual_active {
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('manual_active', ?1)",
            params![profile.to_string()],
        )
        .map_err(storage)?;
    }
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('screen_locked', ?1)",
        params![if state.screen_locked { "1" } else { "0" }],
    )
    .map_err(storage)?;

    Ok(())
}

fn read_legacy(path: &Path) -> Result<crate::engine::State> {
    let text = fs::read_to_string(path)
        .map_err(|error| Error::Storage(format!("could not read {}: {error}", path.display())))?;
    let legacy: LegacyState = serde_json::from_str(&text).map_err(json_error)?;
    if legacy.version != 1 {
        return Err(Error::Storage(format!(
            "unsupported state version {}",
            legacy.version
        )));
    }
    Ok(legacy.state)
}

fn parse_settings(start_of_day: &str, timezone: &str) -> Result<ClockSettings> {
    let start_of_day = chrono::NaiveTime::parse_from_str(start_of_day, "%H:%M:%S")
        .or_else(|_| chrono::NaiveTime::parse_from_str(start_of_day, "%H:%M"))
        .map_err(|error| Error::Storage(format!("invalid start_of_day: {error}")))?;
    let timezone = timezone
        .parse()
        .map_err(|_| Error::Storage(format!("unknown time zone in database: {timezone}")))?;
    Ok(ClockSettings {
        start_of_day,
        timezone,
    })
}

fn parse_payload<T: serde::de::DeserializeOwned>(payload: &str) -> Result<T> {
    serde_json::from_str(payload).map_err(json_error)
}

fn parse_ts(value: &str) -> Result<Timestamp> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| Error::Storage(format!("invalid timestamp {value:?}: {error}")))
}

fn format_ts(value: Timestamp) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn parse_app(value: &str) -> Result<AppKey> {
    AppKey::new(value)
}

fn parse_id(value: &str) -> Result<crate::ids::ProfileId> {
    value
        .parse()
        .map_err(|_| Error::Storage(format!("invalid profile id {value:?}")))
}

fn target_kind(target: &Target) -> &'static str {
    match target {
        Target::App(_) => "app",
        Target::Site(_) => "site",
    }
}

fn parse_target(kind: &str, value: &str) -> Result<Target> {
    match kind {
        "app" => AppKey::new(value).map(Target::App),
        "site" => crate::app::Host::new(value).map(Target::Site),
        other => Err(Error::Storage(format!("invalid target kind {other:?}"))),
    }
    .map_err(|_| Error::Storage(format!("invalid {kind} target {value:?}")))
}

fn storage(error: rusqlite::Error) -> Error {
    Error::Storage(error.to_string())
}

fn json_error(error: serde_json::Error) -> Error {
    Error::Storage(error.to_string())
}

fn create_private_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            Error::Storage(format!("could not create {}: {error}", parent.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    Ok(())
}

fn set_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            let sidecar = PathBuf::from(sidecar);
            if sidecar.exists() {
                let _ = fs::set_permissions(sidecar, fs::Permissions::from_mode(0o600));
            }
        }
    }
}

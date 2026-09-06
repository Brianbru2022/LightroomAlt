//! Catalogue migrations that are known to have existed in released Keepframe builds.
//!
//! We deliberately do not manufacture a history for schema versions predating the
//! migration ledger.  Version 4 is the first migration whose changes are recorded
//! by this source tree, so it is applied atomically and recorded immutably here.

use chrono::Utc;
use rusqlite::{params, Connection};

fn column_exists(connection: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|name| name == column))
}

/// Applies the documented v4 changes in one SQLite transaction.  If any step
/// fails, SQLite rolls back the entire migration and the caller refuses to open
/// the catalogue.
pub(crate) fn apply_v4(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if !existed {
        connection.execute(
            "INSERT INTO schema_migrations(version,applied_at)VALUES(?1,?2)",
            params![4, Utc::now().to_rfc3339()],
        )?;
        connection.pragma_update(None, "user_version", 4)?;
        return Ok(());
    }
    if version >= 4 {
        return Ok(());
    }

    let additions = [
        ("assets", "trashed_at", "ALTER TABLE assets ADD COLUMN trashed_at TEXT"),
        ("imports", "state", "ALTER TABLE imports ADD COLUMN state TEXT NOT NULL DEFAULT 'completed'"),
        ("imports", "mode", "ALTER TABLE imports ADD COLUMN mode TEXT NOT NULL DEFAULT 'move'"),
        ("imports", "duplicate_policy", "ALTER TABLE imports ADD COLUMN duplicate_policy TEXT NOT NULL DEFAULT 'remove_after_verified_match'"),
        ("imports", "cancel_requested", "ALTER TABLE imports ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0"),
        ("import_items", "staging_path", "ALTER TABLE import_items ADD COLUMN staging_path TEXT"),
        ("import_items", "managed_path", "ALTER TABLE import_items ADD COLUMN managed_path TEXT"),
        ("import_items", "source_hash", "ALTER TABLE import_items ADD COLUMN source_hash TEXT"),
        ("import_items", "source_size", "ALTER TABLE import_items ADD COLUMN source_size INTEGER"),
        ("import_items", "source_modified", "ALTER TABLE import_items ADD COLUMN source_modified TEXT"),
        ("import_items", "source_action", "ALTER TABLE import_items ADD COLUMN source_action TEXT"),
    ];
    let missing = additions
        .iter()
        .map(|(table, column, sql)| Ok((*sql, !column_exists(connection, table, column)?)))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tx = connection.transaction()?;
    for (sql, should_add) in missing {
        if should_add {
            tx.execute(sql, [])?;
        }
    }
    tx.execute(
        "UPDATE imports SET state=CASE WHEN completed_at IS NULL THEN 'needs_attention' ELSE 'completed' END",
        [],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(4,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 4)?;
    tx.commit()
}

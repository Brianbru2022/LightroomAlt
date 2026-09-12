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

/// Version 5 preserves embedded GPS separately from catalogue-level placement.
/// This lets a user move or clear a manual pin without rewriting source-derived
/// metadata, and adds the indexes used by the lightweight map query.
pub(crate) fn apply_v5(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if !existed {
        connection.execute(
            "INSERT INTO schema_migrations(version,applied_at)VALUES(?1,?2)",
            params![5, Utc::now().to_rfc3339()],
        )?;
        connection.pragma_update(None, "user_version", 5)?;
        return Ok(());
    }
    if version >= 5 {
        return Ok(());
    }

    let additions = [
        ("embedded_latitude", "ALTER TABLE assets ADD COLUMN embedded_latitude REAL"),
        ("embedded_longitude", "ALTER TABLE assets ADD COLUMN embedded_longitude REAL"),
        ("manual_latitude", "ALTER TABLE assets ADD COLUMN manual_latitude REAL"),
        ("manual_longitude", "ALTER TABLE assets ADD COLUMN manual_longitude REAL"),
        ("location_source", "ALTER TABLE assets ADD COLUMN location_source TEXT NOT NULL DEFAULT 'none'"),
    ];
    let missing = additions
        .iter()
        .map(|(column, sql)| Ok((*sql, !column_exists(connection, "assets", column)?)))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tx = connection.transaction()?;
    for (sql, should_add) in missing {
        if should_add {
            tx.execute(sql, [])?;
        }
    }
    tx.execute(
        "UPDATE assets SET embedded_latitude=latitude,embedded_longitude=longitude,location_source=CASE WHEN latitude IS NULL OR longitude IS NULL THEN 'none' ELSE 'embedded' END WHERE embedded_latitude IS NULL AND embedded_longitude IS NULL",
        [],
    )?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_location ON assets(latitude,longitude,captured_at DESC)", [])?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_location_source ON assets(location_source,captured_at DESC)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(5,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 5)?;
    tx.commit()
}

/// Version 6 records non-destructive library-health observations and the hashes
/// of sidecars Keepframe explicitly exported.  It does not alter image or XMP
/// contents and remains safe to apply to a copied/moved library.
pub(crate) fn apply_v6(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if !existed {
        connection.execute(
            "INSERT INTO schema_migrations(version,applied_at)VALUES(?1,?2)",
            params![6, Utc::now().to_rfc3339()],
        )?;
        connection.pragma_update(None, "user_version", 6)?;
        return Ok(());
    }
    if version >= 6 {
        return Ok(());
    }

    let additions = [
        ("missing_state", "ALTER TABLE assets ADD COLUMN missing_state TEXT NOT NULL DEFAULT 'available'"),
        ("last_verified_at", "ALTER TABLE assets ADD COLUMN last_verified_at TEXT"),
    ];
    let missing = additions
        .iter()
        .map(|(column, sql)| Ok((*sql, !column_exists(connection, "assets", column)?)))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tx = connection.transaction()?;
    for (sql, should_add) in missing {
        if should_add {
            tx.execute(sql, [])?;
        }
    }
    tx.execute(
        "CREATE TABLE IF NOT EXISTS sidecar_exports(asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,path TEXT NOT NULL,content_hash TEXT NOT NULL,exported_at TEXT NOT NULL)",
        [],
    )?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS watch_folders(path TEXT PRIMARY KEY,enabled INTEGER NOT NULL DEFAULT 1,created_at TEXT NOT NULL)",
        [],
    )?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS watch_events(id INTEGER PRIMARY KEY AUTOINCREMENT,folder_path TEXT NOT NULL,path TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN('new_file','file_changed','file_removed')),observed_at TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'inbox',UNIQUE(folder_path,path,kind,state))",
        [],
    )?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_missing_state ON assets(missing_state,captured_at DESC)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(6,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 6)?;
    tx.commit()
}

/// Version 7 introduces the single, authoritative non-destructive Develop
/// recipe for an asset.  It contains settings only: no pixels, previews, or
/// UI state are stored in the catalogue.
pub(crate) fn apply_v7(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if !existed {
        connection.execute(
            "INSERT INTO schema_migrations(version,applied_at)VALUES(?1,?2)",
            params![7, Utc::now().to_rfc3339()],
        )?;
        connection.pragma_update(None, "user_version", 7)?;
        return Ok(());
    }
    if version >= 7 {
        return Ok(());
    }
    let tx = connection.transaction()?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS develop_recipes(asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL)",
        [],
    )?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_develop_recipes_updated ON develop_recipes(updated_at DESC)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(7,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 7)?;
    tx.commit()
}

/// Version 8 stores only user-authored Develop presets.  Built-ins remain in
/// the executable, so an application upgrade can improve their wording or
/// availability without silently changing a recipe which was already applied.
/// A preset is settings plus an explicit category list; it never stores an
/// asset id, file path, preview or any other catalogue/UI data.
pub(crate) fn apply_v8(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if !existed {
        connection.execute(
            "INSERT INTO schema_migrations(version,applied_at)VALUES(?1,?2)",
            params![8, Utc::now().to_rfc3339()],
        )?;
        connection.pragma_update(None, "user_version", 8)?;
        return Ok(());
    }
    if version >= 8 {
        return Ok(());
    }
    let tx = connection.transaction()?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS develop_presets(id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,schema_version INTEGER NOT NULL,preset_json TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)",
        [],
    )?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_develop_presets_name ON develop_presets(name COLLATE NOCASE)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(8,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 8)?;
    tx.commit()
}

/// Version 9 records support for Develop recipe v2. Existing v1 JSON is left
/// byte-for-byte unchanged and upgraded in memory, so appearance cannot drift.
pub(crate) fn apply_v9(connection: &mut Connection, _existed: bool, version: i64) -> rusqlite::Result<()> {
    if version >= 9 { return Ok(()); }
    let tx = connection.transaction()?;
    tx.execute("INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(9,?1)", [Utc::now().to_rfc3339()])?;
    tx.pragma_update(None, "user_version", 9)?;
    tx.commit()
}

/// Version 10 stores user-authored output presets only. Destinations and queue
/// state are deliberately excluded so portable presets cannot disclose paths.
pub(crate) fn apply_v10(connection: &mut Connection, _existed: bool, version: i64) -> rusqlite::Result<()> {
    if version >= 10 { return Ok(()); }
    let tx = connection.transaction()?;
    tx.execute("CREATE TABLE IF NOT EXISTS export_presets(id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,schema_version INTEGER NOT NULL,preset_json TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)", [])?;
    tx.execute("INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(10,?1)", [Utc::now().to_rfc3339()])?;
    tx.pragma_update(None, "user_version", 10)?;
    tx.commit()
}

/// Version 11 adds catalogue-owned descriptive metadata and ratings. These
/// fields never rewrite originals. The compound indexes cover the measured
/// Library flag/rating sorts without multiplying indexes for every filter.
pub(crate) fn apply_v11(connection: &mut Connection, existed: bool, version: i64) -> rusqlite::Result<()> {
    if version >= 11 { return Ok(()); }
    let additions = [
        ("rating", "ALTER TABLE assets ADD COLUMN rating INTEGER NOT NULL DEFAULT 0 CHECK(rating BETWEEN 0 AND 5)"),
        ("title", "ALTER TABLE assets ADD COLUMN title TEXT"),
        ("caption", "ALTER TABLE assets ADD COLUMN caption TEXT"),
        ("copyright", "ALTER TABLE assets ADD COLUMN copyright TEXT"),
        ("creator", "ALTER TABLE assets ADD COLUMN creator TEXT"),
    ];
    let missing = if existed {
        additions.iter().map(|(column, sql)| Ok((*sql, !column_exists(connection, "assets", column)?))).collect::<rusqlite::Result<Vec<_>>>()?
    } else { Vec::new() };
    let tx = connection.transaction()?;
    for (sql, should_add) in missing { if should_add { tx.execute(sql, [])?; } }
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_rating_captured ON assets(rating,captured_at DESC)", [])?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_decision_rating ON assets(decision,rating DESC,captured_at DESC)", [])?;
    tx.execute("INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(11,?1)", [Utc::now().to_rfc3339()])?;
    tx.pragma_update(None, "user_version", 11)?;
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v6_migration_is_idempotent_and_creates_resilience_tables() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY,captured_at TEXT NOT NULL);").unwrap();
        apply_v6(&mut connection, true, 5).unwrap();
        apply_v6(&mut connection, true, 6).unwrap();
        assert!(column_exists(&connection, "assets", "missing_state").unwrap());
        assert!(column_exists(&connection, "assets", "last_verified_at").unwrap());
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 6);
        let sidecars: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='sidecar_exports'", [], |row| row.get(0)).unwrap();
        let watches: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='watch_events'", [], |row| row.get(0)).unwrap();
        assert_eq!((sidecars, watches), (1, 1));
    }

    #[test]
    fn v7_migration_is_atomic_and_creates_develop_recipes() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY);").unwrap();
        apply_v7(&mut connection, true, 6).unwrap();
        apply_v7(&mut connection, true, 7).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        let records: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='develop_recipes'", [], |row| row.get(0)).unwrap();
        assert_eq!((version, records), (7, 1));
    }

    #[test]
    fn v8_migration_is_atomic_and_creates_user_presets() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);").unwrap();
        apply_v8(&mut connection, true, 7).unwrap();
        apply_v8(&mut connection, true, 8).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        let records: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='develop_presets'", [], |row| row.get(0)).unwrap();
        assert_eq!((version, records), (8, 1));
    }

    #[test]
    fn v9_migration_preserves_existing_recipe_json() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE develop_recipes(asset_id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL); INSERT INTO develop_recipes VALUES('asset',1,'{\"schemaVersion\":1,\"settings\":{}}','before');").unwrap();
        apply_v9(&mut connection, true, 8).unwrap(); apply_v9(&mut connection, true, 9).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        let recipe: String = connection.query_row("SELECT recipe_json FROM develop_recipes WHERE asset_id='asset'", [], |row| row.get(0)).unwrap();
        assert_eq!(version, 9); assert_eq!(recipe, "{\"schemaVersion\":1,\"settings\":{}}");
    }

    #[test]
    fn v10_migration_is_atomic_and_creates_export_presets() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);").unwrap();
        apply_v10(&mut connection, true, 9).unwrap(); apply_v10(&mut connection, true, 10).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        let records: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='export_presets'", [], |row| row.get(0)).unwrap();
        assert_eq!((version, records), (10, 1));
    }


    #[test]
    fn v11_migration_adds_library_metadata_idempotently() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY,captured_at TEXT NOT NULL,decision TEXT NOT NULL DEFAULT 'undecided');").unwrap();
        apply_v11(&mut connection, true, 10).unwrap(); apply_v11(&mut connection, true, 11).unwrap();
        assert!(column_exists(&connection, "assets", "rating").unwrap());
        assert!(column_exists(&connection, "assets", "caption").unwrap());
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 11);
    }
}

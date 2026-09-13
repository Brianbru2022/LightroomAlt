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
pub(crate) fn apply_v4(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
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
pub(crate) fn apply_v5(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
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
        (
            "embedded_latitude",
            "ALTER TABLE assets ADD COLUMN embedded_latitude REAL",
        ),
        (
            "embedded_longitude",
            "ALTER TABLE assets ADD COLUMN embedded_longitude REAL",
        ),
        (
            "manual_latitude",
            "ALTER TABLE assets ADD COLUMN manual_latitude REAL",
        ),
        (
            "manual_longitude",
            "ALTER TABLE assets ADD COLUMN manual_longitude REAL",
        ),
        (
            "location_source",
            "ALTER TABLE assets ADD COLUMN location_source TEXT NOT NULL DEFAULT 'none'",
        ),
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
pub(crate) fn apply_v6(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
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
        (
            "missing_state",
            "ALTER TABLE assets ADD COLUMN missing_state TEXT NOT NULL DEFAULT 'available'",
        ),
        (
            "last_verified_at",
            "ALTER TABLE assets ADD COLUMN last_verified_at TEXT",
        ),
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
pub(crate) fn apply_v7(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
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
pub(crate) fn apply_v8(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
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
pub(crate) fn apply_v9(
    connection: &mut Connection,
    _existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
    if version >= 9 {
        return Ok(());
    }
    let tx = connection.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(9,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 9)?;
    tx.commit()
}

/// Version 10 stores user-authored output presets only. Destinations and queue
/// state are deliberately excluded so portable presets cannot disclose paths.
pub(crate) fn apply_v10(
    connection: &mut Connection,
    _existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
    if version >= 10 {
        return Ok(());
    }
    let tx = connection.transaction()?;
    tx.execute("CREATE TABLE IF NOT EXISTS export_presets(id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,schema_version INTEGER NOT NULL,preset_json TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(10,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 10)?;
    tx.commit()
}

/// Version 11 adds catalogue-owned descriptive metadata and ratings. These
/// fields never rewrite originals. The compound indexes cover the measured
/// Library flag/rating sorts without multiplying indexes for every filter.
pub(crate) fn apply_v11(
    connection: &mut Connection,
    existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
    if version >= 11 {
        return Ok(());
    }
    let additions = [
        ("rating", "ALTER TABLE assets ADD COLUMN rating INTEGER NOT NULL DEFAULT 0 CHECK(rating BETWEEN 0 AND 5)"),
        ("title", "ALTER TABLE assets ADD COLUMN title TEXT"),
        ("caption", "ALTER TABLE assets ADD COLUMN caption TEXT"),
        ("copyright", "ALTER TABLE assets ADD COLUMN copyright TEXT"),
        ("creator", "ALTER TABLE assets ADD COLUMN creator TEXT"),
    ];
    let missing = if existed {
        additions
            .iter()
            .map(|(column, sql)| Ok((*sql, !column_exists(connection, "assets", column)?)))
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let tx = connection.transaction()?;
    for (sql, should_add) in missing {
        if should_add {
            tx.execute(sql, [])?;
        }
    }
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_assets_rating_captured ON assets(rating,captured_at DESC)",
        [],
    )?;
    tx.execute("CREATE INDEX IF NOT EXISTS idx_assets_decision_rating ON assets(decision,rating DESC,captured_at DESC)", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(11,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 11)?;
    tx.commit()
}

/// Version 12 separates durable physical sources from Library-visible catalogue
/// items. Existing asset rows become primary items without changing their IDs,
/// recipes or mutable metadata. The legacy source-derived asset columns remain
/// compatibility mirrors for older code and fixtures; `sources` is authoritative.
pub(crate) fn apply_v12(
    connection: &mut Connection,
    _existed: bool,
    version: i64,
) -> rusqlite::Result<()> {
    if version >= 12 {
        return Ok(());
    }
    let tx = connection.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources(
            id TEXT PRIMARY KEY,
            filename TEXT NOT NULL,
            captured_at TEXT NOT NULL,
            date_fallback INTEGER NOT NULL DEFAULT 0,
            camera TEXT,
            lens TEXT,
            width INTEGER,
            height INTEGER,
            latitude REAL,
            longitude REAL,
            embedded_latitude REAL,
            embedded_longitude REAL,
            manual_latitude REAL,
            manual_longitude REAL,
            location_source TEXT NOT NULL DEFAULT 'none',
            missing_state TEXT NOT NULL DEFAULT 'available',
            last_verified_at TEXT,
            thumbnail_path TEXT NOT NULL,
            created_at TEXT NOT NULL,
            trashed_at TEXT,
            versions_collapsed INTEGER NOT NULL DEFAULT 0 CHECK(versions_collapsed IN(0,1))
        );",
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO sources(id,filename,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,created_at,trashed_at)
         SELECT id,filename,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,COALESCE(location_source,'none'),COALESCE(missing_state,'available'),last_verified_at,thumbnail_path,created_at,trashed_at FROM assets",
        [],
    )?;
    if !column_exists(&tx, "assets", "source_id")? {
        tx.execute(
            "ALTER TABLE assets ADD COLUMN source_id TEXT REFERENCES sources(id) ON DELETE CASCADE",
            [],
        )?;
    }
    if !column_exists(&tx, "assets", "is_primary")? {
        tx.execute("ALTER TABLE assets ADD COLUMN is_primary INTEGER NOT NULL DEFAULT 1 CHECK(is_primary IN(0,1))", [])?;
    }
    if !column_exists(&tx, "assets", "version_name")? {
        tx.execute("ALTER TABLE assets ADD COLUMN version_name TEXT", [])?;
    }
    if !column_exists(&tx, "assets", "version_index")? {
        tx.execute("ALTER TABLE assets ADD COLUMN version_index INTEGER NOT NULL DEFAULT 1 CHECK(version_index >= 1)", [])?;
    }
    if !column_exists(&tx, "representations", "source_id")? {
        tx.execute("ALTER TABLE representations ADD COLUMN source_id TEXT REFERENCES sources(id) ON DELETE CASCADE", [])?;
    }
    tx.execute(
        "UPDATE assets SET source_id=id,is_primary=1,version_index=1 WHERE source_id IS NULL",
        [],
    )?;
    tx.execute("UPDATE representations SET source_id=(SELECT a.source_id FROM assets a WHERE a.id=representations.asset_id) WHERE source_id IS NULL", [])?;
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_assets_one_primary_source ON assets(source_id) WHERE is_primary=1;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_assets_source_version_index ON assets(source_id,version_index);
         CREATE INDEX IF NOT EXISTS idx_assets_source_primary ON assets(source_id,is_primary);
         CREATE INDEX IF NOT EXISTS idx_representations_source ON representations(source_id,is_raw,path);
         CREATE TABLE IF NOT EXISTS collection_sets(
            id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            position INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL,updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS collections(
            id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            kind TEXT NOT NULL CHECK(kind IN('manual','smart')),
            set_id TEXT REFERENCES collection_sets(id) ON DELETE SET NULL,
            match_mode TEXT NOT NULL DEFAULT 'all' CHECK(match_mode IN('all','any')),
            rules_json TEXT NOT NULL DEFAULT '[]',position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS collection_items(
            collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
            item_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
            position INTEGER NOT NULL DEFAULT 0,added_at TEXT NOT NULL,
            PRIMARY KEY(collection_id,item_id)
         );
         CREATE INDEX IF NOT EXISTS idx_collection_items_item ON collection_items(item_id,collection_id);
         CREATE TABLE IF NOT EXISTS stacks(
            id TEXT PRIMARY KEY,name TEXT,collapsed INTEGER NOT NULL DEFAULT 1 CHECK(collapsed IN(0,1)),
            top_item_id TEXT NOT NULL REFERENCES assets(id) ON DELETE RESTRICT,
            created_at TEXT NOT NULL,updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS stack_items(
            stack_id TEXT NOT NULL REFERENCES stacks(id) ON DELETE CASCADE,
            item_id TEXT NOT NULL UNIQUE REFERENCES assets(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,PRIMARY KEY(stack_id,item_id),UNIQUE(stack_id,position)
         );
         CREATE INDEX IF NOT EXISTS idx_stack_items_stack_position ON stack_items(stack_id,position);
         CREATE TRIGGER IF NOT EXISTS keepframe_asset_source_after_insert
         AFTER INSERT ON assets WHEN NEW.source_id IS NULL BEGIN
            INSERT OR IGNORE INTO sources(id,filename,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,created_at,trashed_at)
            VALUES(NEW.id,NEW.filename,NEW.captured_at,NEW.date_fallback,NEW.camera,NEW.width,NEW.height,NEW.latitude,NEW.longitude,NEW.embedded_latitude,NEW.embedded_longitude,NEW.manual_latitude,NEW.manual_longitude,COALESCE(NEW.location_source,'none'),COALESCE(NEW.missing_state,'available'),NEW.last_verified_at,NEW.thumbnail_path,NEW.created_at,NEW.trashed_at);
            UPDATE assets SET source_id=NEW.id,is_primary=1,version_index=1 WHERE id=NEW.id;
         END;
         CREATE TRIGGER IF NOT EXISTS keepframe_representation_source_after_insert
         AFTER INSERT ON representations WHEN NEW.source_id IS NULL BEGIN
            UPDATE representations SET source_id=(SELECT source_id FROM assets WHERE id=NEW.asset_id) WHERE id=NEW.id;
         END;
         CREATE TRIGGER IF NOT EXISTS keepframe_asset_source_facts_after_update
         AFTER UPDATE OF filename,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,trashed_at ON assets
         WHEN NEW.source_id IS NOT NULL BEGIN
            UPDATE sources SET filename=NEW.filename,captured_at=NEW.captured_at,date_fallback=NEW.date_fallback,camera=NEW.camera,width=NEW.width,height=NEW.height,latitude=NEW.latitude,longitude=NEW.longitude,embedded_latitude=NEW.embedded_latitude,embedded_longitude=NEW.embedded_longitude,manual_latitude=NEW.manual_latitude,manual_longitude=NEW.manual_longitude,location_source=COALESCE(NEW.location_source,'none'),missing_state=COALESCE(NEW.missing_state,'available'),last_verified_at=NEW.last_verified_at,trashed_at=NEW.trashed_at WHERE id=NEW.source_id;
         END;"
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations(version,applied_at)VALUES(12,?1)",
        [Utc::now().to_rfc3339()],
    )?;
    tx.pragma_update(None, "user_version", 12)?;
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
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6);
        let sidecars: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='sidecar_exports'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let watches: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='watch_events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((sidecars, watches), (1, 1));
    }

    #[test]
    fn v7_migration_is_atomic_and_creates_develop_recipes() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY);").unwrap();
        apply_v7(&mut connection, true, 6).unwrap();
        apply_v7(&mut connection, true, 7).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let records: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='develop_recipes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((version, records), (7, 1));
    }

    #[test]
    fn v8_migration_is_atomic_and_creates_user_presets() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);").unwrap();
        apply_v8(&mut connection, true, 7).unwrap();
        apply_v8(&mut connection, true, 8).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let records: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='develop_presets'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((version, records), (8, 1));
    }

    #[test]
    fn v9_migration_preserves_existing_recipe_json() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE develop_recipes(asset_id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL); INSERT INTO develop_recipes VALUES('asset',1,'{\"schemaVersion\":1,\"settings\":{}}','before');").unwrap();
        apply_v9(&mut connection, true, 8).unwrap();
        apply_v9(&mut connection, true, 9).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let recipe: String = connection
            .query_row(
                "SELECT recipe_json FROM develop_recipes WHERE asset_id='asset'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 9);
        assert_eq!(recipe, "{\"schemaVersion\":1,\"settings\":{}}");
    }

    #[test]
    fn v10_migration_is_atomic_and_creates_export_presets() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);").unwrap();
        apply_v10(&mut connection, true, 9).unwrap();
        apply_v10(&mut connection, true, 10).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let records: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='export_presets'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((version, records), (10, 1));
    }

    #[test]
    fn v11_migration_adds_library_metadata_idempotently() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY,captured_at TEXT NOT NULL,decision TEXT NOT NULL DEFAULT 'undecided');").unwrap();
        apply_v11(&mut connection, true, 10).unwrap();
        apply_v11(&mut connection, true, 11).unwrap();
        assert!(column_exists(&connection, "assets", "rating").unwrap());
        assert!(column_exists(&connection, "assets", "caption").unwrap());
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 11);
    }

    #[test]
    fn v12_migration_creates_one_primary_item_per_source_and_organisation_tables() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY,filename TEXT NOT NULL,decision TEXT NOT NULL DEFAULT 'undecided',rating INTEGER NOT NULL DEFAULT 0,title TEXT,caption TEXT,copyright TEXT,creator TEXT,captured_at TEXT NOT NULL,date_fallback INTEGER NOT NULL DEFAULT 0,camera TEXT,width INTEGER,height INTEGER,latitude REAL,longitude REAL,embedded_latitude REAL,embedded_longitude REAL,manual_latitude REAL,manual_longitude REAL,location_source TEXT NOT NULL DEFAULT 'none',missing_state TEXT NOT NULL DEFAULT 'available',last_verified_at TEXT,thumbnail_path TEXT NOT NULL,preferred_version_id TEXT,created_at TEXT NOT NULL,trashed_at TEXT); CREATE TABLE representations(id TEXT PRIMARY KEY,asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,path TEXT NOT NULL UNIQUE,sha256 TEXT NOT NULL,extension TEXT NOT NULL,stem TEXT NOT NULL,byte_size INTEGER NOT NULL,is_raw INTEGER NOT NULL DEFAULT 0); INSERT INTO assets(id,filename,decision,rating,title,captured_at,thumbnail_path,created_at)VALUES('a','one.jpg','keep',5,'Colour','2026-01-01T00:00:00Z','thumb.jpg','2026-01-01T00:00:00Z'); INSERT INTO representations VALUES('r','a','one.jpg','hash','jpg','one',10,0);").unwrap();
        apply_v12(&mut connection, true, 11).unwrap();
        apply_v12(&mut connection, true, 12).unwrap();
        let migrated: (String, i64, i64, String, i64) = connection
            .query_row(
                "SELECT source_id,is_primary,version_index,title,rating FROM assets WHERE id='a'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(migrated, ("a".into(), 1, 1, "Colour".into(), 5));
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM sources", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN('collections','collection_items','collection_sets','stacks','stack_items')",[],|row|row.get::<_,i64>(0)).unwrap(),5);
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            12
        );
    }

    #[test]
    fn v9_catalogue_reaches_v12_without_changing_recipe_identity_or_metadata() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);CREATE TABLE assets(id TEXT PRIMARY KEY,filename TEXT NOT NULL,decision TEXT NOT NULL DEFAULT 'undecided',captured_at TEXT NOT NULL,date_fallback INTEGER NOT NULL DEFAULT 0,camera TEXT,width INTEGER,height INTEGER,latitude REAL,longitude REAL,embedded_latitude REAL,embedded_longitude REAL,manual_latitude REAL,manual_longitude REAL,location_source TEXT NOT NULL DEFAULT 'none',missing_state TEXT NOT NULL DEFAULT 'available',last_verified_at TEXT,thumbnail_path TEXT NOT NULL,preferred_version_id TEXT,created_at TEXT NOT NULL,trashed_at TEXT);CREATE TABLE representations(id TEXT PRIMARY KEY,asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,path TEXT NOT NULL UNIQUE,sha256 TEXT NOT NULL,extension TEXT NOT NULL,stem TEXT NOT NULL,byte_size INTEGER NOT NULL,is_raw INTEGER NOT NULL DEFAULT 0);CREATE TABLE develop_recipes(asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL);INSERT INTO assets(id,filename,decision,captured_at,camera,thumbnail_path,created_at)VALUES('legacy','legacy.nef','keep','2025-01-02T03:04:05Z','Nikon Z8','thumb.png','2025-01-02T03:04:05Z');INSERT INTO representations VALUES('rep','legacy','legacy.nef','protected-hash','nef','legacy',123,1);INSERT INTO develop_recipes VALUES('legacy',2,'{\"schemaVersion\":2,\"settings\":{},\"masks\":[]}','unchanged');PRAGMA user_version=9;").unwrap();
        apply_v10(&mut connection, true, 9).unwrap();
        apply_v11(&mut connection, true, 10).unwrap();
        connection
            .execute(
                "UPDATE assets SET rating=4,title='Legacy title' WHERE id='legacy'",
                [],
            )
            .unwrap();
        let recipe_before: String = connection
            .query_row(
                "SELECT recipe_json FROM develop_recipes WHERE asset_id='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        apply_v12(&mut connection, true, 11).unwrap();
        let migrated:(String,i64,i64,String,String)=connection.query_row("SELECT source_id,is_primary,rating,title,(SELECT sha256 FROM representations WHERE asset_id=assets.id) FROM assets WHERE id='legacy'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(
            migrated,
            (
                "legacy".into(),
                1,
                4,
                "Legacy title".into(),
                "protected-hash".into()
            )
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT recipe_json FROM develop_recipes WHERE asset_id='legacy'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            recipe_before
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            12
        );
    }
}

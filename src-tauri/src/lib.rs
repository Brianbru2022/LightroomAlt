mod migrations;
mod safety;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
use fs2::FileExt;
use image::{GenericImageView, ImageDecoder, ImageFormat, RgbImage};
use reqwest::multipart;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;
use walkdir::WalkDir;

const SCHEMA_VERSION: i64 = 5;
const SUPPORTED: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "heic", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf",
    "rw2",
];
const RAW: &[&str] = &["dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2"];

fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

#[derive(Default)]
struct AnalysisWorker {
    url: Option<String>,
    token: Option<String>,
    child: Option<Child>,
    last_error: Option<String>,
}
struct AppState {
    root: Mutex<Option<PathBuf>>,
    library_lock: Mutex<Option<fs::File>>,
    library_issue: Mutex<Option<String>>,
    recovery_notice: Mutex<Option<String>>,
    local_ai_url: Mutex<String>,
    health_cache: Mutex<Option<(Instant, ServiceHealth)>>,
    analysis_worker: Mutex<AnalysisWorker>,
    gpu_gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Error)]
enum KeepframeError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    PngEncoding(#[from] png::EncodingError),
    #[error(transparent)]
    Network(#[from] reqwest::Error),
}
type Result<T> = std::result::Result<T, KeepframeError>;
impl serde::Serialize for KeepframeError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Counts {
    total: i64,
    keep: i64,
    undecided: i64,
    discard: i64,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LibraryStatus {
    configured: bool,
    library_root: Option<String>,
    library_issue: Option<String>,
    recovery_notice: Option<String>,
    counts: Counts,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ServiceHealth {
    local_ai_available: bool,
    service_reachable: bool,
    local_ai_busy: bool,
    local_ai_model: Option<String>,
    local_ai_detail: String,
    local_ai_state: String,
    local_ai_url: String,
    analysis_model_installed: bool,
    analysis_available: bool,
    analysis_detail: String,
}
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AssetVersion {
    id: String,
    kind: String,
    provider: Option<String>,
    created_at: String,
    state: String,
    image_url: String,
    prompt: Option<String>,
    is_preferred: bool,
    source_hash: Option<String>,
    output_hash: Option<String>,
}
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct BasicAdjustments {
    exposure: f32,
    light_balance: f32,
    tint: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    dynamic_range: f32,
    texture: f32,
    clarity: f32,
    dehaze: f32,
    colour_boost: f32,
    saturation: f32,
    curve_highlights: f32,
    curve_lights: f32,
    curve_darks: f32,
    curve_shadows: f32,
    crop_left: f32,
    crop_top: f32,
    crop_width: f32,
    crop_height: f32,
    rotate_quadrants: i8,
    straighten: f32,
}

impl BasicAdjustments {
    fn validate(self) -> Result<Self> {
        let unit_values = [
            self.light_balance,
            self.tint,
            self.contrast,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
            self.dynamic_range,
            self.texture,
            self.clarity,
            self.dehaze,
            self.colour_boost,
            self.saturation,
            self.curve_highlights,
            self.curve_lights,
            self.curve_darks,
            self.curve_shadows,
        ];
        let crop_width = if self.crop_width == 0.0 { 1.0 } else { self.crop_width };
        let crop_height = if self.crop_height == 0.0 { 1.0 } else { self.crop_height };
        let valid = self.exposure.is_finite()
            && (-3.0..=3.0).contains(&self.exposure)
            && unit_values
                .iter()
                .all(|value| value.is_finite() && (-100.0..=100.0).contains(value))
            && self.crop_left.is_finite()
            && self.crop_top.is_finite()
            && self.straighten.is_finite()
            && (0.0..=0.95).contains(&self.crop_left)
            && (0.0..=0.95).contains(&self.crop_top)
            && (0.05..=1.0).contains(&crop_width)
            && (0.05..=1.0).contains(&crop_height)
            && self.crop_left + crop_width <= 1.0001
            && self.crop_top + crop_height <= 1.0001
            && (-45.0..=45.0).contains(&self.straighten)
            && (0..=3).contains(&self.rotate_quadrants);
        if !valid {
            return Err(KeepframeError::Message(
                "One or more adjustment values are outside the supported range.".into(),
            ));
        }
        Ok(self)
    }

    fn normalised_crop(self) -> (f32, f32, f32, f32) {
        (self.crop_left, self.crop_top, if self.crop_width == 0.0 { 1.0 } else { self.crop_width }, if self.crop_height == 0.0 { 1.0 } else { self.crop_height })
    }
}

struct AdjustmentInput {
    path: PathBuf,
    source_hash: String,
    captured_at: String,
    expected_dimensions: Option<(u32, u32)>,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AssetFilter {
    decision: String,
    search: String,
    year: Option<i32>,
    tag: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    camera: Option<String>,
    tagged: Option<bool>,
    located: Option<bool>,
    trashed: Option<bool>,
}
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Asset {
    id: String,
    filename: String,
    source_path: String,
    preview_url: String,
    thumbnail_url: String,
    decision: String,
    captured_at: String,
    date_fallback: bool,
    camera: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    location_source: String,
    tags: Vec<String>,
    representation_count: i64,
    preferred_version_url: Option<String>,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MapBounds {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MapQuery {
    filter: AssetFilter,
    bounds: Option<MapBounds>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MapAsset {
    id: String,
    filename: String,
    latitude: f64,
    longitude: f64,
    captured_at: String,
    thumbnail_url: String,
    decision: String,
    location_source: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetPage {
    items: Vec<Asset>,
    total: i64,
    offset: i64,
    limit: i64,
    has_more: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportSummary {
    import_id: String,
    state: String,
    discovered: usize,
    imported: usize,
    copied: usize,
    moved: usize,
    source_retained: usize,
    duplicates: usize,
    unsupported: usize,
    failed: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ImportOptions {
    mode: String,
    duplicate_source_policy: String,
}

impl ImportOptions {
    fn validate(&self) -> Result<()> {
        if !matches!(self.mode.as_str(), "copy" | "move") {
            return Err(KeepframeError::Message(
                "Import mode must be copy or move.".into(),
            ));
        }
        if !matches!(
            self.duplicate_source_policy.as_str(),
            "retain" | "remove_after_verified_match"
        ) {
            return Err(KeepframeError::Message(
                "Duplicate source policy is invalid.".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashSummary {
    affected: usize,
    failed: usize,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecipeOutput {
    format: String,
    preserve_dimensions: bool,
    colour_space: String,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EditRecipe {
    schema_version: u8,
    asset_id: String,
    common_brief: Option<String>,
    #[serde(default)]
    action: Option<String>,
    observations: Vec<String>,
    intents: Vec<String>,
    preserve: Vec<String>,
    negative_constraints: Vec<String>,
    strength: String,
    output: RecipeOutput,
    analysis_model: String,
    analysis_created_at: String,
}
#[derive(Debug, Serialize)]
struct PromptSet {
    local: String,
    chatgpt: String,
    gemini: String,
    negative: String,
}
#[derive(Debug, Deserialize)]
struct AnalysisResponse {
    observations: Vec<String>,
    suggested_intents: Vec<String>,
    preserve: Vec<String>,
    negative_constraints: Vec<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobAttempt {
    attempt_number: i64,
    state: String,
    started_at: String,
    finished_at: Option<String>,
    output_url: Option<String>,
    error: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchJob {
    id: String,
    batch_id: String,
    asset_id: String,
    asset_name: String,
    state: String,
    prompt: String,
    recipe: Option<EditRecipe>,
    attempts: Vec<JobAttempt>,
    error: Option<String>,
    output_url: Option<String>,
}
struct JobExecution {
    asset_id: String,
    prompt: String,
    recipe_json: String,
    negative_prompt: String,
    model: String,
    seed: i64,
    settings_json: String,
    source: String,
    hash: String,
    captured: String,
    width: Option<u32>,
    height: Option<u32>,
}

fn settings_path() -> Result<PathBuf> {
    let dir = std::env::var_os("KEEPFRAME_SETTINGS_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".keepframe")))
        .ok_or_else(|| {
            KeepframeError::Message("Windows user-profile folder is unavailable".into())
        })?;
    fs::create_dir_all(&dir)?;
    Ok(dir.join("settings.json"))
}
fn save_settings(root: &Path, local_ai_url: &str) -> Result<()> {
    fs::write(
        settings_path()?,
        serde_json::to_vec_pretty(&json!({"libraryRoot": root, "localAiUrl": local_ai_url}))?,
    )?;
    Ok(())
}
fn validated_loopback_url(value: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| KeepframeError::Message("The local AI service URL is invalid.".into()))?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "http" || !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]") {
        return Err(KeepframeError::Message(
            "Local AI services must use an HTTP loopback address (127.0.0.1, localhost or ::1)."
                .into(),
        ));
    }
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(KeepframeError::Message(
            "The local AI service URL contains unsupported components.".into(),
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn validated_provider(value: &str) -> Result<&str> {
    match value {
        "chatgpt" | "gemini" => Ok(value),
        _ => Err(KeepframeError::Message(
            "The external edit provider is invalid.".into(),
        )),
    }
}
fn load_settings() -> Option<(PathBuf, String)> {
    let primary = settings_path().ok()?;
    let legacy = dirs::config_dir().map(|path| path.join("Keepframe/settings.json"));
    for path in std::iter::once(primary.clone()).chain(legacy) {
        let value: Value = match fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => continue,
        };
        let root = PathBuf::from(value.get("libraryRoot")?.as_str()?);
        let local_ai_url = value
            .get("localAiUrl")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:7868")
            .to_string();
        let local_ai_url = validated_loopback_url(&local_ai_url)
            .unwrap_or_else(|_| "http://127.0.0.1:7868".into());
        if path != primary {
            let _ = save_settings(&root, &local_ai_url);
        }
        return Some((root, local_ai_url));
    }
    None
}
fn root_from(state: &State<AppState>) -> Result<PathBuf> {
    state
        .root
        .lock()
        .map_err(|_| KeepframeError::Message("Library state lock failed".into()))?
        .clone()
        .ok_or_else(|| KeepframeError::Message("Choose a Keepframe library first".into()))
}
fn db_path(root: &Path) -> PathBuf {
    root.join(".keepframe").join("catalogue.sqlite")
}
fn open_db(root: &Path) -> Result<Connection> {
    let db = db_path(root);
    let connection = Connection::open(db)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(connection)
}

fn append_runtime_log(root: &Path, event: &str, detail: &str) {
    let log_path = root.join(".keepframe/logs/runtime.jsonl");
    let entry = json!({
        "time": Utc::now().to_rfc3339(),
        "event": event,
        "detail": detail,
    });
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(file, "{entry}");
    }
}
fn allow_media_scope(app: &AppHandle, root: &Path) -> Result<()> {
    let scope = app.asset_protocol_scope();
    for directory in [
        root.join(".keepframe/thumbnails"),
        root.join(".keepframe/previews"),
        root.join(".keepframe/review"),
        root.join("Edits"),
    ] {
        scope.allow_directory(directory, true).map_err(|error| {
            KeepframeError::Message(format!("Could not constrain the media scope: {error}"))
        })?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryMarker {
    library_id: String,
    format_version: i64,
}

fn database_integrity(connection: &Connection) -> Result<String> {
    connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(KeepframeError::from)
}

fn backup_database(root: &Path, label: &str) -> Result<PathBuf> {
    let path = db_path(root);
    if !path.is_file() {
        return Err(KeepframeError::Message(
            "The catalogue database does not exist.".into(),
        ));
    }
    let connection = open_db(root)?;
    connection.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
    let backup = root.join(".keepframe/backups").join(format!(
        "catalogue-{}-{}-{}.sqlite",
        label,
        Local::now().format("%Y%m%d-%H%M%S"),
        &Uuid::new_v4().to_string()[..8]
    ));
    let partial = backup.with_extension("sqlite.partial");
    let vacuum_result = connection.execute("VACUUM INTO ?1", [partial.to_string_lossy().as_ref()]);
    if let Err(error) = vacuum_result {
        let _ = fs::remove_file(&partial);
        return Err(KeepframeError::Database(error));
    }
    let verification = Connection::open(&partial)?;
    if database_integrity(&verification)? != "ok" {
        drop(verification);
        let _ = fs::remove_file(&partial);
        return Err(KeepframeError::Message(
            "The catalogue backup failed its integrity check.".into(),
        ));
    }
    drop(verification);
    fs::rename(&partial, &backup)?;
    Ok(backup)
}

fn acquire_library_lock(root: &Path) -> Result<fs::File> {
    let control = root.join(".keepframe");
    fs::create_dir_all(&control)?;
    let path = control.join("catalogue.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive().map_err(|_| {
        KeepframeError::Message("This library is already open in another Keepframe window.".into())
    })?;
    Ok(file)
}

fn initialise_layout(root: &Path) -> Result<()> {
    for relative in [
        "Originals",
        "Edits",
        "Exports",
        ".keepframe/thumbnails",
        ".keepframe/previews",
        ".keepframe/review",
        ".keepframe/staging",
        ".keepframe/Trash",
        ".keepframe/backups",
        ".keepframe/logs",
    ] {
        fs::create_dir_all(root.join(relative))?;
    }
    let path = db_path(root);
    let existed = path.is_file();
    let mut connection = open_db(root)?;
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    if version > SCHEMA_VERSION {
        return Err(KeepframeError::Message(format!(
            "This library uses catalogue format {version}, but this build supports only {SCHEMA_VERSION}."
        )));
    }
    if existed && version < SCHEMA_VERSION {
        backup_database(root, &format!("before-v{SCHEMA_VERSION}"))?;
    }
    connection.execute_batch(r#"
      CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS assets(id TEXT PRIMARY KEY, filename TEXT NOT NULL, decision TEXT NOT NULL DEFAULT 'undecided' CHECK(decision IN('keep','undecided','discard')), captured_at TEXT NOT NULL, date_fallback INTEGER NOT NULL DEFAULT 0, camera TEXT, width INTEGER, height INTEGER, latitude REAL, longitude REAL, embedded_latitude REAL, embedded_longitude REAL, manual_latitude REAL, manual_longitude REAL, location_source TEXT NOT NULL DEFAULT 'none', thumbnail_path TEXT NOT NULL, preferred_version_id TEXT, created_at TEXT NOT NULL, trashed_at TEXT);
      CREATE TABLE IF NOT EXISTS representations(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, path TEXT NOT NULL UNIQUE, sha256 TEXT NOT NULL, extension TEXT NOT NULL, stem TEXT NOT NULL, byte_size INTEGER NOT NULL, is_raw INTEGER NOT NULL DEFAULT 0, UNIQUE(asset_id,path));
      CREATE INDEX IF NOT EXISTS idx_representations_hash ON representations(sha256);
      CREATE INDEX IF NOT EXISTS idx_representations_asset ON representations(asset_id,is_raw,path);
      CREATE TABLE IF NOT EXISTS tags(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, parent_id TEXT REFERENCES tags(id));
      CREATE TABLE IF NOT EXISTS asset_tags(asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY(asset_id,tag_id));
      CREATE INDEX IF NOT EXISTS idx_asset_tags_tag_asset ON asset_tags(tag_id,asset_id);
      CREATE INDEX IF NOT EXISTS idx_assets_captured_at ON assets(captured_at DESC);
      CREATE INDEX IF NOT EXISTS idx_assets_decision_captured_at ON assets(decision,captured_at DESC);
      CREATE INDEX IF NOT EXISTS idx_assets_location ON assets(latitude,longitude,captured_at DESC);
      CREATE INDEX IF NOT EXISTS idx_assets_location_source ON assets(location_source,captured_at DESC);
      CREATE TABLE IF NOT EXISTS imports(id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at TEXT NOT NULL, completed_at TEXT, discovered INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0, duplicates INTEGER NOT NULL DEFAULT 0, unsupported INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT 'running', mode TEXT NOT NULL DEFAULT 'copy', duplicate_policy TEXT NOT NULL DEFAULT 'retain', cancel_requested INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS import_items(id TEXT PRIMARY KEY, import_id TEXT NOT NULL REFERENCES imports(id), source_path TEXT NOT NULL, representation_id TEXT, state TEXT NOT NULL, error TEXT, staging_path TEXT, managed_path TEXT, source_hash TEXT, source_size INTEGER, source_modified TEXT, source_action TEXT);
      CREATE TABLE IF NOT EXISTS versions(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, kind TEXT NOT NULL, path TEXT NOT NULL, provider TEXT, prompt TEXT, recipe_json TEXT, source_hash TEXT, output_hash TEXT, state TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS edit_recipes(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, schema_version INTEGER NOT NULL, recipe_json TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS batches(id TEXT PRIMARY KEY, common_brief TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, paused INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, batch_id TEXT NOT NULL REFERENCES batches(id), asset_id TEXT NOT NULL REFERENCES assets(id), state TEXT NOT NULL, prompt TEXT NOT NULL, recipe_json TEXT, negative_prompt TEXT, model TEXT, seed INTEGER, settings_json TEXT, output_path TEXT, output_hash TEXT, error TEXT, attempts INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS job_attempts(id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, attempt_number INTEGER NOT NULL, state TEXT NOT NULL, source_hash TEXT NOT NULL, recipe_json TEXT NOT NULL, prompt TEXT NOT NULL, negative_prompt TEXT NOT NULL, model TEXT NOT NULL, seed INTEGER NOT NULL, settings_json TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT, output_path TEXT, output_hash TEXT, error TEXT, UNIQUE(job_id,attempt_number));
      CREATE INDEX IF NOT EXISTS idx_job_attempts_job ON job_attempts(job_id,attempt_number DESC);
      CREATE TABLE IF NOT EXISTS audit_log(id INTEGER PRIMARY KEY AUTOINCREMENT, entity_type TEXT NOT NULL, entity_id TEXT NOT NULL, action TEXT NOT NULL, old_json TEXT, new_json TEXT, created_at TEXT NOT NULL, undone INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS trash_operations(id TEXT PRIMARY KEY,state TEXT NOT NULL,created_at TEXT NOT NULL,completed_at TEXT,error TEXT);
      CREATE TABLE IF NOT EXISTS trash_items(id TEXT PRIMARY KEY,operation_id TEXT NOT NULL REFERENCES trash_operations(id),asset_id TEXT NOT NULL REFERENCES assets(id),original_path TEXT NOT NULL,trash_path TEXT NOT NULL,state TEXT NOT NULL,error TEXT,UNIQUE(operation_id,original_path));
      CREATE INDEX IF NOT EXISTS idx_trash_items_asset ON trash_items(asset_id,state);
    "#)?;
    migrations::apply_v4(&mut connection, existed, version)?;
    migrations::apply_v5(&mut connection, existed, version)?;
    if database_integrity(&connection)? != "ok" {
        return Err(KeepframeError::Message(
            "The catalogue failed its integrity check and was not opened.".into(),
        ));
    }
    let marker_path = root.join(".keepframe/library.json");
    if !marker_path.is_file() {
        let marker = LibraryMarker {
            library_id: Uuid::new_v4().to_string(),
            format_version: 1,
        };
        let temporary = marker_path.with_extension("json.tmp");
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&marker)?)?;
        file.sync_all()?;
        fs::rename(temporary, marker_path)?;
    }
    let recovered_at = Utc::now().to_rfc3339();
    connection.execute(
        "UPDATE job_attempts SET state='failed',finished_at=?1,error='Keepframe closed while this attempt was running' WHERE state='running'",
        [&recovered_at],
    )?;
    connection.execute(
        "UPDATE jobs SET state='queued',error='Recovered after Keepframe restarted',updated_at=?1 WHERE state='running'",
        [&recovered_at],
    )?;
    connection.execute(
        "UPDATE jobs SET state='cancelled',error='Legacy job did not contain an image-specific reviewed recipe; create a new batch to analyse it safely',updated_at=?1 WHERE recipe_json IS NULL AND state IN ('review_required','queued')",
        [&recovered_at],
    )?;
    Ok(())
}

/// Reconcile only catalogue state after an unclean shutdown.  This routine never
/// moves or deletes an image: uncertain import files stay in staging/Originals,
/// and uncertain Trash entries remain recoverable for inspection.
fn recover_interrupted_operations(root: &Path) -> Result<Option<String>> {
    let connection = open_db(root)?;
    let recovered_imports = connection.execute(
        "UPDATE imports SET state='needs_attention',completed_at=?1 WHERE state='running'",
        [Utc::now().to_rfc3339()],
    )?;
    let recovered_trash = connection.execute(
        "UPDATE assets SET trashed_at=?1 WHERE id IN (
            SELECT asset_id FROM trash_items GROUP BY asset_id
            HAVING SUM(CASE WHEN state='moved' THEN 1 ELSE 0 END) > 0
               AND SUM(CASE WHEN state IN ('planned','failed','restore_failed') THEN 1 ELSE 0 END) = 0
        ) AND trashed_at IS NULL",
        [Utc::now().to_rfc3339()],
    )?;
    let recovered_restores = connection.execute(
        "UPDATE assets SET trashed_at=NULL WHERE id IN (
            SELECT asset_id FROM trash_items GROUP BY asset_id
            HAVING SUM(CASE WHEN state='restored' THEN 1 ELSE 0 END) > 0
               AND SUM(CASE WHEN state <> 'restored' THEN 1 ELSE 0 END) = 0
        ) AND trashed_at IS NOT NULL",
        [],
    )?;

    let mut missing_trash = 0usize;
    let mut statement = connection.prepare("SELECT id,trash_path FROM trash_items WHERE state='moved'")?;
    let items = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for (id, path) in items {
        if !Path::new(&path).is_file() {
            connection.execute(
                "UPDATE trash_items SET state='empty_failed',error='Trash file was missing after an unclean shutdown; check the Windows Recycle Bin before taking further action.' WHERE id=?1",
                [&id],
            )?;
            missing_trash += 1;
        }
    }

    let mut details = Vec::new();
    if recovered_imports > 0 {
        details.push(format!("{recovered_imports} interrupted import(s) were found. Original source files were not deleted; staged or managed copies were retained for recovery."));
    }
    if recovered_trash > 0 || recovered_restores > 0 || missing_trash > 0 {
        details.push("An interrupted Trash or Restore operation was reconciled without deleting files. Check Trash and the Windows Recycle Bin before retrying any affected action.".into());
    }
    Ok((!details.is_empty()).then(|| details.join(" ")))
}

fn parse_local_ai_health(value: &Value) -> ServiceHealth {
    let reachable = true;
    let busy = value
        .pointer("/image_runtime/busy")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .pointer("/image_runtime/loading")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value
            .pointer("/image_runtime/generating")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let edit_model = value
        .get("edit_models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models.iter().find(|model| {
                model.get("key").and_then(Value::as_str) == Some("qwen-image-edit")
                    && model
                        .get("available")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
        });
    let available = edit_model.is_some();
    let model = edit_model
        .and_then(|item| item.get("label").or_else(|| item.get("key")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let detail = if !available {
        "The service is online, but Qwen image editing is not installed.".into()
    } else if busy {
        "The local editor is busy; new work will wait in Keepframe's queue.".into()
    } else {
        "The local Qwen image editor is ready.".into()
    };
    ServiceHealth {
        local_ai_available: available,
        service_reachable: reachable,
        local_ai_busy: busy,
        local_ai_model: model,
        local_ai_detail: detail,
        local_ai_state: if available { "available" } else { "model_missing" }.into(),
        local_ai_url: String::new(),
        analysis_model_installed: analysis_installed(),
        analysis_available: false,
        analysis_detail: analysis_installation_detail(),
    }
}
async fn local_ai_health(url: &str) -> ServiceHealth {
    let Ok(url) = validated_loopback_url(url) else {
        return ServiceHealth {
            local_ai_available: false,
            service_reachable: false,
            local_ai_busy: false,
            local_ai_model: None,
            local_ai_detail:
                "The configured AI service was blocked because it is not loopback-only.".into(),
            local_ai_state: "incompatible".into(),
            local_ai_url: String::new(),
            analysis_model_installed: analysis_installed(),
            analysis_available: false,
            analysis_detail: analysis_installation_detail(),
        };
    };
    let response = reqwest::Client::new()
        .get(format!("{url}/api/status"))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(value) => parse_local_ai_health(&value),
            Err(_) => ServiceHealth {
                local_ai_available: false,
                service_reachable: true,
                local_ai_busy: false,
                local_ai_model: None,
                local_ai_detail: "The service responded, but its capability report was invalid."
                    .into(),
                local_ai_state: "health_check_failed".into(),
                local_ai_url: url.clone(),
                analysis_model_installed: analysis_installed(),
                analysis_available: false,
                analysis_detail: analysis_installation_detail(),
            },
        },
        _ => ServiceHealth {
            local_ai_available: false,
            service_reachable: false,
            local_ai_busy: false,
            local_ai_model: None,
            local_ai_detail: format!("No local image service is responding at {url}."),
            local_ai_state: "service_not_running".into(),
            local_ai_url: url,
            analysis_model_installed: analysis_installed(),
            analysis_available: false,
            analysis_detail: analysis_installation_detail(),
        },
    }
}

fn analysis_runtime_path() -> Option<PathBuf> {
    let durable = PathBuf::from(r"D:\AI Models\Keepframe\runtime\Scripts\python.exe");
    if durable.is_file() {
        return Some(durable);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = manifest.parent().unwrap_or(Path::new("."));
    let development = project.join("ai-worker/.venv/Scripts/python.exe");
    development.is_file().then_some(development)
}

fn analysis_installation_detail() -> String {
    if !analysis_installed() {
        "Image analysis is unavailable: Qwen3-VL-8B-Instruct is not installed under D:\\AI Models\\Keepframe. Recipes will use the deterministic controls-only fallback.".into()
    } else if analysis_runtime_path().is_none() {
        "Image analysis is unavailable: the Keepframe analysis runtime is not installed. Recipes will use the deterministic controls-only fallback.".into()
    } else {
        "The analysis components are installed, but the worker is not ready.".into()
    }
}

fn ensure_analysis_worker_source() -> Result<PathBuf> {
    let source_root = PathBuf::from(r"D:\AI Models\Keepframe\worker-source");
    let package = source_root.join("keepframe_worker");
    fs::create_dir_all(&package)?;
    for (name, contents) in [
        (
            "__init__.py",
            include_str!("../../ai-worker/keepframe_worker/__init__.py"),
        ),
        (
            "main.py",
            include_str!("../../ai-worker/keepframe_worker/main.py"),
        ),
        (
            "schemas.py",
            include_str!("../../ai-worker/keepframe_worker/schemas.py"),
        ),
    ] {
        let destination = package.join(name);
        if fs::read_to_string(&destination).ok().as_deref() != Some(contents) {
            fs::write(destination, contents)?;
        }
    }
    Ok(source_root)
}

fn analysis_installed() -> bool {
    Path::new(r"D:\AI Models\Keepframe\Qwen3-VL-8B-Instruct").exists()
        || Path::new(r"D:\AI Models\Keepframe\huggingface\hub\models--Qwen--Qwen3-VL-8B-Instruct")
            .exists()
}
fn start_analysis_worker(state: &AppState) {
    if state
        .analysis_worker
        .lock()
        .is_ok_and(|worker| worker.child.is_some())
    {
        return;
    }
    if !analysis_installed() {
        if let Ok(mut worker) = state.analysis_worker.lock() {
            worker.last_error = Some(analysis_installation_detail());
        }
        return;
    }
    let Some(worker_python) = analysis_runtime_path() else {
        if let Ok(mut worker) = state.analysis_worker.lock() {
            worker.last_error = Some(analysis_installation_detail());
        }
        return;
    };
    let worker_source = match ensure_analysis_worker_source() {
        Ok(path) => path,
        Err(error) => {
            if let Ok(mut worker) = state.analysis_worker.lock() {
                worker.last_error = Some(format!("Could not prepare the analysis worker: {error}"));
            }
            return;
        }
    };
    let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
        if let Ok(mut worker) = state.analysis_worker.lock() {
            worker.last_error =
                Some("Could not reserve a loopback port for image analysis.".into());
        }
        return;
    };
    let Ok(port) = listener.local_addr().map(|address| address.port()) else {
        return;
    };
    drop(listener);
    let token = Uuid::new_v4().to_string();
    let mut command = hidden_command(worker_python);
    command
        .args(["-m", "uvicorn", "keepframe_worker.main:app", "--app-dir"])
        .arg(worker_source)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .env("KEEPFRAME_MODEL_ROOT", r"D:\AI Models\Keepframe")
        .env("KEEPFRAME_WORKER_TOKEN", &token)
        .env("HF_HOME", r"D:\AI Models\Keepframe\huggingface")
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1");
    match command.spawn() {
        Ok(child) => {
            if let Ok(mut worker) = state.analysis_worker.lock() {
                worker.last_error = None;
                worker.url = Some(format!("http://127.0.0.1:{port}"));
                worker.token = Some(token);
                worker.child = Some(child);
            }
        }
        Err(error) => {
            if let Ok(mut worker) = state.analysis_worker.lock() {
                worker.last_error = Some(format!("The analysis worker could not start: {error}"));
            }
        }
    }
}
fn counts(connection: &Connection) -> Result<Counts> {
    Ok(Counts {
        total: connection.query_row(
            "SELECT count(*) FROM assets WHERE trashed_at IS NULL",
            [],
            |r| r.get(0),
        )?,
        keep: connection.query_row(
            "SELECT count(*) FROM assets WHERE decision='keep' AND trashed_at IS NULL",
            [],
            |r| r.get(0),
        )?,
        undecided: connection.query_row(
            "SELECT count(*) FROM assets WHERE decision='undecided' AND trashed_at IS NULL",
            [],
            |r| r.get(0),
        )?,
        discard: connection.query_row(
            "SELECT count(*) FROM assets WHERE decision='discard' AND trashed_at IS NULL",
            [],
            |r| r.get(0),
        )?,
    })
}

#[tauri::command]
fn get_library_status(state: State<'_, AppState>) -> Result<LibraryStatus> {
    let issue = state
        .library_issue
        .lock()
        .map_err(|_| KeepframeError::Message("Library issue lock failed".into()))?
        .clone();
    let root = state
        .root
        .lock()
        .map_err(|_| KeepframeError::Message("Library state lock failed".into()))?
        .clone();
    let recovery_notice = state
        .recovery_notice
        .lock()
        .map_err(|_| KeepframeError::Message("Recovery notice lock failed".into()))?
        .clone();
    match root {
        Some(root) if db_path(&root).exists() => {
            let c = counts(&open_db(&root)?)?;
            Ok(LibraryStatus {
                configured: true,
                library_root: Some(root.to_string_lossy().into()),
                library_issue: issue,
                recovery_notice,
                counts: c,
            })
        }
        _ => Ok(LibraryStatus {
            configured: false,
            library_root: None,
            library_issue: issue,
            recovery_notice,
            counts: Counts {
                total: 0,
                keep: 0,
                undecided: 0,
                discard: 0,
            },
        }),
    }
}
#[tauri::command]
async fn get_service_health(force: Option<bool>, state: State<'_, AppState>) -> Result<ServiceHealth> {
    if !force.unwrap_or(false) {
        if let Ok(cache) = state.health_cache.lock() {
            if let Some((checked_at, health)) = cache.as_ref() {
                if checked_at.elapsed() < Duration::from_secs(12) {
                    return Ok(health.clone());
                }
            }
        }
    }
    start_analysis_worker(&state);
    let url = state
        .local_ai_url
        .lock()
        .map_err(|_| KeepframeError::Message("AI service setting lock failed".into()))?
        .clone();
    let mut health = local_ai_health(&url).await;
    health.local_ai_url = url.clone();
    let worker_status = state.analysis_worker.lock().ok().map(|mut worker| {
        let running = worker
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !running && worker.child.is_some() {
            worker.child = None;
            worker.url = None;
            worker.token = None;
            worker
                .last_error
                .get_or_insert_with(|| "The analysis worker exited unexpectedly.".into());
        }
        (
            running,
            worker.url.clone(),
            worker.token.clone(),
            worker
                .last_error
                .clone()
                .unwrap_or_else(analysis_installation_detail),
        )
    });
    if let Some((running, worker_url, token, fallback_detail)) = worker_status {
        let ready = if let (true, Some(worker_url), Some(token)) = (running, worker_url, token) {
            match reqwest::Client::new()
                .get(format!("{worker_url}/health"))
                .header("X-Keepframe-Token", token)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => response
                    .json::<Value>()
                    .await
                    .ok()
                    .and_then(|value| value.get("ready").and_then(Value::as_bool))
                    .unwrap_or(false),
                _ => false,
            }
        } else {
            false
        };
        health.analysis_available = ready;
        health.analysis_detail = if ready {
            "Qwen3-VL-8B image analysis is ready and remains on this computer.".into()
        } else if running {
            "The image-analysis worker is starting; Keepframe will use the deterministic fallback until it is ready.".into()
        } else {
            fallback_detail
        };
    }
    if let Ok(mut cache) = state.health_cache.lock() {
        *cache = Some((Instant::now(), health.clone()));
    }
    Ok(health)
}

#[tauri::command]
fn configure_local_ai(url: String, state: State<'_, AppState>) -> Result<()> {
    let url = validated_loopback_url(&url)?;
    let root = root_from(&state)?;
    save_settings(&root, &url)?;
    *state.local_ai_url.lock().map_err(|_| KeepframeError::Message("AI service setting lock failed".into()))? = url;
    if let Ok(mut cache) = state.health_cache.lock() {
        *cache = None;
    }
    Ok(())
}
#[tauri::command]
async fn initialise_library(
    path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<LibraryStatus> {
    let root = PathBuf::from(path);
    let library_lock = acquire_library_lock(&root)?;
    if let Err(error) = initialise_layout(&root) {
        *state.library_issue.lock().unwrap() = Some(error.to_string());
        return Err(error);
    }
    let recovery_notice = recover_interrupted_operations(&root)?;
    *state
        .root
        .lock()
        .map_err(|_| KeepframeError::Message("Library state lock failed".into()))? =
        Some(root.clone());
    *state.library_lock.lock().unwrap() = Some(library_lock);
    *state.library_issue.lock().unwrap() = None;
    *state.recovery_notice.lock().unwrap() = recovery_notice;
    allow_media_scope(&app, &root)?;
    let url = state.local_ai_url.lock().unwrap().clone();
    save_settings(&root, &url)?;
    get_library_status(state)
}

#[tauri::command]
fn check_catalogue_integrity(state: State<'_, AppState>) -> Result<String> {
    database_integrity(&open_db(&root_from(&state)?)?)
}

#[tauri::command]
fn create_catalogue_backup(state: State<'_, AppState>) -> Result<String> {
    Ok(backup_database(&root_from(&state)?, "manual")?
        .to_string_lossy()
        .into())
}

#[tauri::command]
fn restore_catalogue_backup(path: String, state: State<'_, AppState>) -> Result<()> {
    let root = root_from(&state)?;
    let source = PathBuf::from(path);
    if !source.is_file() || database_integrity(&Connection::open(&source)?)? != "ok" {
        return Err(KeepframeError::Message(
            "The selected backup is not a valid SQLite catalogue.".into(),
        ));
    }
    let _safety_backup = backup_database(&root, "before-restore")?;
    let database = db_path(&root);
    let temporary = database.with_extension("restore.sqlite");
    fs::copy(&source, &temporary)?;
    if database_integrity(&Connection::open(&temporary)?)? != "ok" {
        let _ = fs::remove_file(&temporary);
        return Err(KeepframeError::Message(
            "The copied restore candidate failed verification.".into(),
        ));
    }
    {
        let connection = open_db(&root)?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    let displaced = database.with_extension(format!(
        "pre-restore-{}.sqlite",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::rename(&database, &displaced)?;
    if let Err(error) = fs::rename(&temporary, &database) {
        let _ = fs::rename(&displaced, &database);
        return Err(KeepframeError::Io(error));
    }
    match database_integrity(&open_db(&root)?) {
        Ok(result) if result == "ok" => Ok(()),
        _ => {
            let failed = database.with_extension("failed-restore.sqlite");
            let _ = fs::rename(&database, failed);
            fs::rename(displaced, database)?;
            Err(KeepframeError::Message("The restored catalogue failed verification and the prior catalogue was reinstated.".into()))
        }
    }
}

#[tauri::command]
async fn rebuild_thumbnails(state: State<'_, AppState>, app: AppHandle) -> Result<usize> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<usize> {
        let connection = open_db(&root)?;
        let mut statement = connection.prepare(
            "SELECT a.id,a.thumbnail_path,r.path FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.trashed_at IS NULL AND r.id=(SELECT r2.id FROM representations r2 WHERE r2.asset_id=a.id ORDER BY CASE WHEN r2.is_raw=0 THEN 0 ELSE 1 END,r2.path LIMIT 1) ORDER BY a.id",
        )?;
        let assets = statement.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let total = assets.len();
        let mut rebuilt = 0usize;
        for (index, (asset_id, thumbnail, source)) in assets.into_iter().enumerate() {
            let thumbnail = PathBuf::from(thumbnail);
            let temporary = thumbnail.with_extension("jpg.rebuild");
            match thumbnail_from(Path::new(&source), &temporary) {
                Ok(()) => {
                    if thumbnail.exists() { fs::remove_file(&thumbnail)?; }
                    fs::rename(&temporary, &thumbnail)?;
                    rebuilt += 1;
                    let _ = app.emit("cache-progress", json!({"current":index+1,"total":total,"assetId":asset_id}));
                }
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    let _ = app.emit("cache-progress", json!({"current":index+1,"total":total,"assetId":asset_id,"error":error.to_string()}));
                }
            }
        }
        Ok(rebuilt)
    }).await.map_err(|error| KeepframeError::Message(format!("Thumbnail rebuild failed: {error}")))?
}

#[tauri::command]
fn export_diagnostics(destination: String, state: State<'_, AppState>) -> Result<String> {
    let root = root_from(&state)?;
    let destination = PathBuf::from(destination);
    if !destination.is_dir() {
        return Err(KeepframeError::Message(
            "Choose an existing diagnostics destination.".into(),
        ));
    }
    let connection = open_db(&root)?;
    let output = destination.join(format!(
        "keepframe-diagnostics-{}.json",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    let payload = json!({
        "applicationVersion": env!("CARGO_PKG_VERSION"),
        "createdAt": Utc::now().to_rfc3339(),
        "libraryRoot": root,
        "schemaVersion": connection.pragma_query_value(None, "user_version", |row| row.get::<_,i64>(0))?,
        "integrity": database_integrity(&connection)?,
        "counts": counts(&connection)?,
        "privacy": "No image pixels, prompts, recipes or model inputs are included."
    });
    fs::write(&output, serde_json::to_vec_pretty(&payload)?)?;
    Ok(output.to_string_lossy().into())
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn validate_local_output_path(output: &Path, intended_dir: &Path) -> Result<PathBuf> {
    let candidate = if output.is_absolute() {
        output.to_path_buf()
    } else {
        intended_dir.join(output)
    };
    if !candidate.is_file() {
        return Err(KeepframeError::Message(
            "The local editor did not return a readable image file.".into(),
        ));
    }
    let canonical_dir = intended_dir.canonicalize()?;
    let canonical_output = candidate.canonicalize()?;
    if !canonical_output.starts_with(&canonical_dir) {
        return Err(KeepframeError::Message(
            "The local editor returned a file outside Keepframe's intended Edits folder; the result was rejected.".into(),
        ));
    }
    image::open(&canonical_output).map_err(|_| {
        KeepframeError::Message("The local editor returned an invalid image file.".into())
    })?;
    Ok(canonical_output)
}

/// Accept only a complete, decodable, plausibly-sized image that is distinct
/// from the protected source. The caller owns any failed output and removes it.
fn validate_ai_output(output: &Path, source_hash: &str, expected: Option<(u32, u32)>) -> Result<String> {
    let metadata = fs::metadata(output).map_err(|_| KeepframeError::Message("The AI result file was not found.".into()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(KeepframeError::Message("The AI result is empty or incomplete.".into()));
    }
    let image = image::open(output).map_err(|_| KeepframeError::Message("The AI result is not a valid decodable image.".into()))?;
    let (width, height) = image.dimensions();
    if width < 64 || height < 64 {
        return Err(KeepframeError::Message("The AI result dimensions are implausibly small.".into()));
    }
    if let Some((source_width, source_height)) = expected {
        let source_pixels = u64::from(source_width) * u64::from(source_height);
        let output_pixels = u64::from(width) * u64::from(height);
        if output_pixels < source_pixels / 100 || output_pixels > source_pixels.saturating_mul(100) {
            return Err(KeepframeError::Message("The AI result dimensions are implausible for this photograph.".into()));
        }
    }
    let output_hash = hash_file(output)?;
    if output_hash == source_hash {
        return Err(KeepframeError::Message("The returned image is identical to the protected source; no edit was imported.".into()));
    }
    Ok(output_hash)
}

fn verified_representation_path(connection: &Connection, sha256: &str) -> Result<Option<PathBuf>> {
    let mut statement = connection.prepare("SELECT path FROM representations WHERE sha256=?1")?;
    let paths = statement
        .query_map([sha256], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for path in paths {
        let path = PathBuf::from(path);
        if path.is_file() && hash_file(&path).is_ok_and(|managed_hash| managed_hash == sha256) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn path_is_catalogued(connection: &Connection, path: &Path) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM representations WHERE path=?1 LIMIT 1",
            [path.to_string_lossy().as_ref()],
            |row| row.get::<_, i32>(0),
        )
        .optional()?
        .is_some())
}

fn exiftool_path() -> PathBuf {
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/exiftool/exiftool-13.59_64/exiftool.exe");
    if bundled.exists() {
        return bundled;
    }
    if let Ok(executable) = std::env::current_exe() {
        let resource = executable
            .parent()
            .unwrap_or(Path::new("."))
            .join("resources/exiftool/exiftool.exe");
        if resource.exists() {
            return resource;
        }
    }
    PathBuf::from("exiftool")
}

fn libraw_decoder_path() -> PathBuf {
    const RELATIVE: &str = "resources/libraw/libraw-0.22.2-win64/dcraw_emu.exe";
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(RELATIVE);
    if bundled.is_file() {
        return bundled;
    }
    if let Ok(executable) = std::env::current_exe() {
        let executable_dir = executable.parent().unwrap_or(Path::new("."));
        for relative in [RELATIVE, "resources/libraw/dcraw_emu.exe"] {
            let candidate = executable_dir.join(relative);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("dcraw_emu.exe")
}
fn parse_exif_date(value: &str) -> Option<DateTime<Utc>> {
    for format in ["%Y:%m:%d %H:%M:%S%:z", "%Y:%m:%d %H:%M:%S"] {
        if let Ok(dt) = DateTime::parse_from_str(value, format) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, format) {
            return Some(Utc.from_utc_datetime(&dt));
        }
    }
    None
}
#[derive(Default)]
struct Meta {
    captured: Option<DateTime<Utc>>,
    fallback: bool,
    camera: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    keywords: Vec<String>,
}
fn metadata(path: &Path) -> Meta {
    let mut meta = Meta::default();
    if let Ok(output) = hidden_command(exiftool_path())
        .args([
            "-json",
            "-n",
            "-DateTimeOriginal",
            "-CreateDate",
            "-Model",
            "-ImageWidth",
            "-ImageHeight",
            "-GPSLatitude",
            "-GPSLongitude",
            "-Keywords",
            "-Subject",
        ])
        .arg(path)
        .output()
    {
        if output.status.success() {
            if let Ok(values) = serde_json::from_slice::<Vec<Value>>(&output.stdout) {
                if let Some(v) = values.first() {
                    meta.captured = v
                        .get("DateTimeOriginal")
                        .or_else(|| v.get("CreateDate"))
                        .and_then(Value::as_str)
                        .and_then(parse_exif_date);
                    meta.camera = v.get("Model").and_then(Value::as_str).map(str::to_string);
                    meta.width = v
                        .get("ImageWidth")
                        .and_then(Value::as_u64)
                        .map(|n| n as u32);
                    meta.height = v
                        .get("ImageHeight")
                        .and_then(Value::as_u64)
                        .map(|n| n as u32);
                    meta.latitude = v.get("GPSLatitude").and_then(Value::as_f64);
                    meta.longitude = v.get("GPSLongitude").and_then(Value::as_f64);
                    for key in ["Keywords", "Subject"] {
                        if let Some(value) = v.get(key) {
                            match value {
                                Value::Array(items) => meta.keywords.extend(
                                    items.iter().filter_map(Value::as_str).map(str::to_string),
                                ),
                                Value::String(item) => meta.keywords.push(item.clone()),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    if meta.width.is_none() {
        if let Ok(reader) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
            if let Ok(image) = reader.decode() {
                meta.width = Some(image.width());
                meta.height = Some(image.height());
            }
        }
    }
    if meta.captured.is_none() {
        meta.fallback = true;
        meta.captured = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from);
    }
    meta
}
fn decode_browsing_image(path: &Path) -> Result<image::DynamicImage> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if RAW.contains(&extension.as_str()) || extension == "heic" {
        let mut decoded = None;
        for tag in ["-JpgFromRaw", "-PreviewImage", "-ThumbnailImage"] {
            if let Ok(result) = hidden_command(exiftool_path())
                .args(["-b", tag])
                .arg(path)
                .output()
            {
                if result.status.success() && !result.stdout.is_empty() {
                    if let Ok(img) = image::load_from_memory(&result.stdout) {
                        decoded = Some(img);
                        break;
                    }
                }
            }
        }
        let mut image = decoded.ok_or_else(|| {
            KeepframeError::Message(format!("No usable preview was found in {}", path.display()))
        })?;
        if let Some(orientation) = source_orientation(path) {
            image.apply_orientation(orientation);
        }
        Ok(image)
    } else {
        decode_standard_with_orientation(path)
    }
}

fn source_orientation(path: &Path) -> Option<image::metadata::Orientation> {
    let output = hidden_command(exiftool_path())
        .args(["-s3", "-n", "-Orientation"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u8>()
        .ok()?;
    image::metadata::Orientation::from_exif(value)
}

fn decode_standard_with_orientation(path: &Path) -> Result<image::DynamicImage> {
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn thumbnail_from(path: &Path, output: &Path) -> Result<()> {
    let image = decode_browsing_image(path)?;
    let thumb = image.thumbnail(1600, 1200).to_rgb8();
    image::save_buffer_with_format(
        output,
        &thumb,
        thumb.width(),
        thumb.height(),
        image::ColorType::Rgb8,
        ImageFormat::Jpeg,
    )?;
    Ok(())
}

fn verify_full_resolution(
    image: &image::DynamicImage,
    expected: Option<(u32, u32)>,
    source: &Path,
) -> Result<()> {
    let Some((expected_width, expected_height)) = expected else {
        return Ok(());
    };
    if expected_width == 0 || expected_height == 0 {
        return Ok(());
    }
    let expected_pixels = u64::from(expected_width) * u64::from(expected_height);
    let actual_pixels = u64::from(image.width()) * u64::from(image.height());
    // Allow for normal sensor-edge cropping while rejecting embedded previews.
    if actual_pixels * 100 < expected_pixels * 80 {
        return Err(KeepframeError::Message(format!(
            "Full-resolution preparation for {} produced only {}x{} pixels; the catalogue records {}x{}. No preview was substituted.",
            source.display(),
            image.width(),
            image.height(),
            expected_width,
            expected_height
        )));
    }
    Ok(())
}

fn decode_raw_full_resolution(path: &Path, working_dir: &Path) -> Result<image::DynamicImage> {
    let decoder = libraw_decoder_path();
    if !decoder.is_file() {
        return Err(KeepframeError::Message(format!(
            "The bundled LibRaw decoder is unavailable at {}. Keepframe will not substitute an embedded preview.",
            decoder.display()
        )));
    }
    fs::create_dir_all(working_dir)?;
    let output_path = working_dir.join(format!("raw-{}.tiff", Uuid::new_v4()));
    let decode_result = (|| {
        let output = hidden_command(&decoder)
            .args(["-w", "+M", "-o", "1", "-q", "3", "-6", "-T", "-Z"])
            .arg(&output_path)
            .arg(path)
            .output()?;
        if !output.status.success() || !output_path.is_file() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if detail.is_empty() {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                detail
            };
            return Err(KeepframeError::Message(format!(
                "LibRaw could not decode {} at full resolution{}",
                path.display(),
                if detail.is_empty() {
                    ".".into()
                } else {
                    format!(": {detail}")
                }
            )));
        }
        image::open(&output_path).map_err(KeepframeError::from)
    })();
    let _ = fs::remove_file(&output_path);
    decode_result
}

fn prepare_full_resolution_image(
    path: &Path,
    expected: Option<(u32, u32)>,
    working_dir: &Path,
) -> Result<image::DynamicImage> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let image = if RAW.contains(&extension.as_str()) {
        decode_raw_full_resolution(path, working_dir)?
    } else if extension == "heic" {
        return Err(KeepframeError::Message(format!(
            "Full-resolution HEIC decoding is not yet available for {}. Keepframe will not export its thumbnail instead.",
            path.display()
        )));
    } else {
        decode_standard_with_orientation(path).map_err(|error| {
            KeepframeError::Message(format!(
                "Could not open {} at full resolution: {error}",
                path.display()
            ))
        })?
    };
    verify_full_resolution(&image, expected, path)?;
    Ok(image)
}

fn encode_srgb_png(image: &image::DynamicImage) -> Result<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, rgb.width(), rgb.height());
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.write_header()?.write_image_data(rgb.as_raw())?;
    }
    Ok(bytes)
}

fn adjustment_input(connection: &Connection, asset_id: &str) -> Result<AdjustmentInput> {
    let (captured, width, height, preferred): (String, Option<u32>, Option<u32>, Option<String>) = connection.query_row(
        "SELECT captured_at,width,height,preferred_version_id FROM assets WHERE id=?1 AND trashed_at IS NULL",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if let Some(version_id) = preferred {
        let (path, stored_hash): (String, Option<String>) = connection.query_row(
            "SELECT path,output_hash FROM versions WHERE id=?1 AND asset_id=?2 AND state!='rejected'",
            params![version_id, asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let path = PathBuf::from(path);
        let source_hash = stored_hash.unwrap_or(hash_file(&path)?);
        return Ok(AdjustmentInput {
            path,
            source_hash,
            captured_at: captured,
            expected_dimensions: None,
        });
    }
    let (path, source_hash): (String, String) = connection.query_row(
        "SELECT path,sha256 FROM representations WHERE asset_id=?1 ORDER BY CASE WHEN lower(extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN is_raw=1 THEN 1 ELSE 2 END,path LIMIT 1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(AdjustmentInput {
        path: PathBuf::from(path),
        source_hash,
        captured_at: captured,
        expected_dimensions: width.zip(height),
    })
}

fn protected_endpoints(image: &RgbImage) -> (f32, f32) {
    let mut histogram = [0_u64; 256];
    for pixel in image.pixels() {
        let luminance =
            (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722)
                .round()
                .clamp(0.0, 255.0) as usize;
        histogram[luminance] += 1;
    }
    let total = image.width() as u64 * image.height() as u64;
    let tail = ((total as f64 * 0.0005).round() as u64).max(1);
    let percentile = |target: u64| {
        let mut seen = 0_u64;
        for (index, count) in histogram.iter().enumerate() {
            seen += count;
            if seen >= target {
                return index as f32 / 255.0;
            }
        }
        1.0
    };
    (percentile(tail), percentile(total.saturating_sub(tail)))
}

fn protected_expand(value: f32, low: f32, high: f32) -> f32 {
    if high - low < 0.02 {
        return value;
    }
    ((value - low) / (high - low)).clamp(0.0, 1.0)
}

fn automatic_tone_luminance(value: f32, low: f32, high: f32) -> f32 {
    protected_expand(value, low, high)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let position = ((value - edge0) / (edge1 - edge0).max(0.000_001)).clamp(0.0, 1.0);
    position * position * (3.0 - 2.0 * position)
}

fn tone_adjusted_luminance(value: f32, adjustments: BasicAdjustments) -> f32 {
    let mut result = value;
    let contrast = adjustments.contrast / 100.0;
    result -= contrast * 0.11 * (std::f32::consts::TAU * result).sin();

    let shadows = adjustments.shadows / 100.0;
    let highlights = adjustments.highlights / 100.0;
    result += shadows
        * 0.30
        * (1.0 - result).powi(2)
        * (1.0 - smoothstep(0.48, 0.72, result))
        * smoothstep(0.005, 0.10, result);
    result += highlights
        * 0.24
        * result.powi(2)
        * smoothstep(0.35, 0.75, result)
        * (1.0 - smoothstep(0.90, 0.995, result));

    let black_mask = 1.0 - smoothstep(0.02, 0.30, result);
    let white_mask = smoothstep(0.70, 0.98, result);
    result += adjustments.blacks / 100.0 * 0.13 * black_mask;
    result += adjustments.whites / 100.0 * 0.13 * white_mask;

    let region = |value: f32, centre: f32, width: f32| {
        let distance = (value - centre) / width;
        (-0.5 * distance * distance).exp()
    };
    result += adjustments.curve_shadows / 100.0 * 0.10 * region(result, 0.14, 0.14);
    result += adjustments.curve_darks / 100.0 * 0.10 * region(result, 0.36, 0.18);
    result += adjustments.curve_lights / 100.0 * 0.10 * region(result, 0.64, 0.18);
    result += adjustments.curve_highlights / 100.0 * 0.10 * region(result, 0.86, 0.14);
    result.clamp(0.0, 1.0)
}

fn remap_luminance(values: &mut [f32; 3], current: f32, target: f32) {
    if target >= current {
        let amount = (target - current) / (1.0 - current).max(0.000_001);
        for value in values {
            *value += (1.0 - *value) * amount;
        }
    } else {
        let amount = target / current.max(0.000_001);
        for value in values {
            *value *= amount;
        }
    }
}

fn apply_protected_local_contrast(image: &mut RgbImage, amount: f32, radius_divisor: f32) {
    if amount.abs() < f32::EPSILON || image.width() < 8 || image.height() < 8 {
        return;
    }
    let sigma = (image.width().min(image.height()) as f32 / radius_divisor).clamp(1.2, 48.0);
    let blurred = image::imageops::blur(image, sigma);
    for (pixel, blurred_pixel) in image.pixels_mut().zip(blurred.pixels()) {
        let mut values = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let current = values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
        let local_average = blurred_pixel[0] as f32 / 255.0 * 0.2126
            + blurred_pixel[1] as f32 / 255.0 * 0.7152
            + blurred_pixel[2] as f32 / 255.0 * 0.0722;
        // Fade the local-contrast correction near pure black and white so it
        // cannot turn the protected endpoint placement into broad clipping.
        let endpoint_protection = (4.0 * current * (1.0 - current)).clamp(0.0, 1.0);
        let target = (current + (current - local_average) * amount * endpoint_protection)
            .clamp(0.0, 1.0);
        remap_luminance(&mut values, current, target);
        for (index, value) in values.iter().enumerate() {
            pixel[index] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

fn apply_adjustments_to_image(
    image: &image::DynamicImage,
    adjustments: BasicAdjustments,
) -> RgbImage {
    let adjustments = adjustments.validate().expect("validated adjustment values");
    let mut output = image.to_rgb8();
    let (low, high) = protected_endpoints(&output);
    let range_amount = (adjustments.dynamic_range / 100.0).clamp(-1.0, 1.0);
    let temperature = adjustments.light_balance / 100.0 * 0.18;
    let tint = adjustments.tint / 100.0 * 0.10;
    let exposure = 2_f32.powf(adjustments.exposure);
    let colour = adjustments.colour_boost / 100.0;
    let saturation_adjustment = adjustments.saturation / 100.0;
    let dehaze = adjustments.dehaze / 100.0;
    for pixel in output.pixels_mut() {
        let mut values = [
            pixel[0] as f32 / 255.0 * (1.0 + temperature + tint * 0.5),
            pixel[1] as f32 / 255.0 * (1.0 - tint),
            pixel[2] as f32 / 255.0 * (1.0 - temperature + tint * 0.5),
        ];
        for value in &mut values {
            *value = value.clamp(0.0, 1.0);
        }
        let current_luminance = values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
        let target_luminance = if range_amount >= 0.0 {
            let automatic = automatic_tone_luminance(current_luminance, low, high);
            current_luminance + (automatic - current_luminance) * range_amount
        } else {
            0.5 + (current_luminance - 0.5) * (1.0 + range_amount * 0.5)
        };
        remap_luminance(&mut values, current_luminance, target_luminance);
        for value in &mut values {
            *value = (*value * exposure).clamp(0.0, 1.0);
        }
        let exposed_luminance = values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
        let mut tonal_target = tone_adjusted_luminance(exposed_luminance, adjustments);
        tonal_target = 0.5 + (tonal_target - 0.5) * (1.0 + dehaze * 0.20);
        remap_luminance(&mut values, exposed_luminance, tonal_target.clamp(0.0, 1.0));
        let maximum = values.iter().copied().fold(0.0_f32, f32::max);
        let minimum = values.iter().copied().fold(1.0_f32, f32::min);
        let luminance = values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
        let saturation = if maximum > 0.0 {
            (maximum - minimum) / maximum
        } else {
            0.0
        };
        let mut colour_factor = if colour >= 0.0 {
            1.0 + colour * 1.2 * (1.0 - saturation)
        } else {
            1.0 + colour
        };
        colour_factor *= (1.0 + saturation_adjustment).max(0.0);
        if maximum > luminance {
            colour_factor = colour_factor.min((1.0 - luminance) / (maximum - luminance));
        }
        if minimum < luminance {
            colour_factor = colour_factor.min(luminance / (luminance - minimum));
        }
        for (index, value) in values.iter().enumerate() {
            pixel[index] = ((luminance + (*value - luminance) * colour_factor).clamp(0.0, 1.0)
                * 255.0)
                .round() as u8;
        }
    }
    apply_protected_local_contrast(&mut output, adjustments.texture / 100.0 * 0.28, 220.0);
    apply_protected_local_contrast(&mut output, adjustments.clarity / 100.0 * 0.42, 75.0);
    apply_protected_local_contrast(&mut output, dehaze * 0.18, 48.0);
    apply_geometry(&output, adjustments)
}

fn apply_geometry(source: &RgbImage, adjustments: BasicAdjustments) -> RgbImage {
    let mut image = source.clone();
    for _ in 0..adjustments.rotate_quadrants {
        image = image::imageops::rotate90(&image);
    }
    if adjustments.straighten.abs() >= 0.01 {
        let angle = adjustments.straighten.to_radians();
        let (width, height) = image.dimensions();
        let centre_x = (width as f32 - 1.0) / 2.0;
        let centre_y = (height as f32 - 1.0) / 2.0;
        let (sin, cos) = angle.sin_cos();
        let mut rotated = RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let dx = x as f32 - centre_x;
                let dy = y as f32 - centre_y;
                let source_x = (cos * dx + sin * dy + centre_x).round() as i32;
                let source_y = (-sin * dx + cos * dy + centre_y).round() as i32;
                if source_x >= 0 && source_y >= 0 && source_x < width as i32 && source_y < height as i32 {
                    *rotated.get_pixel_mut(x, y) = *image.get_pixel(source_x as u32, source_y as u32);
                }
            }
        }
        image = rotated;
    }
    let (left, top, crop_width, crop_height) = adjustments.normalised_crop();
    let width = image.width();
    let height = image.height();
    let x = (left * width as f32).round() as u32;
    let y = (top * height as f32).round() as u32;
    let crop_w = ((crop_width * width as f32).round() as u32).clamp(1, width.saturating_sub(x).max(1));
    let crop_h = ((crop_height * height as f32).round() as u32).clamp(1, height.saturating_sub(y).max(1));
    image::imageops::crop_imm(&image, x.min(width - 1), y.min(height - 1), crop_w, crop_h).to_image()
}

fn suggested_basic_adjustments(image: &image::DynamicImage) -> BasicAdjustments {
    let rgb = image.to_rgb8();
    let step = ((rgb.width() as usize * rgb.height() as usize) / 250_000).max(1);
    let mut saturation_total = 0.0_f64;
    let mut luminances = Vec::new();
    let mut samples = 0_u64;
    for pixel in rgb.pixels().step_by(step) {
        let maximum = *pixel.0.iter().max().unwrap_or(&0) as f64 / 255.0;
        let minimum = *pixel.0.iter().min().unwrap_or(&0) as f64 / 255.0;
        saturation_total += if maximum > 0.0 {
            (maximum - minimum) / maximum
        } else {
            0.0
        };
        luminances.push((pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722) / 255.0);
        samples += 1;
    }
    luminances.sort_by(|left, right| left.total_cmp(right));
    let percentile = |fraction: f32| -> f32 {
        if luminances.is_empty() { return 0.5; }
        luminances[((luminances.len() - 1) as f32 * fraction).round() as usize]
    };
    let median = percentile(0.50);
    let dark = percentile(0.01);
    let bright = percentile(0.99);
    let average_saturation = if samples > 0 {
        saturation_total / samples as f64
    } else {
        0.0
    };
    BasicAdjustments {
        // Do not darken an already bright photograph merely to centre its
        // median: that would pull the measured white point away from white.
        exposure: (0.45 / median.max(0.05)).log2().clamp(0.0, 0.75),
        light_balance: 0.0,
        tint: 0.0,
        contrast: if bright - dark < 0.65 { 10.0 } else { 5.0 },
        highlights: (-18.0 - bright * 24.0).clamp(-45.0, -18.0),
        shadows: (8.0 + (0.18 - dark).max(0.0) * 100.0).clamp(8.0, 28.0),
        whites: ((0.98 - bright) * 80.0).clamp(2.0, 14.0),
        blacks: (-4.0 - dark * 30.0).clamp(-12.0, -4.0),
        dynamic_range: 100.0,
        texture: 4.0,
        clarity: 7.0,
        dehaze: 3.0,
        colour_boost: if average_saturation < 0.2 {
            12.0
        } else if average_saturation < 0.4 {
            9.0
        } else {
            6.0
        },
        saturation: 2.0,
        curve_highlights: 0.0,
        curve_lights: 3.0,
        curve_darks: -2.0,
        curve_shadows: 0.0,
        crop_left: 0.0,
        crop_top: 0.0,
        crop_width: 1.0,
        crop_height: 1.0,
        rotate_quadrants: 0,
        straighten: 0.0,
    }
}

#[tauri::command]
async fn auto_basic_adjustments(
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<BasicAdjustments> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<BasicAdjustments> {
        let connection = open_db(&root)?;
        let preview: String = connection.query_row(
            "SELECT thumbnail_path FROM assets WHERE id=?1 AND trashed_at IS NULL",
            [&asset_id],
            |row| row.get(0),
        )?;
        let image = image::open(preview)?;
        Ok(suggested_basic_adjustments(&image))
    })
    .await
    .map_err(|error| {
        KeepframeError::Message(format!("Automatic adjustment analysis failed: {error}"))
    })?
}

#[tauri::command]
async fn preview_basic_adjustments(
    asset_id: String,
    adjustments: BasicAdjustments,
    state: State<'_, AppState>,
) -> Result<String> {
    let adjustments = adjustments.validate()?;
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<String> {
        let connection = open_db(&root)?;
        let preview: String = connection.query_row(
            "SELECT thumbnail_path FROM assets WHERE id=?1 AND trashed_at IS NULL",
            [&asset_id],
            |row| row.get(0),
        )?;
        let image = image::open(preview)?;
        let adjusted =
            image::DynamicImage::ImageRgb8(apply_adjustments_to_image(&image, adjustments));
        let settings = serde_json::to_vec(&adjustments)?;
        let mut digest = Sha256::new();
        digest.update(b"photographic-controls-v5");
        digest.update(asset_id.as_bytes());
        digest.update(settings);
        let key = format!("{:x}", digest.finalize());
        let output = root.join(".keepframe/previews").join(format!(
            "adjustment-{}-{}.png",
            asset_id,
            &key[..12]
        ));
        if !output.exists() {
            let temporary = output.with_extension("png.tmp");
            fs::write(&temporary, encode_srgb_png(&adjusted)?)?;
            fs::rename(&temporary, &output)?;
        }
        Ok(output.to_string_lossy().into())
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Adjustment preview failed: {error}")))?
}

#[tauri::command]
async fn apply_basic_adjustments(
    asset_id: String,
    adjustments: BasicAdjustments,
    state: State<'_, AppState>,
) -> Result<AssetVersion> {
    let adjustments = adjustments.validate()?;
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<AssetVersion> {
        let mut connection = open_db(&root)?;
        let input = adjustment_input(&connection, &asset_id)?;
        let image = prepare_full_resolution_image(&input.path, input.expected_dimensions, &root.join(".keepframe/staging/working"))?;
        let adjusted = image::DynamicImage::ImageRgb8(apply_adjustments_to_image(&image, adjustments));
        let captured = DateTime::parse_from_rfc3339(&input.captured_at).map(|value| value.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now());
        let version_id = Uuid::new_v4().to_string();
        let output_dir = root.join("Edits").join(format!("{:04}", captured.year())).join(format!("{:02}", captured.month())).join(format!("{:02}", captured.day())).join(&asset_id);
        fs::create_dir_all(&output_dir)?;
        let output = output_dir.join(format!("adjusted-{}-{}.png", Local::now().format("%H%M%S"), &version_id[..8]));
        let temporary = output.with_extension("png.tmp");
        fs::write(&temporary, encode_srgb_png(&adjusted)?)?;
        fs::rename(&temporary, &output)?;
        let output_hash = hash_file(&output)?;
        let now = Utc::now().to_rfc3339();
        let summary = format!(
            "Exposure {:+.2} EV; temperature {:+.0}; tint {:+.0}; contrast {:+.0}; highlights {:+.0}; shadows {:+.0}; whites {:+.0}; blacks {:+.0}; texture {:+.0}; clarity {:+.0}; dehaze {:+.0}; vibrance {:+.0}; saturation {:+.0}",
            adjustments.exposure, adjustments.light_balance, adjustments.tint,
            adjustments.contrast, adjustments.highlights, adjustments.shadows,
            adjustments.whites, adjustments.blacks, adjustments.texture,
            adjustments.clarity, adjustments.dehaze, adjustments.colour_boost,
            adjustments.saturation
        );
        let recipe_json = serde_json::to_string(&adjustments)?;
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO versions(id,asset_id,kind,path,provider,prompt,recipe_json,source_hash,output_hash,state,created_at)VALUES(?1,?2,'adjusted',?3,'keepframe-controls',?4,?5,?6,?7,'candidate',?8)", params![version_id,asset_id,output.to_string_lossy(),summary,recipe_json,input.source_hash,output_hash,now])?;
        tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,new_json,created_at)VALUES('version',?1,'create_adjusted_version',?2,?3)", params![version_id,recipe_json,now])?;
        tx.commit()?;
        Ok(AssetVersion { id: version_id, kind: "adjusted".into(), provider: Some("keepframe-controls".into()), created_at: now, state: "candidate".into(), image_url: output.to_string_lossy().into(), prompt: Some(summary), is_preferred: false, source_hash: Some(input.source_hash), output_hash: Some(output_hash) })
    }).await.map_err(|error| KeepframeError::Message(format!("Saving adjusted version failed: {error}")))?
}

#[tauri::command]
async fn prepare_review_preview(asset_id: String, state: State<'_, AppState>) -> Result<String> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<String> {
        let connection = open_db(&root)?;
        let (source, source_hash, width, height): (String, String, Option<u32>, Option<u32>) = connection.query_row(
            "SELECT r.path,r.sha256,a.width,a.height FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.id=?1 AND a.trashed_at IS NULL ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.path LIMIT 1",
            [&asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let output = root.join(".keepframe/review").join(format!("{}-{}.png", asset_id, &source_hash[..12]));
        if !output.exists() {
            let image = prepare_full_resolution_image(Path::new(&source), width.zip(height), &root.join(".keepframe/staging/working"))?;
            let temporary = output.with_extension("png.tmp");
            fs::write(&temporary, encode_srgb_png(&image)?)?;
            fs::rename(&temporary, &output)?;
        }
        Ok(output.to_string_lossy().into())
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Full-resolution review failed: {error}")))?
}

#[tauri::command]
async fn import_photos(
    paths: Vec<String>,
    options: ImportOptions,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportSummary> {
    options.validate()?;
    let root = root_from(&state)?;
    let canonical_root = root.canonicalize()?;
    let import_id = Uuid::new_v4().to_string();
    let mut files = Vec::new();
    let mut unsupported = 0usize;
    for raw in &paths {
        let path = PathBuf::from(raw);
        if !path.exists() {
            unsupported += 1;
            continue;
        }
        if path.canonicalize()?.starts_with(&canonical_root) {
            return Err(KeepframeError::Message(
                "The master library cannot be imported into itself.".into(),
            ));
        }
        if path.is_file() {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if SUPPORTED.contains(&extension.as_str()) {
                files.push(path)
            } else {
                unsupported += 1;
            }
        } else {
            for entry in WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let extension = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if SUPPORTED.contains(&extension.as_str()) {
                    files.push(entry.into_path())
                } else {
                    unsupported += 1;
                }
            }
        }
    }
    let required_bytes = files.iter().try_fold(0u64, |total, path| {
        let bytes = fs::metadata(path)?.len();
        total.checked_add(bytes).ok_or_else(|| {
            KeepframeError::Message("Import size overflowed the safety check.".into())
        })
    })?;
    let available_bytes = fs2::available_space(&root)?;
    safety::ensure_capacity(required_bytes, available_bytes)?;
    let items = files
        .into_iter()
        .map(|path| (Uuid::new_v4().to_string(), path))
        .collect::<Vec<_>>();
    {
        let mut connection = open_db(&root)?;
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO imports(id,source,started_at,discovered,unsupported,state,mode,duplicate_policy) VALUES(?1,?2,?3,?4,?5,'running',?6,?7)",params![import_id,paths.join(";"),Utc::now().to_rfc3339(),items.len() as i64,unsupported as i64,options.mode,options.duplicate_source_policy])?;
        for (item_id, source) in &items {
            let source_metadata = fs::metadata(source)?;
            tx.execute(
                "INSERT INTO import_items(id,import_id,source_path,state,source_size,source_modified,source_action)VALUES(?1,?2,?3,'discovered',?4,?5,'pending')",
                params![item_id, import_id, source.to_string_lossy(), source_metadata.len() as i64, source_metadata.modified().ok().map(DateTime::<Utc>::from).map(|value|value.to_rfc3339())],
            )?;
        }
        tx.commit()?;
    }
    let mut imported = 0;
    let mut copied = 0;
    let mut moved = 0;
    let mut source_retained = 0;
    let mut duplicates = 0;
    let mut failed = 0;
    let mut final_state = "completed".to_string();
    for (index, (item_id, source)) in items.iter().enumerate() {
        let cancelled: bool = open_db(&root)?.query_row(
            "SELECT cancel_requested!=0 FROM imports WHERE id=?1",
            [&import_id],
            |row| row.get(0),
        )?;
        if cancelled {
            final_state = "cancelled".into();
            break;
        }
        let _ = app.emit(
            "import-progress",
            json!({"importId":import_id,"current":index+1,"total":items.len(),"file":source}),
        );
        let result = (|| -> Result<(bool, String, String, Option<PathBuf>)> {
            let source_hash = hash_file(source)?;
            let mut connection = open_db(&root)?;
            if let Some(managed_path) = verified_representation_path(&connection, &source_hash)? {
                let tx = connection.transaction()?;
                tx.execute("UPDATE import_items SET state='duplicate',source_hash=?2,source_action='retained' WHERE id=?1",params![item_id,source_hash])?;
                tx.commit()?;
                return Ok((true, item_id.clone(), source_hash, Some(managed_path)));
            }
            let meta = metadata(source);
            let captured = meta.captured.unwrap_or_else(Utc::now);
            let stage_dir = root.join(".keepframe/staging").join(&import_id);
            fs::create_dir_all(&stage_dir)?;
            let source_name = source
                .file_name()
                .ok_or_else(|| KeepframeError::Message("Source has no filename".into()))?;
            let staged = stage_dir.join(format!("{}-{}", item_id, source_name.to_string_lossy()));
            connection.execute("UPDATE import_items SET state='staging',staging_path=?2,source_hash=?3,error=NULL WHERE id=?1",params![item_id,staged.to_string_lossy(),source_hash])?;
            safety::copy_and_verify(source, &staged, &source_hash)?;
            connection.execute(
                "UPDATE import_items SET state='verified' WHERE id=?1",
                [&item_id],
            )?;
            let target_dir = root
                .join("Originals")
                .join(format!("{:04}", captured.year()))
                .join(format!("{:02}", captured.month()))
                .join(format!("{:02}", captured.day()));
            fs::create_dir_all(&target_dir)?;
            let original_name = source.file_name().unwrap().to_string_lossy().to_string();
            let mut target = target_dir.join(&original_name);
            if target.exists() || path_is_catalogued(&connection, &target)? {
                let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                let extension = source.extension().unwrap_or_default().to_string_lossy();
                let mut suffix = 0usize;
                loop {
                    let discriminator = if suffix == 0 {
                        source_hash[..8].to_string()
                    } else {
                        format!("{}_{}", &source_hash[..8], suffix)
                    };
                    let candidate = target_dir.join(format!("{stem}_{discriminator}.{extension}"));
                    if !candidate.exists() && !path_is_catalogued(&connection, &candidate)? {
                        target = candidate;
                        break;
                    }
                    suffix += 1;
                }
            }
            safety::promote_and_verify(&staged, &target, &source_hash)?;
            connection.execute(
                "UPDATE import_items SET state='placed',managed_path=?2 WHERE id=?1",
                params![item_id, target.to_string_lossy()],
            )?;
            let stem = source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let extension = source
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let is_raw = RAW.contains(&extension.as_str());
            let captured_string = captured.to_rfc3339();
            let paired:Option<String>=connection.query_row("SELECT a.id FROM assets a JOIN representations r ON r.asset_id=a.id WHERE lower(r.stem)=?1 AND a.captured_at=?2 AND r.is_raw!=?3 LIMIT 1",params![stem,captured_string,is_raw],|r|r.get(0)).optional()?;
            let asset_id = paired.unwrap_or_else(|| Uuid::new_v4().to_string());
            let thumb = root
                .join(".keepframe/thumbnails")
                .join(format!("{}.jpg", asset_id));
            let mut created_thumb = false;
            if !thumb.exists() {
                if let Err(error) = thumbnail_from(&target, &thumb) {
                    let _ = fs::remove_file(&target);
                    return Err(error);
                }
                created_thumb = true;
            }
            let asset_exists = connection
                .query_row("SELECT 1 FROM assets WHERE id=?1", [&asset_id], |r| {
                    r.get::<_, i32>(0)
                })
                .optional()?
                .is_some();
            let representation_id = Uuid::new_v4().to_string();
            let byte_size = fs::metadata(&target)?.len() as i64;
            let persist_result = (|| -> Result<()> {
                let tx = connection.transaction()?;
                if !asset_exists {
                    let location_source = if meta.latitude.is_some() && meta.longitude.is_some() { "embedded" } else { "none" };
                    tx.execute("INSERT INTO assets(id,filename,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,location_source,thumbnail_path,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?8,?9,?10,?11,?12)",params![asset_id,original_name,captured_string,meta.fallback,meta.camera,meta.width,meta.height,meta.latitude,meta.longitude,location_source,thumb.to_string_lossy(),Utc::now().to_rfc3339()])?;
                }
                tx.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![representation_id,asset_id,target.to_string_lossy(),source_hash,extension,stem,byte_size,RAW.contains(&extension.as_str())])?;
                for name in meta.keywords {
                    let tag_id = tx
                        .query_row("SELECT id FROM tags WHERE name=?1", [&name], |r| {
                            r.get::<_, String>(0)
                        })
                        .optional()?
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    tx.execute(
                        "INSERT OR IGNORE INTO tags(id,name)VALUES(?1,?2)",
                        params![tag_id, name],
                    )?;
                    tx.execute(
                        "INSERT OR IGNORE INTO asset_tags(asset_id,tag_id)VALUES(?1,?2)",
                        params![asset_id, tag_id],
                    )?;
                }
                tx.execute(
                    "UPDATE import_items SET representation_id=?2,state='catalogued' WHERE id=?1",
                    params![item_id, representation_id],
                )?;
                tx.commit()?;
                Ok(())
            })();
            if let Err(error) = persist_result {
                let _ = fs::remove_file(&target);
                if created_thumb {
                    let _ = fs::remove_file(&thumb);
                }
                return Err(error);
            }
            Ok((false, item_id.clone(), source_hash, Some(target)))
        })();
        match result {
            Ok((is_duplicate, item_id, source_hash, managed_path)) => {
                let remove_source = options.mode == "move"
                    && (!is_duplicate
                        || options.duplicate_source_policy == "remove_after_verified_match");
                let source_action = if remove_source {
                    safety::delete_verified_external_source(
                        source,
                        managed_path.as_deref().ok_or_else(|| KeepframeError::Message(
                            "No verified managed copy was available; the source was retained.".into(),
                        ))?,
                        &source_hash,
                        &root,
                    )
                } else {
                    Ok(())
                };
                match source_action {
                    Ok(()) => {
                        if is_duplicate {
                            duplicates += 1;
                        } else {
                            imported += 1;
                            if remove_source {
                                moved += 1;
                            } else {
                                copied += 1;
                            }
                        }
                        if !remove_source {
                            source_retained += 1;
                        }
                        let connection = open_db(&root)?;
                        connection.execute(
                            "UPDATE import_items SET state=?2,source_action=?3,error=NULL WHERE id=?1",
                            params![item_id,if is_duplicate { "duplicate" } else { "completed" },if remove_source { "removed" } else { "retained" }],
                        )?;
                    }
                    Err(error) => {
                        if is_duplicate {
                            duplicates += 1;
                        } else {
                            imported += 1;
                            copied += 1;
                        }
                        source_retained += 1;
                        let connection = open_db(&root)?;
                        connection.execute(
                        "UPDATE import_items SET state='source_retained',error=?2 WHERE id=?1",
                        params![item_id, format!("Catalogue commit succeeded but the source was retained: {error}")],
                    )?;
                    }
                }
            }
            Err(error) => {
                failed += 1;
                let connection = open_db(&root)?;
                connection.execute(
                    "UPDATE import_items SET state='failed',error=?2 WHERE id=?1",
                    params![item_id, error.to_string()],
                )?;
            }
        }
    }
    let connection = open_db(&root)?;
    connection.execute(
        "UPDATE imports SET completed_at=?2,imported=?3,duplicates=?4,failed=?5,state=?6 WHERE id=?1",
        params![
            import_id,
            Utc::now().to_rfc3339(),
            imported as i64,
            duplicates as i64,
            failed as i64,
            final_state
        ],
    )?;
    if final_state == "completed" && failed == 0 {
        let _ = fs::remove_dir_all(root.join(".keepframe/staging").join(&import_id));
    }
    Ok(ImportSummary {
        import_id,
        state: final_state,
        discovered: items.len(),
        imported,
        copied,
        moved,
        source_retained,
        duplicates,
        unsupported,
        failed,
    })
}

#[tauri::command]
fn cancel_import(import_id: String, state: State<'_, AppState>) -> Result<()> {
    let connection = open_db(&root_from(&state)?)?;
    if connection.execute(
        "UPDATE imports SET cancel_requested=1 WHERE id=?1 AND state='running'",
        [&import_id],
    )? != 1
    {
        return Err(KeepframeError::Message(
            "The import is no longer running.".into(),
        ));
    }
    Ok(())
}

#[tauri::command]
async fn resume_import(
    import_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportSummary> {
    let root = root_from(&state)?;
    let (mode, duplicate_source_policy, paths) = {
        let connection = open_db(&root)?;
        let (mode, duplicate_source_policy): (String, String) = connection.query_row(
            "SELECT mode,duplicate_policy FROM imports WHERE id=?1 AND state IN ('cancelled','needs_attention','running')",
            [&import_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let paths = {
            let mut statement = connection.prepare(
                "SELECT DISTINCT source_path FROM import_items WHERE import_id=?1 AND state NOT IN ('completed','duplicate') ORDER BY source_path",
            )?;
            let selected = statement
                .query_map([&import_id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            selected
        };
        connection.execute(
            "UPDATE imports SET state='cancelled',completed_at=?2 WHERE id=?1",
            params![import_id, Utc::now().to_rfc3339()],
        )?;
        (mode, duplicate_source_policy, paths)
    };
    if paths.is_empty() {
        return Err(KeepframeError::Message(
            "The import has no incomplete items to resume.".into(),
        ));
    }
    import_photos(
        paths,
        ImportOptions {
            mode,
            duplicate_source_policy,
        },
        app,
        state,
    )
    .await
}

fn asset_where(filter: &AssetFilter) -> (String, Vec<SqlValue>) {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    clauses.push(if filter.trashed.unwrap_or(false) {
        "a.trashed_at IS NOT NULL AND EXISTS (SELECT 1 FROM trash_items ti WHERE ti.asset_id=a.id AND ti.state IN ('moved','restore_failed','empty_failed'))"
    } else {
        "a.trashed_at IS NULL"
    });
    if filter.decision != "all" {
        clauses.push("a.decision = ?");
        values.push(SqlValue::Text(filter.decision.clone()));
    }
    if let Some(year) = filter.year {
        clauses.push("a.captured_at >= ? AND a.captured_at < ?");
        values.push(SqlValue::Text(format!("{year:04}-01-01")));
        values.push(SqlValue::Text(format!("{:04}-01-01", year + 1)));
    }
    if let Some(tag) = filter.tag.as_ref().filter(|tag| !tag.trim().is_empty()) {
        clauses.push("EXISTS (SELECT 1 FROM asset_tags fat JOIN tags ft ON ft.id=fat.tag_id WHERE fat.asset_id=a.id AND ft.name=? COLLATE NOCASE)");
        values.push(SqlValue::Text(tag.trim().to_string()));
    }
    if let Some(date_from) = filter.date_from.as_ref().filter(|date| !date.trim().is_empty()) {
        clauses.push("a.captured_at >= ?");
        values.push(SqlValue::Text(date_from.trim().to_string()));
    }
    if let Some(date_to) = filter.date_to.as_ref().filter(|date| !date.trim().is_empty()) {
        clauses.push("a.captured_at < datetime(?,'+1 day')");
        values.push(SqlValue::Text(date_to.trim().to_string()));
    }
    if let Some(camera) = filter.camera.as_ref().filter(|camera| !camera.trim().is_empty()) {
        clauses.push("a.camera = ? COLLATE NOCASE");
        values.push(SqlValue::Text(camera.trim().to_string()));
    }
    if let Some(tagged) = filter.tagged {
        clauses.push(if tagged { "EXISTS (SELECT 1 FROM asset_tags ata WHERE ata.asset_id=a.id)" } else { "NOT EXISTS (SELECT 1 FROM asset_tags ata WHERE ata.asset_id=a.id)" });
    }
    if let Some(located) = filter.located {
        clauses.push(if located { "a.latitude IS NOT NULL AND a.longitude IS NOT NULL" } else { "a.latitude IS NULL OR a.longitude IS NULL" });
    }
    if !filter.search.trim().is_empty() {
        clauses.push("(lower(a.filename) LIKE ? OR lower(COALESCE(a.camera,'')) LIKE ? OR EXISTS (SELECT 1 FROM asset_tags sat JOIN tags st ON st.id=sat.tag_id WHERE sat.asset_id=a.id AND lower(st.name) LIKE ?))");
        let search = SqlValue::Text(format!("%{}%", filter.search.trim().to_lowercase()));
        values.extend([search.clone(), search.clone(), search]);
    }
    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    (sql, values)
}

fn query_assets_in(
    connection: &Connection,
    filter: &AssetFilter,
    offset: i64,
    limit: i64,
) -> Result<AssetPage> {
    let offset = offset.max(0);
    let limit = limit.clamp(1, 500);
    let (where_sql, values) = asset_where(filter);
    let total = connection.query_row(
        &format!("SELECT count(*) FROM assets a{where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get::<_, i64>(0),
    )?;
    let sql = format!(
        "SELECT a.id,a.filename,a.decision,a.captured_at,a.date_fallback,a.camera,a.width,a.height,a.latitude,a.longitude,a.thumbnail_path,
         (SELECT count(*) FROM representations r WHERE r.asset_id=a.id),
         (SELECT path FROM representations r WHERE r.asset_id=a.id ORDER BY is_raw ASC,path ASC LIMIT 1),
         (SELECT path FROM versions v WHERE v.id=a.preferred_version_id),
         COALESCE((SELECT group_concat(name,char(31)) FROM (SELECT t.name AS name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=a.id ORDER BY t.name COLLATE NOCASE)),''),
         COALESCE(a.location_source,CASE WHEN a.latitude IS NULL OR a.longitude IS NULL THEN 'none' ELSE 'embedded' END)
         FROM assets a{where_sql} ORDER BY a.captured_at DESC,a.id LIMIT ? OFFSET ?"
    );
    let mut page_values = values;
    page_values.push(SqlValue::Integer(limit));
    page_values.push(SqlValue::Integer(offset));
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), |row| {
        let thumbnail: String = row.get(10)?;
        let tag_string: String = row.get(14)?;
        Ok(Asset {
            id: row.get(0)?,
            filename: row.get(1)?,
            decision: row.get(2)?,
            captured_at: row.get(3)?,
            date_fallback: row.get::<_, i64>(4)? != 0,
            camera: row.get(5)?,
            width: row.get::<_, Option<i64>>(6)?.map(|value| value as u32),
            height: row.get::<_, Option<i64>>(7)?.map(|value| value as u32),
            latitude: row.get(8)?,
            longitude: row.get(9)?,
            preview_url: thumbnail.clone(),
            thumbnail_url: thumbnail,
            representation_count: row.get(11)?,
            source_path: row.get(12)?,
            preferred_version_url: row.get(13)?,
            tags: if tag_string.is_empty() {
                Vec::new()
            } else {
                tag_string.split('\u{1f}').map(str::to_string).collect()
            },
            location_source: row.get(15)?,
        })
    })?;
    let items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(AssetPage {
        has_more: offset + (items.len() as i64) < total,
        items,
        total,
        offset,
        limit,
    })
}

fn get_asset_in(connection: &Connection, asset_id: &str) -> Result<Option<Asset>> {
    let mut statement = connection.prepare(
        "SELECT a.id,a.filename,a.decision,a.captured_at,a.date_fallback,a.camera,a.width,a.height,a.latitude,a.longitude,a.thumbnail_path,
         (SELECT count(*) FROM representations r WHERE r.asset_id=a.id),
         (SELECT path FROM representations r WHERE r.asset_id=a.id ORDER BY is_raw ASC,path ASC LIMIT 1),
         (SELECT path FROM versions v WHERE v.id=a.preferred_version_id),
         COALESCE((SELECT group_concat(name,char(31)) FROM (SELECT t.name AS name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=a.id ORDER BY t.name COLLATE NOCASE)),''),
         COALESCE(a.location_source,CASE WHEN a.latitude IS NULL OR a.longitude IS NULL THEN 'none' ELSE 'embedded' END)
         FROM assets a WHERE a.id=?1",
    )?;
    statement.query_row([asset_id], |row| {
        let thumbnail: String = row.get(10)?;
        let tag_string: String = row.get(14)?;
        Ok(Asset {
            id: row.get(0)?, filename: row.get(1)?, decision: row.get(2)?, captured_at: row.get(3)?,
            date_fallback: row.get::<_, i64>(4)? != 0, camera: row.get(5)?,
            width: row.get::<_, Option<i64>>(6)?.map(|value| value as u32), height: row.get::<_, Option<i64>>(7)?.map(|value| value as u32),
            latitude: row.get(8)?, longitude: row.get(9)?, preview_url: thumbnail.clone(), thumbnail_url: thumbnail,
            representation_count: row.get(11)?, source_path: row.get(12)?, preferred_version_url: row.get(13)?,
            tags: if tag_string.is_empty() { Vec::new() } else { tag_string.split('\u{1f}').map(str::to_string).collect() }, location_source: row.get(15)?,
        })
    }).optional().map_err(KeepframeError::from)
}

#[tauri::command]
fn query_assets(
    filter: AssetFilter,
    offset: i64,
    limit: i64,
    state: State<'_, AppState>,
) -> Result<AssetPage> {
    let connection = open_db(&root_from(&state)?)?;
    query_assets_in(&connection, &filter, offset, limit)
}

#[tauri::command]
fn get_asset(asset_id: String, state: State<'_, AppState>) -> Result<Option<Asset>> {
    let connection = open_db(&root_from(&state)?)?;
    get_asset_in(&connection, &asset_id)
}

fn append_map_clause(where_sql: &mut String, clause: &str) {
    if where_sql.is_empty() {
        where_sql.push_str(" WHERE ");
    } else {
        where_sql.push_str(" AND ");
    }
    where_sql.push_str(clause);
}

fn validate_map_bounds(bounds: &MapBounds) -> Result<()> {
    if !bounds.south.is_finite()
        || !bounds.west.is_finite()
        || !bounds.north.is_finite()
        || !bounds.east.is_finite()
        || !(-90.0..=90.0).contains(&bounds.south)
        || !(-90.0..=90.0).contains(&bounds.north)
        || !(-180.0..=180.0).contains(&bounds.west)
        || !(-180.0..=180.0).contains(&bounds.east)
        || bounds.south > bounds.north
    {
        return Err(KeepframeError::Message("Map bounds are invalid.".into()));
    }
    Ok(())
}

fn query_map_assets_in(connection: &Connection, query: &MapQuery) -> Result<Vec<MapAsset>> {
    let (mut where_sql, mut values) = asset_where(&query.filter);
    append_map_clause(
        &mut where_sql,
        "a.latitude BETWEEN -90 AND 90 AND a.longitude BETWEEN -180 AND 180",
    );
    if let Some(bounds) = query.bounds.as_ref() {
        validate_map_bounds(bounds)?;
        append_map_clause(&mut where_sql, "a.latitude >= ? AND a.latitude <= ?");
        values.push(SqlValue::Real(bounds.south));
        values.push(SqlValue::Real(bounds.north));
        if bounds.west <= bounds.east {
            append_map_clause(&mut where_sql, "a.longitude >= ? AND a.longitude <= ?");
            values.push(SqlValue::Real(bounds.west));
            values.push(SqlValue::Real(bounds.east));
        } else {
            append_map_clause(&mut where_sql, "(a.longitude >= ? OR a.longitude <= ?)");
            values.push(SqlValue::Real(bounds.west));
            values.push(SqlValue::Real(bounds.east));
        }
    }
    let sql = format!(
        "SELECT a.id,a.filename,a.latitude,a.longitude,a.captured_at,a.thumbnail_path,a.decision,\
         COALESCE(a.location_source,CASE WHEN a.latitude IS NULL OR a.longitude IS NULL THEN 'none' ELSE 'embedded' END)\
         FROM assets a{where_sql} ORDER BY a.captured_at DESC,a.id"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values.iter()), |row| {
        Ok(MapAsset {
            id: row.get(0)?,
            filename: row.get(1)?,
            latitude: row.get(2)?,
            longitude: row.get(3)?,
            captured_at: row.get(4)?,
            thumbnail_url: row.get(5)?,
            decision: row.get(6)?,
            location_source: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[tauri::command]
fn query_map_assets(query: MapQuery, state: State<'_, AppState>) -> Result<Vec<MapAsset>> {
    let connection = open_db(&root_from(&state)?)?;
    query_map_assets_in(&connection, &query)
}

#[tauri::command]
fn query_asset_ids(filter: AssetFilter, state: State<'_, AppState>) -> Result<Vec<String>> {
    let connection = open_db(&root_from(&state)?)?;
    let (where_sql, values) = asset_where(&filter);
    let mut statement = connection.prepare(&format!(
        "SELECT a.id FROM assets a{where_sql} ORDER BY a.captured_at DESC,a.id"
    ))?;
    let ids = statement
        .query_map(params_from_iter(values.iter()), |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}
#[tauri::command]
fn set_decision(asset_id: String, decision: String, state: State<'_, AppState>) -> Result<()> {
    let mut connection = open_db(&root_from(&state)?)?;
    set_decision_in(&mut connection, &asset_id, &decision)
}

fn set_decision_in(connection: &mut Connection, asset_id: &str, decision: &str) -> Result<()> {
    if !["keep", "undecided", "discard"].contains(&decision) {
        return Err(KeepframeError::Message("Invalid triage decision".into()));
    }
    let old: String = connection.query_row(
        "SELECT decision FROM assets WHERE id=?1",
        [&asset_id],
        |r| r.get(0),
    )?;
    if old == decision {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE assets SET decision=?2 WHERE id=?1",
        params![asset_id, decision],
    )?;
    tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'decision',?2,?3,?4)",params![asset_id,json!({"decision":old}).to_string(),json!({"decision":decision}).to_string(),Utc::now().to_rfc3339()])?;
    tx.commit()?;
    Ok(())
}

fn asset_tags(connection: &Connection, asset_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let tags = statement
        .query_map([asset_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(tags)
}

fn normalise_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tags = tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| seen.insert(tag.to_lowercase()))
        .collect::<Vec<_>>();
    tags.sort_by_key(|tag| tag.to_lowercase());
    tags
}

fn replace_asset_tags(
    tx: &rusqlite::Transaction<'_>,
    asset_id: &str,
    tags: &[String],
) -> Result<()> {
    tx.execute("DELETE FROM asset_tags WHERE asset_id=?1", [asset_id])?;
    for name in tags {
        tx.execute(
            "INSERT OR IGNORE INTO tags(id,name)VALUES(?1,?2)",
            params![Uuid::new_v4().to_string(), name],
        )?;
        let tag_id: String = tx.query_row(
            "SELECT id FROM tags WHERE name=?1 COLLATE NOCASE",
            [name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO asset_tags(asset_id,tag_id)VALUES(?1,?2)",
            params![asset_id, tag_id],
        )?;
    }
    tx.execute(
        "DELETE FROM tags WHERE NOT EXISTS(SELECT 1 FROM asset_tags WHERE asset_tags.tag_id=tags.id)",
        [],
    )?;
    Ok(())
}

fn catalogued_paths(connection: &Connection, asset_id: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for sql in [
        "SELECT path FROM representations WHERE asset_id=?1",
        "SELECT path FROM versions WHERE asset_id=?1",
        "SELECT output_path FROM jobs WHERE asset_id=?1 AND output_path IS NOT NULL",
    ] {
        let mut statement = connection.prepare(sql)?;
        paths.extend(
            statement
                .query_map([asset_id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .map(PathBuf::from),
        );
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    Ok(paths)
}

fn contained_library_file(root: &Path, candidate: &Path) -> Result<Option<PathBuf>> {
    if !candidate.is_absolute() || !candidate.exists() {
        return Ok(None);
    }
    let canonical_root = root.canonicalize()?;
    let canonical = candidate.canonicalize()?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return Err(KeepframeError::Message(format!(
            "Refusing a file outside the master library: {}",
            candidate.display()
        )));
    }
    Ok(Some(canonical))
}

fn move_to_trash_inner(
    asset_ids: Vec<String>,
    root: PathBuf,
    app: AppHandle,
) -> Result<TrashSummary> {
    let connection = open_db(&root)?;
    let operation_id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO trash_operations(id,state,created_at)VALUES(?1,'running',?2)",
        params![operation_id, created_at],
    )?;
    let asset_ids = asset_ids.into_iter().collect::<HashSet<_>>();
    let total = asset_ids.len();
    let mut affected = 0usize;
    let mut failed = 0usize;

    for (index, asset_id) in asset_ids.into_iter().enumerate() {
        let result = (|| -> Result<()> {
            let state: Option<(String, Option<String>)> = connection
                .query_row(
                    "SELECT decision,trashed_at FROM assets WHERE id=?1",
                    [&asset_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (decision, trashed_at) = state
                .ok_or_else(|| KeepframeError::Message("Photograph no longer exists".into()))?;
            if decision != "discard" || trashed_at.is_some() {
                return Err(KeepframeError::Message(
                    "Only discarded photographs outside Trash can be moved to Trash".into(),
                ));
            }

            let paths = catalogued_paths(&connection, &asset_id)?;
            let mut journal = Vec::new();
            for candidate in paths {
                let Some(original) = contained_library_file(&root, &candidate)? else {
                    continue;
                };
                let relative = original.strip_prefix(root.canonicalize()?).map_err(|_| {
                    KeepframeError::Message("Could not resolve the library-relative path".into())
                })?;
                if relative.starts_with(".keepframe\\Trash") {
                    return Err(KeepframeError::Message(
                        "Photograph already contains a Trash path".into(),
                    ));
                }
                let item_id = Uuid::new_v4().to_string();
                let destination = root
                    .join(".keepframe")
                    .join("Trash")
                    .join(&operation_id)
                    .join(&item_id)
                    .join(relative.file_name().ok_or_else(|| {
                        KeepframeError::Message("Trash source has no filename".into())
                    })?);
                connection.execute(
                    "INSERT INTO trash_items(id,operation_id,asset_id,original_path,trash_path,state)VALUES(?1,?2,?3,?4,?5,'planned')",
                    params![item_id, operation_id, asset_id, original.to_string_lossy(), destination.to_string_lossy()],
                )?;
                journal.push((item_id, original, destination));
            }
            if journal.is_empty() {
                return Err(KeepframeError::Message(
                    "No managed files were available to move to Trash".into(),
                ));
            }

            let mut moved = Vec::new();
            for (item_id, original, destination) in &journal {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                match fs::rename(original, destination) {
                    Ok(()) => {
                        connection.execute(
                            "UPDATE trash_items SET state='moved',error=NULL WHERE id=?1",
                            [item_id],
                        )?;
                        moved.push((item_id, original, destination));
                    }
                    Err(error) => {
                        connection.execute(
                            "UPDATE trash_items SET state='failed',error=?2 WHERE id=?1",
                            params![item_id, error.to_string()],
                        )?;
                        let mut rollback_failed = false;
                        for (moved_id, moved_original, moved_destination) in moved.iter().rev() {
                            if let Some(parent) = moved_original.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            if fs::rename(moved_destination, moved_original).is_ok() {
                                connection.execute(
                                    "UPDATE trash_items SET state='restored',error=NULL WHERE id=?1",
                                    [moved_id],
                                )?;
                            } else {
                                rollback_failed = true;
                            }
                        }
                        if rollback_failed {
                            connection.execute(
                                "UPDATE assets SET trashed_at=?2 WHERE id=?1",
                                params![asset_id, Utc::now().to_rfc3339()],
                            )?;
                        }
                        return Err(KeepframeError::Io(error));
                    }
                }
            }
            connection.execute(
                "UPDATE assets SET trashed_at=?2 WHERE id=?1",
                params![asset_id, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => affected += 1,
            Err(error) => {
                failed += 1;
                let _ = app.emit(
                    "trash-progress",
                    json!({"current":index + 1,"total":total,"assetId":asset_id,"error":error.to_string()}),
                );
                continue;
            }
        }
        let _ = app.emit(
            "trash-progress",
            json!({"current":index + 1,"total":total,"assetId":asset_id}),
        );
    }
    connection.execute(
        "UPDATE trash_operations SET state=?2,completed_at=?3 WHERE id=?1",
        params![
            operation_id,
            if failed == 0 {
                "completed"
            } else {
                "needs_attention"
            },
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(TrashSummary { affected, failed })
}

#[tauri::command]
async fn move_to_trash(
    asset_ids: Vec<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<TrashSummary> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || move_to_trash_inner(asset_ids, root, app))
        .await
        .map_err(|error| KeepframeError::Message(format!("Trash task failed: {error}")))?
}

#[tauri::command]
async fn restore_from_trash(
    asset_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashSummary> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<TrashSummary> {
        let connection = open_db(&root)?;
        let mut affected = 0usize;
        let mut failed = 0usize;
        for asset_id in asset_ids.into_iter().collect::<HashSet<_>>() {
            let mut statement = connection.prepare(
                "SELECT id,original_path,trash_path FROM trash_items WHERE asset_id=?1 AND state='moved' ORDER BY rowid DESC",
            )?;
            let items = statement
                .query_map([&asset_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(statement);
            let mut restored = Vec::new();
            let mut asset_failed = false;
            for (item_id, original, trashed) in items {
                let original = PathBuf::from(original);
                let trashed = PathBuf::from(trashed);
                if original.exists() || contained_library_file(&root, &trashed)?.is_none() {
                    connection.execute(
                        "UPDATE trash_items SET state='restore_failed',error='Original path occupied or Trash file missing' WHERE id=?1",
                        [&item_id],
                    )?;
                    asset_failed = true;
                    continue;
                }
                if let Some(parent) = original.parent() {
                    fs::create_dir_all(parent)?;
                }
                match fs::rename(&trashed, &original) {
                    Ok(()) => {
                        connection.execute(
                            "UPDATE trash_items SET state='restored',error=NULL WHERE id=?1",
                            [&item_id],
                        )?;
                        restored.push(item_id);
                    }
                    Err(error) => {
                        connection.execute(
                            "UPDATE trash_items SET state='restore_failed',error=?2 WHERE id=?1",
                            params![item_id, error.to_string()],
                        )?;
                        asset_failed = true;
                    }
                }
            }
            if !asset_failed && !restored.is_empty() {
                connection.execute("UPDATE assets SET trashed_at=NULL WHERE id=?1", [&asset_id])?;
                affected += 1;
            } else {
                failed += 1;
            }
        }
        Ok(TrashSummary { affected, failed })
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Restore task failed: {error}")))?
}

#[tauri::command]
async fn empty_trash(
    operation_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<TrashSummary> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<TrashSummary> {
        let connection = open_db(&root)?;
        let operation_ids = if operation_ids.is_empty() {
            let mut statement = connection.prepare(
                "SELECT DISTINCT operation_id FROM trash_items WHERE state='moved'",
            )?;
            let selected = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<HashSet<_>, _>>()?;
            selected
        } else {
            operation_ids.into_iter().collect::<HashSet<_>>()
        };
        let mut affected = 0usize;
        let mut failed = 0usize;
        for operation_id in operation_ids {
            let mut statement = connection.prepare(
                "SELECT id,trash_path FROM trash_items WHERE operation_id=?1 AND state='moved'",
            )?;
            let items = statement
                .query_map([&operation_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(statement);
            for (item_id, path) in items {
                let path = PathBuf::from(path);
                let Some(path) = contained_library_file(&root, &path)? else {
                    connection.execute(
                        "UPDATE trash_items SET state='missing',error='Trash file missing during Empty Trash' WHERE id=?1",
                        [&item_id],
                    )?;
                    failed += 1;
                    continue;
                };
                match trash::delete(&path) {
                    Ok(()) => {
                        connection.execute(
                            "UPDATE trash_items SET state='recycled',error=NULL WHERE id=?1",
                            [&item_id],
                        )?;
                        affected += 1;
                    }
                    Err(error) => {
                        connection.execute(
                            "UPDATE trash_items SET state='empty_failed',error=?2 WHERE id=?1",
                            params![item_id, error.to_string()],
                        )?;
                        failed += 1;
                    }
                }
            }
        }
        Ok(TrashSummary { affected, failed })
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Empty Trash task failed: {error}")))?
}

fn undo_last_action_in(connection: &mut Connection) -> Result<bool> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let action:Option<(i64,String,String,String)>=tx.query_row("SELECT id,entity_id,action,old_json FROM audit_log WHERE undone=0 ORDER BY id DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    if let Some((id, entity, kind, old)) = action {
        let value: Value = serde_json::from_str(&old)?;
        match kind.as_str() {
            "decision" => {
                let decision = value["decision"].as_str().ok_or_else(|| {
                    KeepframeError::Message("Decision undo record is invalid".into())
                })?;
                tx.execute(
                    "UPDATE assets SET decision=?2 WHERE id=?1",
                    params![entity, decision],
                )?;
            }
            "tags" => {
                let tags = serde_json::from_value::<Vec<String>>(value["tags"].clone())?;
                replace_asset_tags(&tx, &entity, &tags)?;
            }
            "location" => {
                tx.execute(
                    "UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=?4,manual_longitude=?5,location_source=?6 WHERE id=?1",
                    params![
                        entity,
                        value["latitude"].as_f64(),
                        value["longitude"].as_f64(),
                        value["manualLatitude"].as_f64(),
                        value["manualLongitude"].as_f64(),
                        value["locationSource"].as_str().unwrap_or(if value["latitude"].is_null() { "none" } else { "embedded" })
                    ],
                )?;
            }
            _ => {
                return Err(KeepframeError::Message(format!(
                    "Unsupported undo action: {kind}"
                )))
            }
        }
        tx.execute("UPDATE audit_log SET undone=1 WHERE id=?1", [id])?;
        tx.commit()?;
        return Ok(true);
    }
    tx.commit()?;
    Ok(false)
}

#[tauri::command]
fn undo_last_action(state: State<'_, AppState>) -> Result<bool> {
    let mut connection = open_db(&root_from(&state)?)?;
    undo_last_action_in(&mut connection)
}

fn update_tags_in(connection: &mut Connection, asset_id: &str, tags: Vec<String>) -> Result<()> {
    let old = asset_tags(connection, asset_id)?;
    let tags = normalise_tags(tags);
    if old == tags {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    replace_asset_tags(&tx, asset_id, &tags)?;
    tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'tags',?2,?3,?4)",params![asset_id,json!({"tags":old}).to_string(),json!({"tags":tags}).to_string(),Utc::now().to_rfc3339()])?;
    tx.commit()?;
    Ok(())
}

#[tauri::command]
fn update_tags(asset_id: String, tags: Vec<String>, state: State<'_, AppState>) -> Result<()> {
    let mut connection = open_db(&root_from(&state)?)?;
    update_tags_in(&mut connection, &asset_id, tags)
}

fn update_location_in(
    connection: &mut Connection,
    asset_id: &str,
    latitude: f64,
    longitude: f64,
) -> Result<()> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(KeepframeError::Message(
            "Coordinates are outside the valid range".into(),
        ));
    }
    let old: (Option<f64>, Option<f64>, Option<f64>, Option<f64>, String) = connection.query_row(
        "SELECT latitude,longitude,manual_latitude,manual_longitude,location_source FROM assets WHERE id=?1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    if old.0 == Some(latitude) && old.1 == Some(longitude) && old.2 == Some(latitude) && old.3 == Some(longitude) && old.4 == "manual" {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=?2,manual_longitude=?3,location_source='manual' WHERE id=?1",
        params![asset_id, latitude, longitude],
    )?;
    tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'location',?2,?3,?4)",params![asset_id,json!({"latitude":old.0,"longitude":old.1,"manualLatitude":old.2,"manualLongitude":old.3,"locationSource":old.4}).to_string(),json!({"latitude":latitude,"longitude":longitude,"manualLatitude":latitude,"manualLongitude":longitude,"locationSource":"manual"}).to_string(),Utc::now().to_rfc3339()])?;
    tx.commit()?;
    Ok(())
}

#[tauri::command]
fn update_location(
    asset_id: String,
    latitude: f64,
    longitude: f64,
    state: State<'_, AppState>,
) -> Result<()> {
    let mut connection = open_db(&root_from(&state)?)?;
    update_location_in(&mut connection, &asset_id, latitude, longitude)
}

fn update_locations_in(connection: &mut Connection, asset_ids: &[String], latitude: f64, longitude: f64) -> Result<usize> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(KeepframeError::Message("Coordinates are outside the valid range".into()));
    }
    let mut unique = HashSet::new();
    let ids = asset_ids.iter().filter(|id| unique.insert(id.as_str())).collect::<Vec<_>>();
    if ids.is_empty() { return Ok(0); }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for asset_id in &ids {
        let old: (Option<f64>, Option<f64>, Option<f64>, Option<f64>, String) = tx.query_row(
            "SELECT latitude,longitude,manual_latitude,manual_longitude,location_source FROM assets WHERE id=?1", [asset_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        tx.execute("UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=?2,manual_longitude=?3,location_source='manual' WHERE id=?1", params![asset_id, latitude, longitude])?;
        tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'location',?2,?3,?4)",params![asset_id,json!({"latitude":old.0,"longitude":old.1,"manualLatitude":old.2,"manualLongitude":old.3,"locationSource":old.4}).to_string(),json!({"latitude":latitude,"longitude":longitude,"manualLatitude":latitude,"manualLongitude":longitude,"locationSource":"manual"}).to_string(),Utc::now().to_rfc3339()])?;
    }
    tx.commit()?;
    Ok(ids.len())
}

#[tauri::command]
fn update_locations(asset_ids: Vec<String>, latitude: f64, longitude: f64, state: State<'_, AppState>) -> Result<usize> {
    let mut connection = open_db(&root_from(&state)?)?;
    update_locations_in(&mut connection, &asset_ids, latitude, longitude)
}

fn clear_manual_location_in(connection: &mut Connection, asset_id: &str) -> Result<bool> {
    let old: (Option<f64>, Option<f64>, Option<f64>, Option<f64>, String) = connection.query_row(
        "SELECT latitude,longitude,manual_latitude,manual_longitude,location_source FROM assets WHERE id=?1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    if old.2.is_none() || old.3.is_none() {
        return Ok(false);
    }
    let embedded: (Option<f64>, Option<f64>) = connection.query_row(
        "SELECT embedded_latitude,embedded_longitude FROM assets WHERE id=?1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let source = if embedded.0.is_some() && embedded.1.is_some() { "embedded" } else { "none" };
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=NULL,manual_longitude=NULL,location_source=?4 WHERE id=?1",
        params![asset_id, embedded.0, embedded.1, source],
    )?;
    tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'location',?2,?3,?4)",params![asset_id,json!({"latitude":old.0,"longitude":old.1,"manualLatitude":old.2,"manualLongitude":old.3,"locationSource":old.4}).to_string(),json!({"latitude":embedded.0,"longitude":embedded.1,"manualLatitude":Value::Null,"manualLongitude":Value::Null,"locationSource":source}).to_string(),Utc::now().to_rfc3339()])?;
    tx.commit()?;
    Ok(true)
}

#[tauri::command]
fn clear_manual_location(asset_id: String, state: State<'_, AppState>) -> Result<bool> {
    let mut connection = open_db(&root_from(&state)?)?;
    clear_manual_location_in(&mut connection, &asset_id)
}

#[tauri::command]
async fn create_edit_recipe(
    asset_id: String,
    intent: String,
    action: Option<String>,
    common_brief: Option<String>,
    state: State<'_, AppState>,
) -> Result<EditRecipe> {
    let root = root_from(&state)?;
    let worker = state
        .analysis_worker
        .lock()
        .ok()
        .and_then(|value| Some((value.url.clone()?, value.token.clone()?)));
    if let Some(action) = action.as_ref() {
        if !AI_ACTIONS.contains(&action.as_str()) {
            return Err(KeepframeError::Message("Unsupported AI action".into()));
        }
    }
    analyse_asset(&root, &asset_id, &intent, action, common_brief, worker).await
}

async fn unload_analysis_worker(state: &State<'_, AppState>) -> Result<()> {
    let worker = state
        .analysis_worker
        .lock()
        .ok()
        .and_then(|value| Some((value.url.clone()?, value.token.clone()?)));
    let Some((url, token)) = worker else {
        return Ok(());
    };
    reqwest::Client::new()
        .post(format!("{url}/unload"))
        .header("X-Keepframe-Token", token)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

const EDIT_INTENTS: &[&str] = &[
    "restoration",
    "scratch_repair",
    "denoise",
    "sharpen",
    "upscale",
    "lighting_correction",
    "object_removal",
    "sky_replacement",
    "colourisation",
    "custom",
];
const AI_ACTIONS: &[&str] = &[
    "improve_photo", "improve_lighting", "enhance_colour", "restore_old_photo",
    "remove_distraction", "custom_instruction",
];
const PRESERVE_CONSTRAINTS: &[&str] = &[
    "identity_faces",
    "composition",
    "text",
    "period_detail",
    "skin_texture",
    "grain",
    "monochrome_tonality",
];

fn validate_recipe(recipe: &EditRecipe, asset_id: &str) -> Result<()> {
    if recipe.schema_version != 1 || recipe.asset_id != asset_id {
        return Err(KeepframeError::Message(
            "The reviewed recipe does not belong to this photograph or schema version.".into(),
        ));
    }
    if recipe.observations.is_empty()
        || recipe.intents.is_empty()
        || recipe.negative_constraints.is_empty()
        || !recipe
            .intents
            .iter()
            .all(|value| EDIT_INTENTS.contains(&value.as_str()))
        || !recipe
            .preserve
            .iter()
            .all(|value| PRESERVE_CONSTRAINTS.contains(&value.as_str()))
        || recipe.action.as_ref().is_some_and(|value| !AI_ACTIONS.contains(&value.as_str()))
        || !["subtle", "balanced", "strong"].contains(&recipe.strength.as_str())
        || recipe.output.format != "png"
        || !recipe.output.preserve_dimensions
        || recipe.output.colour_space != "sRGB"
    {
        return Err(KeepframeError::Message(
            "The reviewed edit recipe is incomplete or contains unsupported values.".into(),
        ));
    }
    Ok(())
}

async fn analyse_asset(
    root: &Path,
    asset_id: &str,
    intent: &str,
    action: Option<String>,
    common_brief: Option<String>,
    worker: Option<(String, String)>,
) -> Result<EditRecipe> {
    if !EDIT_INTENTS.contains(&intent) {
        return Err(KeepframeError::Message("Unsupported edit intent".into()));
    }
    let (filename, date_fallback, camera, thumbnail): (String, i64, Option<String>, String) = {
        let connection = open_db(root)?;
        connection.query_row(
            "SELECT filename,date_fallback,camera,thumbnail_path FROM assets WHERE id=?1",
            [&asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?
    };
    let mut analysis_model = "deterministic-fallback".to_string();
    let mut observations = vec![
        "No vision analysis was run. Inspect the supplied photograph and apply only the selected intent and user brief.".into(),
        format!("Treat {} as the authoritative source photograph.", filename),
        if date_fallback != 0 {
            "Capture date came from the file timestamp; do not infer a historical period.".into()
        } else {
            "Retain the photographed moment and original composition.".into()
        },
        format!(
            "Use restrained processing appropriate for {}.",
            camera.unwrap_or_else(|| "the source photograph".into())
        ),
    ];
    let mut suggested = vec![intent.to_string()];
    let mut preserve = vec![
        "identity_faces".into(),
        "composition".into(),
        "skin_texture".into(),
    ];
    let mut constraints = vec![
        "Do not reshape faces.".into(),
        "Do not invent objects, text or jewellery.".into(),
        "Avoid plastic skin and excessive sharpening.".into(),
    ];
    if let Some((url, token)) = worker {
        let form = multipart::Form::new()
            .text("requested_intent", intent.to_string())
            .text("common_brief", common_brief.clone().unwrap_or_default())
            .part(
                "image",
                multipart::Part::bytes(fs::read(&thumbnail)?)
                    .file_name("analysis.jpg")
                    .mime_str("image/jpeg")?,
            );
        match reqwest::Client::new()
            .post(format!("{url}/v1/analyse"))
            .header("X-Keepframe-Token", token)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<AnalysisResponse>().await {
                    Ok(value) => {
                        let valid = !value.observations.is_empty()
                            && !value.suggested_intents.is_empty()
                            && !value.negative_constraints.is_empty()
                            && value
                                .suggested_intents
                                .iter()
                                .all(|item| EDIT_INTENTS.contains(&item.as_str()))
                            && value
                                .preserve
                                .iter()
                                .all(|item| PRESERVE_CONSTRAINTS.contains(&item.as_str()));
                        if valid {
                            observations = value.observations;
                            suggested = value.suggested_intents;
                            preserve = value.preserve;
                            constraints = value.negative_constraints;
                            analysis_model = "Qwen/Qwen3-VL-8B-Instruct".into();
                        } else {
                            append_runtime_log(
                                root,
                                "analysis_invalid",
                                "The worker returned a recipe outside Keepframe's schema.",
                            );
                        }
                    }
                    Err(error) => {
                        append_runtime_log(root, "analysis_invalid_json", &error.to_string())
                    }
                }
            }
            Ok(response) => append_runtime_log(
                root,
                "analysis_http_error",
                &format!("Worker returned HTTP {}", response.status()),
            ),
            Err(error) => append_runtime_log(root, "analysis_request_failed", &error.to_string()),
        }
    }
    let recipe = EditRecipe {
        schema_version: 1,
        asset_id: asset_id.to_string(),
        common_brief,
        action,
        observations,
        intents: suggested,
        preserve,
        negative_constraints: constraints,
        strength: "subtle".into(),
        output: RecipeOutput {
            format: "png".into(),
            preserve_dimensions: true,
            colour_space: "sRGB".into(),
        },
        analysis_model,
        analysis_created_at: Utc::now().to_rfc3339(),
    };
    validate_recipe(&recipe, asset_id)?;
    let connection = open_db(root)?;
    connection.execute("INSERT INTO edit_recipes(id,asset_id,schema_version,recipe_json,created_at)VALUES(?1,?2,1,?3,?4)",params![Uuid::new_v4().to_string(),asset_id,serde_json::to_string(&recipe)?,Utc::now().to_rfc3339()])?;
    Ok(recipe)
}
#[tauri::command]
fn render_prompts(recipe: EditRecipe) -> PromptSet {
    fn intent_instruction(intent: &str) -> &'static str {
        match intent {
            "restoration" => "Restore only visible age, fading, dust or damage while retaining authentic photographic detail.",
            "scratch_repair" => "Remove visible scratches, dust marks and small surface defects; reconstruct only from neighbouring evidence.",
            "denoise" => "Reduce distracting sensor or scan noise without smearing faces, edges, texture or natural grain.",
            "sharpen" => "Apply restrained, edge-aware sharpening without halos, crunchy texture or invented detail.",
            "upscale" => "Increase usable resolution while preserving identity, geometry and believable fine detail.",
            "lighting_correction" => "Correct exposure, contrast and colour balance naturally; recover available highlight and shadow detail without an HDR look.",
            "object_removal" => "Remove only the object identified in the user brief and fill the area consistently with the surrounding scene.",
            "sky_replacement" => "Replace only the sky described in the user brief, matching scene lighting, reflections, horizon and depth.",
            "colourisation" => "Colourise plausibly and conservatively while preserving tonal structure, period detail and identity.",
            "custom" => "Apply only the change explicitly described in the user brief.",
            _ => "Apply only the selected edit.",
        }
    }

    let requested_work = recipe
        .intents
        .iter()
        .map(|intent| intent_instruction(intent))
        .collect::<Vec<_>>()
        .join(" ");
    let brief = recipe
        .common_brief
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("No additional user brief was supplied; do not make changes beyond the selected intent.");
    let observations = recipe.observations.join(" ");
    let preserve = recipe.preserve.join(", ").replace('_', " ");
    let constraints = recipe.negative_constraints.join(" ");
    let core = format!(
        "Use the attached photograph as the sole visual source. Goal: {brief} Selected editing instructions: {requested_work} Relevant notes: {observations} Preserve: {preserve}. Restrictions: {constraints} Editing strength: {}. Preserve the original composition and dimensions. Output one colour-managed sRGB PNG.",
        recipe.strength
    );
    PromptSet {
        local: format!(
            "Qwen Image Edit instruction. {core} Make local, targeted changes only. Retain natural photographic texture and leave unaffected areas unchanged."
        ),
        chatgpt: format!(
            "Edit the attached photograph rather than generating a replacement scene. {core} Inspect the image itself before editing and return only the finished photograph."
        ),
        gemini: format!(
            "Perform a faithful image edit on the attached photograph. {core} Maintain subject and scene consistency; do not add unrequested generative content."
        ),
        negative: recipe.negative_constraints.join(", "),
    }
}
fn attempts_for_job(connection: &Connection, job_id: &str) -> Result<Vec<JobAttempt>> {
    let mut statement = connection.prepare(
        "SELECT attempt_number,state,started_at,finished_at,output_path,error
         FROM job_attempts WHERE job_id=?1 ORDER BY attempt_number DESC",
    )?;
    let attempts = statement
        .query_map([job_id], |row| {
            Ok(JobAttempt {
                attempt_number: row.get(0)?,
                state: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                output_url: row.get(4)?,
                error: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(attempts)
}

fn refresh_batch_state(connection: &Connection, batch_id: &str) -> Result<()> {
    let state: String = connection.query_row(
        "SELECT CASE
           WHEN EXISTS(SELECT 1 FROM jobs WHERE batch_id=?1 AND state IN ('draft','analysing')) THEN 'analysing'
           WHEN EXISTS(SELECT 1 FROM jobs WHERE batch_id=?1 AND state='review_required') THEN 'review_required'
           WHEN EXISTS(SELECT 1 FROM jobs WHERE batch_id=?1 AND state IN ('queued','running')) THEN 'running'
           WHEN EXISTS(SELECT 1 FROM jobs WHERE batch_id=?1 AND state='failed') THEN 'failed'
           WHEN EXISTS(SELECT 1 FROM jobs WHERE batch_id=?1 AND state='succeeded') THEN 'succeeded'
           ELSE 'complete' END",
        [batch_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "UPDATE batches SET state=?2 WHERE id=?1",
        params![batch_id, state],
    )?;
    Ok(())
}

#[tauri::command]
async fn enqueue_batch(
    asset_ids: Vec<String>,
    common_brief: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<BatchJob>> {
    let root = root_from(&state)?;
    let mut connection = open_db(&root)?;
    let tx = connection.transaction()?;
    let batch_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO batches(id,common_brief,state,created_at)VALUES(?1,?2,'analysing',?3)",
        params![batch_id, common_brief, now],
    )?;
    let mut jobs = Vec::new();
    let mut pending = Vec::new();
    let mut unique = HashSet::new();
    for asset_id in asset_ids {
        if !unique.insert(asset_id.clone()) {
            continue;
        }
        let name: String = tx.query_row(
            "SELECT filename FROM assets WHERE id=?1",
            [&asset_id],
            |r| r.get(0),
        )?;
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO jobs(id,batch_id,asset_id,state,prompt,created_at,updated_at)VALUES(?1,?2,?3,'analysing','',?4,?4)",params![id,batch_id,asset_id,now])?;
        pending.push((id.clone(), asset_id.clone()));
        jobs.push(BatchJob {
            id,
            batch_id: batch_id.clone(),
            asset_id,
            asset_name: name,
            state: "analysing".into(),
            prompt: String::new(),
            recipe: None,
            attempts: Vec::new(),
            error: None,
            output_url: None,
        });
    }
    if jobs.is_empty() {
        return Err(KeepframeError::Message(
            "Choose at least one photograph for the batch.".into(),
        ));
    }
    tx.commit()?;
    let worker = state
        .analysis_worker
        .lock()
        .ok()
        .and_then(|value| Some((value.url.clone()?, value.token.clone()?)));
    let batch_for_task = batch_id.clone();
    tauri::async_runtime::spawn(async move {
        for (job_id, asset_id) in pending {
            let still_analysing = open_db(&root)
                .and_then(|connection| {
                    connection
                        .query_row("SELECT state FROM jobs WHERE id=?1", [&job_id], |row| {
                            row.get::<_, String>(0)
                        })
                        .map_err(KeepframeError::from)
                })
                .is_ok_and(|value| value == "analysing");
            if !still_analysing {
                continue;
            }
            let result = analyse_asset(
                &root,
                &asset_id,
                "restoration",
                None,
                Some(common_brief.clone()),
                worker.clone(),
            )
            .await;
            if let Ok(connection) = open_db(&root) {
                match result {
                    Ok(recipe) => {
                        let prompts = render_prompts(recipe.clone());
                        let recipe_json = serde_json::to_string(&recipe).unwrap_or_default();
                        let settings = json!({"steps":50,"guidance":4.0}).to_string();
                        let _ = connection.execute(
                            "UPDATE jobs SET state='review_required',recipe_json=?2,prompt=?3,negative_prompt=?4,model='qwen-image-edit',seed=42,settings_json=?5,error=NULL,updated_at=?6 WHERE id=?1 AND state='analysing'",
                            params![job_id, recipe_json, prompts.local, prompts.negative, settings, Utc::now().to_rfc3339()],
                        );
                        let _ = app.emit(
                            "job-progress",
                            json!({"jobId":job_id,"state":"review_required"}),
                        );
                    }
                    Err(error) => {
                        let _ = connection.execute(
                            "UPDATE jobs SET state='failed',error=?2,updated_at=?3 WHERE id=?1 AND state='analysing'",
                            params![job_id, error.to_string(), Utc::now().to_rfc3339()],
                        );
                        let _ = app.emit(
                            "job-progress",
                            json!({"jobId":job_id,"state":"failed","error":error.to_string()}),
                        );
                    }
                }
            }
        }
        if let Ok(connection) = open_db(&root) {
            let _ = refresh_batch_state(&connection, &batch_for_task);
        }
    });
    Ok(jobs)
}

#[tauri::command]
fn enqueue_reviewed_recipe(
    asset_id: String,
    recipe: EditRecipe,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<BatchJob> {
    validate_recipe(&recipe, &asset_id)?;
    if prompt.trim().is_empty() {
        return Err(KeepframeError::Message(
            "The local prompt cannot be empty.".into(),
        ));
    }
    let root = root_from(&state)?;
    let mut connection = open_db(&root)?;
    let name: String = connection.query_row(
        "SELECT filename FROM assets WHERE id=?1",
        [&asset_id],
        |row| row.get(0),
    )?;
    let batch_id = Uuid::new_v4().to_string();
    let job_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let negative = recipe.negative_constraints.join(", ");
    let recipe_json = serde_json::to_string(&recipe)?;
    let tx = connection.transaction()?;
    tx.execute(
        "INSERT INTO batches(id,common_brief,state,created_at)VALUES(?1,?2,'review_required',?3)",
        params![
            batch_id,
            recipe.common_brief.clone().unwrap_or_default(),
            now
        ],
    )?;
    tx.execute(
        "INSERT INTO jobs(id,batch_id,asset_id,state,prompt,recipe_json,negative_prompt,model,seed,settings_json,created_at,updated_at)VALUES(?1,?2,?3,'review_required',?4,?5,?6,'qwen-image-edit',42,?7,?8,?8)",
        params![job_id,batch_id,asset_id,prompt,recipe_json,negative,json!({"steps":50,"guidance":4.0}).to_string(),now],
    )?;
    tx.commit()?;
    Ok(BatchJob {
        id: job_id,
        batch_id,
        asset_id,
        asset_name: name,
        state: "review_required".into(),
        prompt,
        recipe: Some(recipe),
        attempts: Vec::new(),
        error: None,
        output_url: None,
    })
}

#[tauri::command]
fn save_job_review(
    job_id: String,
    recipe: EditRecipe,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<()> {
    let connection = open_db(&root_from(&state)?)?;
    let (asset_id, current): (String, String) = connection.query_row(
        "SELECT asset_id,state FROM jobs WHERE id=?1",
        [&job_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if current != "review_required" {
        return Err(KeepframeError::Message(
            "Only a job awaiting review can be edited.".into(),
        ));
    }
    validate_recipe(&recipe, &asset_id)?;
    if prompt.trim().is_empty() {
        return Err(KeepframeError::Message(
            "The local prompt cannot be empty.".into(),
        ));
    }
    connection.execute(
        "UPDATE jobs SET recipe_json=?2,prompt=?3,negative_prompt=?4,updated_at=?5 WHERE id=?1 AND state='review_required'",
        params![job_id,serde_json::to_string(&recipe)?,prompt,recipe.negative_constraints.join(", "),Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
#[tauri::command]
fn list_jobs(state: State<'_, AppState>) -> Result<Vec<BatchJob>> {
    let connection = open_db(&root_from(&state)?)?;
    let mut statement=connection.prepare("SELECT j.id,j.batch_id,j.asset_id,a.filename,j.state,j.prompt,j.recipe_json,j.error,j.output_path FROM jobs j JOIN assets a ON a.id=j.asset_id ORDER BY j.created_at DESC,j.id")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut jobs = Vec::with_capacity(rows.len());
    for (id, batch_id, asset_id, asset_name, state, prompt, recipe_json, error, output_url) in rows
    {
        let recipe = recipe_json.and_then(|value| serde_json::from_str(&value).ok());
        jobs.push(BatchJob {
            attempts: attempts_for_job(&connection, &id)?,
            id,
            batch_id,
            asset_id,
            asset_name,
            state,
            prompt,
            recipe,
            error,
            output_url,
        });
    }
    Ok(jobs)
}

async fn execute_job(root: PathBuf, url: String, job_id: String, app: AppHandle) -> Result<()> {
    let url = validated_loopback_url(&url)?;
    let execution = {
        let connection = open_db(&root)?;
        connection.query_row(
            "SELECT j.asset_id,j.prompt,COALESCE(j.recipe_json,''),COALESCE(j.negative_prompt,''),COALESCE(j.model,'qwen-image-edit'),COALESCE(j.seed,42),COALESCE(j.settings_json,'{\"steps\":50,\"guidance\":4.0}'),r.path,r.sha256,a.captured_at,a.width,a.height
             FROM jobs j
             JOIN assets a ON a.id=j.asset_id
             JOIN representations r ON r.asset_id=a.id
             WHERE j.id=?1
             ORDER BY CASE
               WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0
               WHEN r.is_raw=1 THEN 1
               ELSE 2
             END, r.path ASC
             LIMIT 1",
            [&job_id],
            |r| {
                Ok(JobExecution {
                    asset_id: r.get(0)?,
                    prompt: r.get(1)?,
                    recipe_json: r.get(2)?,
                    negative_prompt: r.get(3)?,
                    model: r.get(4)?,
                    seed: r.get(5)?,
                    settings_json: r.get(6)?,
                    source: r.get(7)?,
                    hash: r.get(8)?,
                    captured: r.get(9)?,
                    width: r.get(10)?,
                    height: r.get(11)?,
                })
            },
        )?
    };
    let JobExecution {
        asset_id,
        prompt,
        recipe_json,
        negative_prompt,
        model,
        seed,
        settings_json,
        source,
        hash,
        captured,
        width,
        height,
    } = execution;
    if recipe_json.is_empty() {
        return Err(KeepframeError::Message(
            "This job has no reviewed image-specific recipe.".into(),
        ));
    }
    let attempt_number = {
        let mut connection = open_db(&root)?;
        let tx = connection.transaction()?;
        let current: String =
            tx.query_row("SELECT state FROM jobs WHERE id=?1", [&job_id], |row| {
                row.get(0)
            })?;
        if current != "queued" {
            return Err(KeepframeError::Message(format!(
                "Cannot start a job while it is {current}."
            )));
        }
        let number: i64 = tx.query_row(
            "SELECT attempts+1 FROM jobs WHERE id=?1",
            [&job_id],
            |row| row.get(0),
        )?;
        let now = Utc::now().to_rfc3339();
        let changed = tx.execute(
            "UPDATE jobs SET state='running',attempts=?2,updated_at=?3,error=NULL WHERE id=?1 AND state='queued'",
            params![job_id, number, now],
        )?;
        if changed != 1 {
            return Err(KeepframeError::Message(
                "The job state changed before it could start.".into(),
            ));
        }
        tx.execute(
            "INSERT INTO job_attempts(id,job_id,attempt_number,state,source_hash,recipe_json,prompt,negative_prompt,model,seed,settings_json,started_at)VALUES(?1,?2,?3,'running',?4,?5,?6,?7,?8,?9,?10,?11)",
            params![Uuid::new_v4().to_string(),job_id,number,hash,recipe_json,prompt,negative_prompt,model,seed,settings_json,now],
        )?;
        tx.commit()?;
        number
    };
    let _ = app.emit("job-progress", json!({"jobId":job_id,"state":"running"}));
    let captured = DateTime::parse_from_rfc3339(&captured)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let output_dir = root
        .join("Edits")
        .join(format!("{:04}", captured.year()))
        .join(format!("{:02}", captured.month()))
        .join(format!("{:02}", captured.day()))
        .join(&asset_id);
    fs::create_dir_all(&output_dir)?;
    // Decode every representation to a sensor/full-sized sRGB PNG. RAW files
    // themselves and their embedded previews are never submitted to the model.
    let working_dir = root.join(".keepframe/staging/working");
    let source_for_decode = PathBuf::from(&source);
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let image =
            prepare_full_resolution_image(&source_for_decode, width.zip(height), &working_dir)?;
        encode_srgb_png(&image)
    })
    .await
    .map_err(|error| {
        KeepframeError::Message(format!(
            "Full-resolution preparation stopped unexpectedly: {error}"
        ))
    })??;
    let settings: Value = serde_json::from_str(&settings_json).unwrap_or_else(|_| json!({}));
    let steps = settings.get("steps").and_then(Value::as_i64).unwrap_or(50);
    let guidance = settings
        .get("guidance")
        .and_then(Value::as_f64)
        .unwrap_or(4.0);
    let form = multipart::Form::new()
        .text("prompt", prompt.clone())
        .text("negative_prompt", negative_prompt.clone())
        .text("model", model.clone())
        .text("seed", seed.to_string())
        .text("steps", steps.to_string())
        .text("guidance", guidance.to_string())
        .text("output_dir", output_dir.to_string_lossy().to_string())
        .text("metadata_marker", "true")
        .part(
            "source_image",
            multipart::Part::bytes(bytes)
                .file_name("keepframe-source.png")
                .mime_str("image/png")?,
        );
    let response = reqwest::Client::new()
        .post(format!("{url}/api/edit-image"))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60 * 30))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(KeepframeError::Message(format!(
            "Local AI service returned {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )));
    }
    let value: Value = response.json().await?;
    let returned_output = value["images"][0]["path"]
        .as_str()
        .ok_or_else(|| KeepframeError::Message("Local AI service returned no output path".into()))?
        .to_string();
    let output = validate_local_output_path(Path::new(&returned_output), &output_dir)?;
    let output_hash = match validate_ai_output(&output, &hash, width.zip(height)) {
        Ok(output_hash) => output_hash,
        Err(error) => {
            let _ = fs::remove_file(&output);
            return Err(error);
        }
    };
    let output = output.to_string_lossy().into_owned();
    let connection = open_db(&root)?;
    let current_state: String =
        connection.query_row("SELECT state FROM jobs WHERE id=?1", [&job_id], |row| {
            row.get(0)
        })?;
    if current_state != "running" {
        connection.execute(
            "UPDATE job_attempts SET state='cancelled',finished_at=?3,error='Result ignored because the job was cancelled' WHERE job_id=?1 AND attempt_number=?2 AND state='running'",
            params![job_id,attempt_number,Utc::now().to_rfc3339()],
        )?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    let mut connection = connection;
    let tx = connection.transaction()?;
    tx.execute(
        "UPDATE jobs SET state='succeeded',output_path=?2,output_hash=?3,updated_at=?4 WHERE id=?1 AND state='running'",
        params![job_id, output, output_hash, now],
    )?;
    tx.execute(
        "UPDATE job_attempts SET state='succeeded',finished_at=?3,output_path=?4,output_hash=?5 WHERE job_id=?1 AND attempt_number=?2 AND state='running'",
        params![job_id,attempt_number,now,output,output_hash],
    )?;
    tx.execute("INSERT INTO versions(id,asset_id,kind,path,provider,prompt,recipe_json,source_hash,output_hash,state,created_at)VALUES(?1,?2,'edited',?3,'local-qwen',?4,?5,?6,?7,'candidate',?8)",params![Uuid::new_v4().to_string(),asset_id,output,prompt,recipe_json,hash,output_hash,now])?;
    let batch_id: String =
        tx.query_row("SELECT batch_id FROM jobs WHERE id=?1", [&job_id], |row| {
            row.get(0)
        })?;
    refresh_batch_state(&tx, &batch_id)?;
    tx.commit()?;
    let _ = app.emit("job-progress", json!({"jobId":job_id,"state":"succeeded"}));
    Ok(())
}

fn fail_running_attempt(root: &Path, job_id: &str, error: &str) -> Result<()> {
    let mut connection = open_db(root)?;
    let tx = connection.transaction()?;
    let now = Utc::now().to_rfc3339();
    tx.execute(
        "UPDATE job_attempts SET state='failed',finished_at=?2,error=?3 WHERE job_id=?1 AND state='running'",
        params![job_id,now,error],
    )?;
    tx.execute(
        "UPDATE jobs SET state='failed',error=?2,updated_at=?3 WHERE id=?1 AND state IN ('queued','running')",
        params![job_id,error,now],
    )?;
    if let Some(batch_id) = tx
        .query_row("SELECT batch_id FROM jobs WHERE id=?1", [job_id], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
    {
        refresh_batch_state(&tx, &batch_id)?;
    }
    tx.commit()?;
    Ok(())
}

fn transition_job_in(connection: &Connection, job_id: &str, action: &str) -> Result<String> {
    let (current, batch_id, asset_id, recipe_json, output_path): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = connection.query_row(
        "SELECT state,batch_id,asset_id,recipe_json,output_path FROM jobs WHERE id=?1",
        [job_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let next = match action {
        "approve" if current == "review_required" && recipe_json.is_some() => "queued",
        "retry" if current == "failed" && recipe_json.is_some() => "queued",
        "cancel"
            if matches!(
                current.as_str(),
                "draft" | "analysing" | "review_required" | "queued" | "running" | "waiting_external"
            ) =>
        {
            "cancelled"
        }
        "accept" if current == "succeeded" && output_path.is_some() => "accepted",
        "reject" if current == "succeeded" && output_path.is_some() => "rejected",
        "approve" if current == "review_required" => {
            return Err(KeepframeError::Message(
                "This job has no reviewed image-specific recipe.".into(),
            ));
        }
        "retry" if current == "failed" => {
            return Err(KeepframeError::Message(
                "This analysis did not produce a recipe; create a new batch for the photograph."
                    .into(),
            ));
        }
        "approve" | "retry" | "cancel" | "accept" | "reject" => {
            return Err(KeepframeError::Message(format!(
                "Cannot {action} a job while it is {current}."
            )));
        }
        _ => return Err(KeepframeError::Message("Unknown job action".into())),
    };
    let changed = connection.execute(
        "UPDATE jobs SET state=?2,error=CASE WHEN ?2='queued' THEN NULL ELSE error END,updated_at=?3 WHERE id=?1 AND state=?4",
        params![job_id,next,Utc::now().to_rfc3339(),current],
    )?;
    if changed != 1 {
        return Err(KeepframeError::Message(
            "The job changed state before the action completed.".into(),
        ));
    }
    if matches!(action, "accept" | "reject") {
        let path = output_path.expect("validated output path");
        let version_id: String = connection.query_row(
            "SELECT id FROM versions WHERE asset_id=?1 AND path=?2 ORDER BY created_at DESC LIMIT 1",
            params![asset_id, path],
            |row| row.get(0),
        )?;
        connection.execute(
            "UPDATE versions SET state=?2 WHERE id=?1",
            params![version_id, next],
        )?;
        if action == "accept" {
            connection.execute(
                "UPDATE assets SET preferred_version_id=?2 WHERE id=?1",
                params![asset_id, version_id],
            )?;
        }
    }
    refresh_batch_state(connection, &batch_id)?;
    Ok(next.into())
}

fn spawn_queued_job(
    root: PathBuf,
    url: String,
    job_id: String,
    app: AppHandle,
    gpu_gate: Arc<tokio::sync::Mutex<()>>,
) {
    tauri::async_runtime::spawn(async move {
        let _gpu_guard = gpu_gate.lock().await;
        let still_queued = open_db(&root)
            .and_then(|connection| {
                connection
                    .query_row("SELECT state FROM jobs WHERE id=?1", [&job_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(KeepframeError::from)
            })
            .is_ok_and(|value| value == "queued");
        if !still_queued {
            return;
        }
        if let Err(error) = execute_job(root.clone(), url, job_id.clone(), app.clone()).await {
            append_runtime_log(&root, "local_edit_failed", &error.to_string());
            let _ = fail_running_attempt(&root, &job_id, &error.to_string());
            let _ = app.emit(
                "job-progress",
                json!({"jobId":job_id,"state":"failed","error":error.to_string()}),
            );
        }
    });
}

#[tauri::command]
async fn update_job(
    job_id: String,
    action: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    if matches!(action.as_str(), "approve" | "retry") {
        unload_analysis_worker(&state).await?;
    }
    let root = root_from(&state)?;
    let connection = open_db(&root)?;
    let next = transition_job_in(&connection, &job_id, &action)?;
    if next == "queued" {
        let url = state.local_ai_url.lock().unwrap().clone();
        let gpu_gate = Arc::clone(&state.gpu_gate);
        spawn_queued_job(root, url, job_id, app, gpu_gate);
    }
    Ok(())
}

#[tauri::command]
async fn approve_jobs(
    job_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    unload_analysis_worker(&state).await?;
    let root = root_from(&state)?;
    let mut connection = open_db(&root)?;
    let tx = connection.transaction()?;
    let mut approved = Vec::new();
    for job_id in job_ids {
        transition_job_in(&tx, &job_id, "approve")?;
        approved.push(job_id);
    }
    tx.commit()?;
    let url = state.local_ai_url.lock().unwrap().clone();
    let gpu_gate = Arc::clone(&state.gpu_gate);
    for job_id in approved {
        spawn_queued_job(
            root.clone(),
            url.clone(),
            job_id,
            app.clone(),
            Arc::clone(&gpu_gate),
        );
    }
    Ok(())
}

#[tauri::command]
fn list_versions(asset_id: String, state: State<'_, AppState>) -> Result<Vec<AssetVersion>> {
    let connection = open_db(&root_from(&state)?)?;
    let (original_id, original_path, original_hash, created_at, preferred_id): (String, String, String, String, Option<String>) = connection.query_row(
        "SELECT r.id,a.thumbnail_path,r.sha256,a.created_at,a.preferred_version_id FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.id=?1 ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 ELSE 1 END,r.path LIMIT 1",
        [&asset_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    )?;
    let mut versions = vec![AssetVersion {
        id: format!("original:{original_id}"),
        kind: "original".into(),
        provider: Some("Keepframe protected original".into()),
        created_at,
        state: "protected".into(),
        image_url: original_path,
        prompt: None,
        is_preferred: preferred_id.is_none(),
        source_hash: Some(original_hash),
        output_hash: None,
    }];
    let mut statement = connection.prepare("SELECT id,kind,path,provider,prompt,source_hash,output_hash,state,created_at FROM versions WHERE asset_id=?1 ORDER BY created_at DESC,id DESC")?;
    let edited = statement
        .query_map([&asset_id], |row| {
            let id: String = row.get(0)?;
            Ok(AssetVersion {
                is_preferred: preferred_id.as_deref() == Some(id.as_str()),
                id,
                kind: row.get(1)?,
                image_url: row.get(2)?,
                provider: row.get(3)?,
                prompt: row.get(4)?,
                source_hash: row.get(5)?,
                output_hash: row.get(6)?,
                state: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    versions.extend(edited);
    Ok(versions)
}

#[tauri::command]
fn set_preferred_version(
    asset_id: String,
    version_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<()> {
    let mut connection = open_db(&root_from(&state)?)?;
    let tx = connection.transaction()?;
    let target = version_id.filter(|id| !id.starts_with("original:"));
    if let Some(id) = target.as_deref() {
        let belongs: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM versions WHERE id=?1 AND asset_id=?2 AND state!='rejected')", params![id,asset_id], |row| row.get(0))?;
        if !belongs {
            return Err(KeepframeError::Message(
                "That version does not belong to this photograph or has been rejected.".into(),
            ));
        }
        tx.execute("UPDATE versions SET state='accepted' WHERE id=?1", [id])?;
    }
    tx.execute(
        "UPDATE assets SET preferred_version_id=?2 WHERE id=?1",
        params![asset_id, target],
    )?;
    tx.commit()?;
    Ok(())
}

fn import_replacement_in(root: &Path, asset_id: &str, source: &Path) -> Result<AssetVersion> {
    image::open(source)
        .map_err(|_| KeepframeError::Message("The replacement is not a supported image.".into()))?;
    let mut connection = open_db(root)?;
    let (captured, source_hash): (String, String) = connection.query_row(
        "SELECT a.captured_at,r.sha256 FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.id=?1 ORDER BY r.path LIMIT 1",
        [asset_id], |row| Ok((row.get(0)?,row.get(1)?)))?;
    let date = DateTime::parse_from_rfc3339(&captured)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let output_dir = root
        .join("Edits")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}", date.day()))
        .join(asset_id);
    fs::create_dir_all(&output_dir)?;
    let extension = source
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("png")
        .to_ascii_lowercase();
    let version_id = Uuid::new_v4().to_string();
    let output = output_dir.join(format!(
        "replacement-{}-{}.{}",
        Local::now().format("%H%M%S"),
        &version_id[..8],
        extension
    ));
    fs::copy(source, &output)?;
    let output_hash = hash_file(&output)?;
    let now = Utc::now().to_rfc3339();
    let mut tags = asset_tags(&connection, asset_id)?;
    if !tags.iter().any(|tag| tag.eq_ignore_ascii_case("replaced")) {
        tags.push("replaced".into());
    }
    let tags = normalise_tags(tags);
    let tx = connection.transaction()?;
    tx.execute("INSERT INTO versions(id,asset_id,kind,path,provider,prompt,source_hash,output_hash,state,created_at)VALUES(?1,?2,'replacement',?3,'manual-replacement','User-selected replacement; catalogue metadata retained',?4,?5,'accepted',?6)", params![version_id,asset_id,output.to_string_lossy(),source_hash,output_hash,now])?;
    tx.execute(
        "UPDATE assets SET preferred_version_id=?2 WHERE id=?1",
        params![asset_id, version_id],
    )?;
    replace_asset_tags(&tx, asset_id, &tags)?;
    tx.commit()?;
    Ok(AssetVersion {
        id: version_id,
        kind: "replacement".into(),
        provider: Some("manual-replacement".into()),
        created_at: now,
        state: "accepted".into(),
        image_url: output.to_string_lossy().into(),
        prompt: Some("User-selected replacement; catalogue metadata retained".into()),
        is_preferred: true,
        source_hash: Some(source_hash),
        output_hash: Some(output_hash),
    })
}

#[tauri::command]
fn import_replacement(
    asset_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<AssetVersion> {
    import_replacement_in(&root_from(&state)?, &asset_id, Path::new(&path))
}

fn unused_export_path(destination: &Path, stem: &str, extension: &str) -> PathBuf {
    let first = destination.join(format!("{stem}-export.{extension}"));
    if !first.exists() {
        return first;
    }
    for number in 2..10_000 {
        let candidate = destination.join(format!("{stem}-export-{number}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    destination.join(format!("{stem}-export-{}.{}", Uuid::new_v4(), extension))
}

fn export_asset_image_in(root: &Path, asset_id: &str, destination: &Path) -> Result<PathBuf> {
    if !destination.is_dir() {
        return Err(KeepframeError::Message("Choose an existing export folder.".into()));
    }
    let connection = open_db(root)?;
    let (filename, width, height, preferred): (String, Option<u32>, Option<u32>, Option<String>) = connection.query_row(
        "SELECT filename,width,height,preferred_version_id FROM assets WHERE id=?1 AND trashed_at IS NULL",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let stem = Path::new(&filename)
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("photograph");

    if let Some(version_id) = preferred {
        let stored: String = connection.query_row(
            "SELECT path FROM versions WHERE id=?1 AND asset_id=?2 AND state!='rejected'",
            params![version_id, asset_id],
            |row| row.get(0),
        )?;
        let source = contained_library_file(root, Path::new(&stored))?.ok_or_else(|| {
            KeepframeError::Message("The preferred image is missing from the library.".into())
        })?;
        let extension = source.extension().and_then(OsStr::to_str).unwrap_or("png").to_ascii_lowercase();
        let output = unused_export_path(destination, stem, &extension);
        fs::copy(source, &output)?;
        return Ok(output);
    }

    let (stored, is_raw): (String, bool) = connection.query_row(
        "SELECT path,is_raw FROM representations WHERE asset_id=?1 ORDER BY is_raw DESC,path LIMIT 1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let source = contained_library_file(root, Path::new(&stored))?.ok_or_else(|| {
        KeepframeError::Message("The protected original is missing from the library.".into())
    })?;
    if is_raw {
        let image = prepare_full_resolution_image(
            &source,
            width.zip(height),
            &root.join(".keepframe/staging/working"),
        )?;
        let output = unused_export_path(destination, stem, "png");
        fs::write(&output, encode_srgb_png(&image)?)?;
        return Ok(output);
    }
    let extension = source.extension().and_then(OsStr::to_str).unwrap_or("png").to_ascii_lowercase();
    let output = unused_export_path(destination, stem, &extension);
    fs::copy(source, &output)?;
    Ok(output)
}

#[tauri::command]
async fn export_asset_image(
    asset_id: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<String> {
    let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        export_asset_image_in(&root, &asset_id, Path::new(&destination))
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Image export failed: {error}")))?
}

fn prepare_external_export(
    root: &Path,
    asset_id: &str,
    provider: &str,
    prompt: &str,
    destination: &Path,
) -> Result<PathBuf> {
    validated_provider(provider)?;
    if !destination.is_dir() {
        return Err(KeepframeError::Message(
            "Choose an existing export folder.".into(),
        ));
    }
    let connection = open_db(root)?;
    let (filename, source, width, height): (String, String, Option<u32>, Option<u32>) = connection.query_row(
        "SELECT a.filename,r.path,a.width,a.height FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.id=?1 ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.path LIMIT 1",
        [asset_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    let image = prepare_full_resolution_image(
        Path::new(&source),
        width.zip(height),
        &root.join(".keepframe/staging/working"),
    )?;
    let safe_provider = provider;
    let stem = Path::new(&filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let base = format!(
        "{}-{}-{}-{}",
        Local::now().format("%Y%m%d-%H%M%S"),
        stem,
        safe_provider,
        &Uuid::new_v4().to_string()[..8]
    );
    let output = destination.join(format!("{base}.png"));
    fs::write(&output, encode_srgb_png(&image)?)?;
    fs::write(destination.join(format!("{base}.prompt.txt")), prompt)?;
    Ok(output)
}

#[tauri::command]
fn export_external_edit(
    asset_id: String,
    provider: String,
    prompt: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<String> {
    let output = prepare_external_export(
        &root_from(&state)?,
        &asset_id,
        &provider,
        &prompt,
        Path::new(&destination),
    )?;
    Ok(output.to_string_lossy().into())
}

fn prepare_cloud_handoff_in(
    root: &Path,
    asset_id: &str,
    provider: &str,
    prompt: &str,
) -> Result<PathBuf> {
    validated_provider(provider)?;
    let connection = open_db(root)?;
    let (filename, source, width, height): (String, String, Option<u32>, Option<u32>) = connection
        .query_row(
            "SELECT a.filename,r.path,a.width,a.height
         FROM assets a
         JOIN representations r ON r.asset_id=a.id
         WHERE a.id=?1
         ORDER BY CASE
           WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0
           WHEN r.is_raw=1 THEN 1
           ELSE 2
         END, r.path ASC
         LIMIT 1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let image = prepare_full_resolution_image(
        Path::new(&source),
        width.zip(height),
        &root.join(".keepframe/staging/working"),
    )?;
    let png = encode_srgb_png(&image)?;
    let stem = Path::new(&filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let safe_provider = provider;
    let output = root.join("Exports").join(format!(
        "{}-{}-{}.png",
        Local::now().format("%Y%m%d-%H%M%S"),
        stem,
        safe_provider
    ));
    fs::write(&output, png)?;
    fs::write(output.with_extension("prompt.txt"), prompt)?;
    let now = Utc::now().to_rfc3339();
    let batch_id = Uuid::new_v4().to_string();
    let job_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO batches(id,common_brief,state,created_at)VALUES(?1,?2,'waiting_external',?3)",
        params![batch_id, prompt, now],
    )?;
    connection.execute(
        "INSERT INTO jobs(id,batch_id,asset_id,state,prompt,model,created_at,updated_at)VALUES(?1,?2,?3,'waiting_external',?4,?5,?6,?6)",
        params![job_id, batch_id, asset_id, prompt, provider, now],
    )?;
    Ok(output)
}

#[tauri::command]
fn prepare_cloud_export(
    asset_id: String,
    provider: String,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<String> {
    prepare_cloud_handoff_in(&root_from(&state)?, &asset_id, &provider, &prompt)
        .map(|output| output.to_string_lossy().into_owned())
}

fn import_returned_edit_in(
    root: &Path,
    asset_id: &str,
    path: &Path,
    provider: &str,
    prompt: &str,
    recipe: Option<EditRecipe>,
) -> Result<BatchJob> {
    validated_provider(provider)?;
    let mut connection = open_db(root)?;
    if let Some(recipe) = recipe.as_ref() {
        validate_recipe(recipe, asset_id)?;
    }
    let (captured,source_hash,asset_name,width,height):(String,String,String,Option<u32>,Option<u32>)=connection.query_row("SELECT a.captured_at,r.sha256,a.filename,a.width,a.height FROM assets a JOIN representations r ON r.asset_id=a.id WHERE a.id=?1 LIMIT 1",[asset_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let date = DateTime::parse_from_rfc3339(&captured)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let output_dir = root
        .join("Edits")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}", date.day()))
        .join(asset_id);
    fs::create_dir_all(&output_dir)?;
    let source = path.to_path_buf();
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff") {
        return Err(KeepframeError::Message(
            "The returned edit has an unsupported file extension.".into(),
        ));
    }
    let output = output_dir.join(format!(
        "cloud-{}-{}.{}",
        provider,
        Local::now().format("%H%M%S"),
        extension
    ));
    fs::copy(&source, &output)?;
    let output_hash = match validate_ai_output(&output, &source_hash, width.zip(height)) {
        Ok(output_hash) => output_hash,
        Err(error) => {
            let _ = fs::remove_file(&output);
            return Err(error);
        }
    };
    let now = Utc::now().to_rfc3339();
    let waiting: Option<(String, String)> = connection.query_row(
        "SELECT id,batch_id FROM jobs WHERE asset_id=?1 AND state='waiting_external' AND model=?2 ORDER BY updated_at DESC LIMIT 1",
        params![asset_id, provider],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let had_waiting = waiting.is_some();
    let (job_id, batch_id) = waiting.unwrap_or_else(|| (Uuid::new_v4().to_string(), Uuid::new_v4().to_string()));
    let tx = connection.transaction()?;
    if !had_waiting {
        tx.execute(
            "INSERT INTO batches(id,common_brief,state,created_at)VALUES(?1,?2,'succeeded',?3)",
            params![batch_id, prompt, now],
        )?;
    } else {
        tx.execute("UPDATE batches SET state='succeeded' WHERE id=?1", [&batch_id])?;
    }
    let recipe_json = recipe.as_ref().map(serde_json::to_string).transpose()?;
    if had_waiting {
        tx.execute("UPDATE jobs SET state='succeeded',prompt=?2,recipe_json=?3,output_path=?4,output_hash=?5,updated_at=?6,error=NULL WHERE id=?1",params![job_id,prompt,recipe_json,output.to_string_lossy(),output_hash,now])?;
    } else {
        tx.execute("INSERT INTO jobs(id,batch_id,asset_id,state,prompt,recipe_json,output_path,output_hash,created_at,updated_at)VALUES(?1,?2,?3,'succeeded',?4,?5,?6,?7,?8,?8)",params![job_id,batch_id,asset_id,prompt,recipe_json,output.to_string_lossy(),output_hash,now])?;
    }
    tx.execute("INSERT INTO versions(id,asset_id,kind,path,provider,prompt,recipe_json,source_hash,output_hash,state,created_at)VALUES(?1,?2,'edited',?3,?4,?5,?6,?7,?8,'candidate',?9)",params![Uuid::new_v4().to_string(),asset_id,output.to_string_lossy(),provider,prompt,recipe_json,source_hash,output_hash,now])?;
    tx.commit()?;
    Ok(BatchJob {
        id: job_id,
        batch_id,
        asset_id: asset_id.into(),
        asset_name,
        state: "succeeded".into(),
        prompt: prompt.into(),
        recipe,
        attempts: Vec::new(),
        error: None,
        output_url: Some(output.to_string_lossy().into()),
    })
}

#[tauri::command]
fn import_returned_edit(
    asset_id: String,
    path: String,
    provider: String,
    prompt: String,
    recipe: Option<EditRecipe>,
    state: State<'_, AppState>,
) -> Result<BatchJob> {
    import_returned_edit_in(
        &root_from(&state)?,
        &asset_id,
        Path::new(&path),
        &provider,
        &prompt,
        recipe,
    )
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            root: Mutex::new(None),
            library_lock: Mutex::new(None),
            library_issue: Mutex::new(None),
            recovery_notice: Mutex::new(None),
            local_ai_url: Mutex::new("http://127.0.0.1:7868".into()),
            health_cache: Mutex::new(None),
            analysis_worker: Mutex::new(AnalysisWorker::default()),
            gpu_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
        .setup(|app| {
            if let Some((root, url)) = load_settings() {
                if root.exists() {
                    match acquire_library_lock(&root).and_then(|lock| {
                        initialise_layout(&root)?;
                        Ok(lock)
                    }) {
                        Ok(lock) => {
                            let recovery_notice = recover_interrupted_operations(&root)?;
                            allow_media_scope(app.handle(), &root)?;
                            *app.state::<AppState>().root.lock().unwrap() = Some(root);
                            *app.state::<AppState>().library_lock.lock().unwrap() = Some(lock);
                            *app.state::<AppState>().recovery_notice.lock().unwrap() = recovery_notice;
                        }
                        Err(error) => {
                            *app.state::<AppState>().library_issue.lock().unwrap() =
                                Some(error.to_string());
                        }
                    }
                } else {
                    *app.state::<AppState>().library_issue.lock().unwrap() = Some(format!(
                        "The configured library is unavailable at {}. Reconnect the library instead of creating a replacement.",
                        root.display()
                    ));
                }
                *app.state::<AppState>().local_ai_url.lock().unwrap() = url;
            }
            start_analysis_worker(&app.state::<AppState>());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library_status,
            get_service_health,
            configure_local_ai,
            initialise_library,
            prepare_review_preview,
            check_catalogue_integrity,
            create_catalogue_backup,
            restore_catalogue_backup,
            rebuild_thumbnails,
            export_diagnostics,
            import_photos,
            cancel_import,
            resume_import,
            query_assets,
            get_asset,
            query_map_assets,
            query_asset_ids,
            set_decision,
            move_to_trash,
            restore_from_trash,
            empty_trash,
            undo_last_action,
            update_tags,
            update_location,
            update_locations,
            clear_manual_location,
            create_edit_recipe,
            render_prompts,
            enqueue_batch,
            enqueue_reviewed_recipe,
            save_job_review,
            list_jobs,
            update_job,
            approve_jobs,
            list_versions,
            set_preferred_version,
            auto_basic_adjustments,
            preview_basic_adjustments,
            apply_basic_adjustments,
            import_replacement,
            export_asset_image,
            export_external_edit,
            prepare_cloud_export,
            import_returned_edit
        ])
        .build(tauri::generate_context!())
        .expect("error while building Keepframe");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Ok(mut worker) = handle.state::<AppState>().analysis_worker.lock() {
                if let Some(child) = worker.child.as_mut() {
                    let _ = child.kill();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_formats_are_lowercase_and_unique() {
        let mut values = SUPPORTED.to_vec();
        let len = values.len();
        values.sort();
        values.dedup();
        assert_eq!(len, values.len());
        assert!(SUPPORTED.contains(&"cr3"));
        assert!(SUPPORTED.contains(&"heic"));
    }

    #[test]
    fn adjustment_ranges_are_validated() {
        let neutral = BasicAdjustments {
            exposure: 0.0,
            light_balance: 0.0,
            dynamic_range: 0.0,
            colour_boost: 0.0,
            ..BasicAdjustments::default()
        };
        assert_eq!(neutral.validate().unwrap(), neutral);
        assert!(BasicAdjustments {
            exposure: 3.1,
            ..neutral
        }
        .validate()
        .is_err());
        assert!(BasicAdjustments {
            dynamic_range: f32::NAN,
            ..neutral
        }
        .validate()
        .is_err());
    }

    #[test]
    fn geometry_adjustments_crop_and_rotate_the_same_rendered_image() {
        let mut source = RgbImage::new(3, 2);
        source.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        source.put_pixel(1, 0, image::Rgb([0, 255, 0]));
        source.put_pixel(2, 0, image::Rgb([0, 0, 255]));
        source.put_pixel(0, 1, image::Rgb([255, 255, 0]));
        source.put_pixel(1, 1, image::Rgb([0, 255, 255]));
        source.put_pixel(2, 1, image::Rgb([255, 0, 255]));
        let rotated = apply_geometry(&source, BasicAdjustments { rotate_quadrants: 1, ..BasicAdjustments::default() });
        assert_eq!(rotated.dimensions(), (2, 3));
        assert_eq!(rotated.get_pixel(1, 0), &image::Rgb([255, 0, 0]));
        let cropped = apply_geometry(&source, BasicAdjustments { crop_left: 1.0 / 3.0, crop_width: 2.0 / 3.0, crop_height: 1.0, ..BasicAdjustments::default() });
        assert_eq!(cropped.dimensions(), (2, 2));
        assert_eq!(cropped.get_pixel(0, 0), &image::Rgb([0, 255, 0]));
    }

    #[test]
    fn protected_range_expansion_reaches_black_and_white_with_minimal_tail_clipping() {
        let mut source = RgbImage::new(4_000, 1);
        for (index, pixel) in source.pixels_mut().enumerate() {
            let value = if index == 0 {
                0
            } else if index == 3_999 {
                255
            } else {
                20 + ((index - 1) * 210 / 3_997) as u8
            };
            *pixel = image::Rgb([value, value, value]);
        }
        let settings = BasicAdjustments {
            exposure: 0.0,
            light_balance: 0.0,
            dynamic_range: 100.0,
            colour_boost: 10.0,
            ..BasicAdjustments::default()
        };
        let output = apply_adjustments_to_image(&image::DynamicImage::ImageRgb8(source), settings);
        assert_eq!(output.get_pixel(1, 0)[0], 0);
        assert_eq!(output.get_pixel(3_998, 0)[0], 255);
        assert!(output.get_pixel(2_000, 0)[0] > output.get_pixel(1_000, 0)[0]);
    }

    #[test]
    fn automatic_range_uses_a_restrained_colour_boost() {
        let source =
            image::DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([90, 92, 94])));
        let settings = suggested_basic_adjustments(&source);
        assert!(settings.exposure.is_finite());
        assert_eq!(settings.dynamic_range, 100.0);
        assert!((6.0..=12.0).contains(&settings.colour_boost));
        assert!(settings.highlights < 0.0);
        assert!(settings.shadows > 0.0);
        assert!(settings.blacks < 0.0);
    }

    #[test]
    fn automatic_range_opens_dark_midtones_while_retaining_black() {
        let mut source = RgbImage::from_pixel(4_000, 1, image::Rgb([50, 50, 50]));
        for index in 3_600..3_998 {
            source.put_pixel(index, 0, image::Rgb([200, 200, 200]));
        }
        source.put_pixel(0, 0, image::Rgb([0, 0, 0]));
        source.put_pixel(1, 0, image::Rgb([0, 0, 0]));
        source.put_pixel(3_998, 0, image::Rgb([255, 255, 255]));
        source.put_pixel(3_999, 0, image::Rgb([255, 255, 255]));
        let settings = suggested_basic_adjustments(&image::DynamicImage::ImageRgb8(source.clone()));
        let output = apply_adjustments_to_image(&image::DynamicImage::ImageRgb8(source), settings);
        assert_eq!(output.get_pixel(0, 0)[0], 0);
        assert!(output.get_pixel(100, 0)[0] >= 85);
        assert_eq!(output.get_pixel(3_999, 0)[0], 255);
    }

    #[test]
    fn automatic_range_lifts_shadows_and_restrains_highlights() {
        let mut source = RgbImage::new(4_000, 1);
        for (index, pixel) in source.pixels_mut().enumerate() {
            let value = match index {
                0..=1 => 0,
                2..=999 => 35,
                1_000..=2_999 => 125,
                3_000..=3_897 => 185,
                3_898..=3_997 => 240,
                _ => 255,
            };
            *pixel = image::Rgb([value, value, value]);
        }
        let image = image::DynamicImage::ImageRgb8(source);
        let output = apply_adjustments_to_image(&image, suggested_basic_adjustments(&image));
        assert_eq!(output.get_pixel(0, 0)[0], 0);
        assert!(output.get_pixel(100, 0)[0] > 35);
        assert!(
            output.get_pixel(3_100, 0)[0] < 185,
            "highlight value was {}",
            output.get_pixel(3_100, 0)[0]
        );
        assert_eq!(output.get_pixel(3_999, 0)[0], 255);
    }

    #[test]
    fn protected_local_contrast_strengthens_detail_without_moving_flat_areas() {
        let mut source = RgbImage::from_pixel(64, 64, image::Rgb([120, 120, 120]));
        for y in 20..44 {
            for x in 24..40 {
                source.put_pixel(x, y, image::Rgb([100, 100, 100]));
            }
        }
        apply_protected_local_contrast(&mut source, 1.0, 75.0);
        assert_eq!(source.get_pixel(2, 2)[0], 120);
        assert!(source.get_pixel(23, 32)[0] - source.get_pixel(24, 32)[0] > 20);
    }

    #[test]
    fn old_adjustment_json_defaults_new_controls() {
        let settings: BasicAdjustments = serde_json::from_str(
            r#"{"exposure":0.25,"lightBalance":4,"dynamicRange":100,"colourBoost":8}"#,
        )
        .unwrap();
        assert_eq!(settings.exposure, 0.25);
        assert_eq!(settings.contrast, 0.0);
        assert_eq!(settings.curve_shadows, 0.0);
    }

    #[test]
    fn local_service_urls_are_loopback_only() {
        assert_eq!(
            validated_loopback_url("http://127.0.0.1:7868/").unwrap(),
            "http://127.0.0.1:7868"
        );
        assert!(validated_loopback_url("http://localhost:7868").is_ok());
        assert!(validated_loopback_url("http://[::1]:7868").is_ok());
        assert!(validated_loopback_url("https://127.0.0.1:7868").is_err());
        assert!(validated_loopback_url("http://192.168.1.10:7868").is_err());
        assert!(validated_loopback_url("http://127.0.0.1:7868?token=secret").is_err());
        assert!(validated_loopback_url("http://user@127.0.0.1:7868").is_err());
    }

    #[test]
    fn returned_edit_providers_are_an_enum() {
        assert_eq!(validated_provider("chatgpt").unwrap(), "chatgpt");
        assert_eq!(validated_provider("gemini").unwrap(), "gemini");
        assert!(validated_provider("../../outside").is_err());
        assert!(validated_provider("openai").is_err());
    }

    #[test]
    fn catalogue_backup_contains_committed_wal_data() {
        let root = std::env::temp_dir().join(format!("keepframe-backup-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        connection
            .execute(
                "INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at) VALUES('asset','photo.jpg','2026-09-02T12:00:00Z','thumb.jpg','2026-09-02T12:00:00Z')",
                [],
            )
            .unwrap();

        let backup = backup_database(&root, "test").unwrap();
        let restored = Connection::open(&backup).unwrap();
        let count: i64 = restored
            .query_row("SELECT COUNT(*) FROM assets WHERE id='asset'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(database_integrity(&restored).unwrap(), "ok");
        assert!(!backup.with_extension("sqlite.partial").exists());

        drop(restored);
        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fresh_catalogue_records_the_current_immutable_migration() {
        let root = std::env::temp_dir().join(format!("keepframe-migration-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let recorded: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations WHERE version=5", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(recorded, 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v5_location_migration_is_atomic_and_preserves_legacy_embedded_coordinates() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL); CREATE TABLE assets(id TEXT PRIMARY KEY,captured_at TEXT NOT NULL,latitude REAL,longitude REAL);") .unwrap();
        connection.execute("INSERT INTO assets(id,captured_at,latitude,longitude)VALUES('embedded','2026-09-12T12:00:00Z',56.2,-3.0),('none','2026-09-12T12:00:00Z',NULL,NULL)", []).unwrap();
        migrations::apply_v5(&mut connection, true, 4).unwrap();
        let embedded: (Option<f64>, Option<f64>, String) = connection.query_row("SELECT embedded_latitude,embedded_longitude,location_source FROM assets WHERE id='embedded'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        let none: String = connection.query_row("SELECT location_source FROM assets WHERE id='none'", [], |row| row.get(0)).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(embedded, (Some(56.2), Some(-3.0), "embedded".into()));
        assert_eq!(none, "none");
        assert_eq!(version, 5);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version=5", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        migrations::apply_v5(&mut connection, true, 5).unwrap();
    }

    #[test]
    fn interrupted_import_is_marked_for_attention_without_removing_files() {
        let root = std::env::temp_dir().join(format!("keepframe-import-recovery-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let staged = root.join(".keepframe/staging/import-1/photo.jpg");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged, b"staged pixels").unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute(
            "INSERT INTO imports(id,source,started_at,state) VALUES('import-1','D:/camera','2026-09-06T12:00:00Z','running')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO import_items(id,import_id,source_path,state,staging_path) VALUES('item-1','import-1','D:/camera/photo.jpg','staging',?1)",
            [staged.to_string_lossy().as_ref()],
        ).unwrap();
        drop(connection);

        let notice = recover_interrupted_operations(&root).unwrap().unwrap();
        let connection = open_db(&root).unwrap();
        let state: String = connection.query_row("SELECT state FROM imports WHERE id='import-1'", [], |row| row.get(0)).unwrap();
        assert_eq!(state, "needs_attention");
        assert!(notice.contains("Original source files were not deleted"));
        assert_eq!(fs::read(&staged).unwrap(), b"staged pixels");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_trash_is_reconciled_without_deleting_the_managed_file() {
        let root = std::env::temp_dir().join(format!("keepframe-trash-recovery-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let trashed = root.join(".keepframe/Trash/operation-1/item-1/photo.jpg");
        fs::create_dir_all(trashed.parent().unwrap()).unwrap();
        fs::write(&trashed, b"managed pixels").unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute(
            "INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at) VALUES('asset-1','photo.jpg','2026-09-06T12:00:00Z','thumb.jpg','2026-09-06T12:00:00Z')",
            [],
        ).unwrap();
        connection.execute("INSERT INTO trash_operations(id,state,created_at) VALUES('operation-1','running','2026-09-06T12:00:00Z')", []).unwrap();
        connection.execute(
            "INSERT INTO trash_items(id,operation_id,asset_id,original_path,trash_path,state) VALUES('item-1','operation-1','asset-1',?1,?2,'moved')",
            [root.join("Originals/photo.jpg").to_string_lossy().as_ref(), trashed.to_string_lossy().as_ref()],
        ).unwrap();
        drop(connection);

        let notice = recover_interrupted_operations(&root).unwrap().unwrap();
        let connection = open_db(&root).unwrap();
        let is_trashed: Option<String> = connection.query_row("SELECT trashed_at FROM assets WHERE id='asset-1'", [], |row| row.get(0)).unwrap();
        assert!(is_trashed.is_some());
        assert!(notice.contains("without deleting files"));
        assert_eq!(fs::read(&trashed).unwrap(), b"managed pixels");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_jpeg_png_and_tiff_fixtures_produce_thumbnails_and_reject_corruption() {
        let root = std::env::temp_dir().join(format!("keepframe-image-fixtures-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let image = image::DynamicImage::ImageRgb8(RgbImage::from_fn(24, 12, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 20) as u8, 90])
        }));
        for (name, format) in [("fixture.jpg", image::ImageFormat::Jpeg), ("fixture.png", image::ImageFormat::Png), ("fixture.tiff", image::ImageFormat::Tiff)] {
            let source = root.join(name);
            let thumbnail = root.join(format!("{name}.thumbnail.jpg"));
            image.save_with_format(&source, format).unwrap();
            let info = metadata(&source);
            assert_eq!((info.width, info.height), (Some(24), Some(12)));
            thumbnail_from(&source, &thumbnail).unwrap();
            assert!(thumbnail.is_file());
        }
        let malformed = root.join("malformed.jpg");
        fs::write(&malformed, b"not an image").unwrap();
        assert!(thumbnail_from(&malformed, &root.join("bad.thumbnail.jpg")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn jpeg_png_and_tiff_full_resolution_geometry_matches_preview_renderer() {
        let root = std::env::temp_dir().join(format!("keepframe-geometry-parity-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source_image = image::DynamicImage::ImageRgb8(RgbImage::from_fn(30, 20, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 11) as u8, ((x + y) * 5) as u8])
        }));
        let adjustments = BasicAdjustments { crop_left: 0.2, crop_top: 0.1, crop_width: 0.5, crop_height: 0.75, rotate_quadrants: 1, straighten: 3.0, ..BasicAdjustments::default() }.validate().unwrap();
        for (name, format) in [("fixture.jpg", image::ImageFormat::Jpeg), ("fixture.png", image::ImageFormat::Png), ("fixture.tiff", image::ImageFormat::Tiff)] {
            let source = root.join(name);
            source_image.save_with_format(&source, format).unwrap();
            let before_hash = hash_file(&source).unwrap();
            let preview = apply_adjustments_to_image(&image::open(&source).unwrap(), adjustments);
            let full_resolution = prepare_full_resolution_image(&source, Some((30, 20)), &root.join("working")).unwrap();
            let candidate = apply_adjustments_to_image(&full_resolution, adjustments);
            assert_eq!(candidate.dimensions(), (10, 23), "{name}");
            assert_eq!(candidate, preview, "{name}");
            let output = root.join(format!("{name}.candidate.png"));
            fs::write(&output, encode_srgb_png(&image::DynamicImage::ImageRgb8(candidate)).unwrap()).unwrap();
            let rendered = image::open(&output).unwrap();
            assert_eq!((rendered.width(), rendered.height()), (10, 23), "{name}");
            assert_eq!(hash_file(&source).unwrap(), before_hash, "{name} source was modified");
        }
        let malformed = root.join("malformed.tiff");
        fs::write(&malformed, b"not an image").unwrap();
        assert!(prepare_full_resolution_image(&malformed, None, &root.join("working")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_library_rejects_a_second_writer() {
        let root = std::env::temp_dir().join(format!("keepframe-lock-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let first = acquire_library_lock(&root).unwrap();
        assert!(acquire_library_lock(&root).is_err());
        drop(first);
        assert!(acquire_library_lock(&root).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trash_view_hides_fully_emptied_tombstones() {
        let trashed = AssetFilter {
            decision: "all".into(),
            search: String::new(),
            year: None,
            tag: None,
            date_from: None,
            date_to: None,
            camera: None,
            tagged: None,
            located: None,
            trashed: Some(true),
        };
        let normal = AssetFilter {
            decision: "all".into(),
            search: String::new(),
            year: None,
            tag: None,
            date_from: None,
            date_to: None,
            camera: None,
            tagged: None,
            located: None,
            trashed: Some(false),
        };
        let (trash_where, _) = asset_where(&trashed);
        let (normal_where, _) = asset_where(&normal);
        assert!(trash_where.contains("trash_items"));
        assert!(trash_where.contains("empty_failed"));
        assert!(!trash_where.contains("recycled"));
        assert!(normal_where.contains("a.trashed_at IS NULL"));
    }

    fn map_filter() -> AssetFilter {
        AssetFilter { decision: "all".into(), search: String::new(), year: None, tag: None, date_from: None, date_to: None, camera: None, tagged: None, located: None, trashed: Some(false) }
    }

    #[test]
    fn map_query_reads_the_full_filtered_catalogue_and_excludes_bad_coordinates() {
        let root = std::env::temp_dir().join(format!("keepframe-map-query-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        for (id, decision, latitude, longitude) in [("keep", "keep", 56.12, -3.1), ("discard", "discard", 56.15, -3.2), ("bad", "keep", 95.0, -3.3), ("none", "keep", 0.0, 0.0)] {
            connection.execute("INSERT INTO assets(id,filename,decision,captured_at,latitude,longitude,location_source,thumbnail_path,created_at)VALUES(?1,?1,?2,'2026-09-12T12:00:00Z',?3,?4,'embedded','thumb.jpg','2026-09-12T12:00:00Z')", params![id, decision, latitude, longitude]).unwrap();
        }
        connection.execute("UPDATE assets SET latitude=NULL,longitude=NULL,location_source='none' WHERE id='none'", []).unwrap();
        let all = query_map_assets_in(&connection, &MapQuery { filter: map_filter(), bounds: None }).unwrap();
        assert_eq!(all.len(), 2);
        let mut keep = map_filter(); keep.decision = "keep".into();
        let bounded = query_map_assets_in(&connection, &MapQuery { filter: keep, bounds: Some(MapBounds { south: 56.0, west: -3.15, north: 56.13, east: -3.0 }) }).unwrap();
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].id, "keep");
        assert!(query_map_assets_in(&connection, &MapQuery { filter: map_filter(), bounds: Some(MapBounds { south: 20.0, west: 0.0, north: -20.0, east: 1.0 }) }).is_err());
        drop(connection); fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_location_preserves_embedded_gps_and_can_be_cleared() {
        let root = std::env::temp_dir().join(format!("keepframe-manual-location-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let mut connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,latitude,longitude,embedded_latitude,embedded_longitude,location_source,thumbnail_path,created_at)VALUES('asset','photo.jpg','2026-09-12T12:00:00Z',56.1,-3.1,56.1,-3.1,'embedded','thumb.jpg','2026-09-12T12:00:00Z')", []).unwrap();
        update_location_in(&mut connection, "asset", 41.3851, 2.1734).unwrap();
        let manual: (f64, f64, f64, f64, String) = connection.query_row("SELECT latitude,longitude,embedded_latitude,embedded_longitude,location_source FROM assets WHERE id='asset'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(manual, (41.3851, 2.1734, 56.1, -3.1, "manual".into()));
        assert!(clear_manual_location_in(&mut connection, "asset").unwrap());
        let restored: (Option<f64>, Option<f64>, Option<f64>, String) = connection.query_row("SELECT latitude,longitude,manual_latitude,location_source FROM assets WHERE id='asset'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(restored, (Some(56.1), Some(-3.1), None, "embedded".into()));
        drop(connection); fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn map_query_scales_to_ten_thousand_lightweight_markers() {
        let root = std::env::temp_dir().join(format!("keepframe-map-scale-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let mut connection = open_db(&root).unwrap();
        for range in [0..2_000, 2_000..10_000] {
            let expected: usize = if range.start == 0 { 2_000 } else { 10_000 };
            let tx = connection.transaction().unwrap();
            for number in range {
                tx.execute("INSERT INTO assets(id,filename,captured_at,latitude,longitude,location_source,thumbnail_path,created_at)VALUES(?1,?2,'2026-09-12T12:00:00Z',?3,?4,'embedded','thumb.jpg','2026-09-12T12:00:00Z')", params![format!("asset-{number}"), format!("photo-{number}.jpg"), 55.0 + (number % 500) as f64 / 1000.0, -4.0 + (number % 500) as f64 / 1000.0]).unwrap();
            }
            tx.commit().unwrap();
            let started = Instant::now();
            let markers = query_map_assets_in(&connection, &MapQuery { filter: map_filter(), bounds: None }).unwrap();
            assert_eq!(markers.len(), expected);
            assert!(started.elapsed() < Duration::from_secs(5), "lightweight marker query should remain responsive");
        }
        drop(connection); fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn prompt_keeps_safety_constraints() {
        let recipe = EditRecipe {
            schema_version: 1,
            asset_id: "a".into(),
            common_brief: Some("Restore".into()),
            action: Some("restore_old_photo".into()),
            observations: vec!["Scratch".into()],
            intents: vec!["restoration".into()],
            preserve: vec!["identity_faces".into()],
            negative_constraints: vec!["No face reshaping".into()],
            strength: "subtle".into(),
            output: RecipeOutput {
                format: "png".into(),
                preserve_dimensions: true,
                colour_space: "sRGB".into(),
            },
            analysis_model: "test".into(),
            analysis_created_at: Utc::now().to_rfc3339(),
        };
        let prompts = render_prompts(recipe);
        assert!(prompts.local.contains("No face reshaping"));
        assert!(prompts
            .chatgpt
            .contains("rather than generating a replacement scene"));
        assert!(prompts.local.contains("Qwen Image Edit instruction"));
        assert!(prompts
            .gemini
            .contains("Maintain subject and scene consistency"));
    }

    fn test_recipe(asset_id: &str) -> EditRecipe {
        EditRecipe {
            schema_version: 1,
            asset_id: asset_id.into(),
            common_brief: Some("Restore naturally".into()),
            action: Some("restore_old_photo".into()),
            observations: vec!["Visible dust requires restrained repair.".into()],
            intents: vec!["restoration".into()],
            preserve: vec!["composition".into()],
            negative_constraints: vec!["Do not invent detail.".into()],
            strength: "subtle".into(),
            output: RecipeOutput {
                format: "png".into(),
                preserve_dimensions: true,
                colour_space: "sRGB".into(),
            },
            analysis_model: "deterministic-fallback".into(),
            analysis_created_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn job_transitions_require_review_and_record_attempts() {
        let root = std::env::temp_dir().join(format!("keepframe-job-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at)VALUES('asset','photo.jpg','2026-09-02T12:00:00Z','thumb.jpg','2026-09-02T12:00:00Z')", []).unwrap();
        connection.execute("INSERT INTO batches(id,common_brief,state,created_at)VALUES('batch','Restore','review_required','2026-09-02T12:00:00Z')", []).unwrap();
        let recipe_json = serde_json::to_string(&test_recipe("asset")).unwrap();
        connection.execute("INSERT INTO jobs(id,batch_id,asset_id,state,prompt,recipe_json,created_at,updated_at)VALUES('job','batch','asset','review_required','Individual prompt',?1,'2026-09-02T12:00:00Z','2026-09-02T12:00:00Z')", [&recipe_json]).unwrap();

        assert!(transition_job_in(&connection, "job", "accept").is_err());
        assert_eq!(
            transition_job_in(&connection, "job", "approve").unwrap(),
            "queued"
        );
        assert!(transition_job_in(&connection, "job", "approve").is_err());
        assert_eq!(
            transition_job_in(&connection, "job", "cancel").unwrap(),
            "cancelled"
        );
        assert!(transition_job_in(&connection, "job", "retry").is_err());

        connection.execute("INSERT INTO job_attempts(id,job_id,attempt_number,state,source_hash,recipe_json,prompt,negative_prompt,model,seed,settings_json,started_at,finished_at,error)VALUES('attempt','job',1,'failed','hash',?1,'Individual prompt','No invention','qwen-image-edit',42,'{}','2026-09-02T12:01:00Z','2026-09-02T12:02:00Z','Service unavailable')", [&recipe_json]).unwrap();
        let attempts = attempts_for_job(&connection, "job").unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_number, 1);
        assert_eq!(attempts[0].state, "failed");

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn restart_requeues_running_job_and_closes_attempt() {
        let root = std::env::temp_dir().join(format!("keepframe-recovery-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at)VALUES('asset','photo.jpg','2026-09-02T12:00:00Z','thumb.jpg','2026-09-02T12:00:00Z')", []).unwrap();
        connection.execute("INSERT INTO batches(id,common_brief,state,created_at)VALUES('batch','Restore','running','2026-09-02T12:00:00Z')", []).unwrap();
        let recipe_json = serde_json::to_string(&test_recipe("asset")).unwrap();
        connection.execute("INSERT INTO jobs(id,batch_id,asset_id,state,prompt,recipe_json,attempts,created_at,updated_at)VALUES('job','batch','asset','running','Individual prompt',?1,1,'2026-09-02T12:00:00Z','2026-09-02T12:00:00Z')", [&recipe_json]).unwrap();
        connection.execute("INSERT INTO job_attempts(id,job_id,attempt_number,state,source_hash,recipe_json,prompt,negative_prompt,model,seed,settings_json,started_at)VALUES('attempt','job',1,'running','hash',?1,'Individual prompt','No invention','qwen-image-edit',42,'{}','2026-09-02T12:01:00Z')", [&recipe_json]).unwrap();
        drop(connection);

        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT state FROM jobs WHERE id='job'", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "queued"
        );
        let recovered: (String, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT state,finished_at,error FROM job_attempts WHERE id='attempt'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(recovered.0, "failed");
        assert!(recovered.1.is_some());
        assert!(recovered.2.unwrap().contains("closed"));

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn catalogue_layout_and_thumbnail_cache_are_rebuildable() {
        let root = std::env::temp_dir().join(format!("keepframe-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        assert!(db_path(&root).exists());
        let source = root.join("test.png");
        image::RgbaImage::from_pixel(80, 60, image::Rgba([30, 80, 55, 255]))
            .save(&source)
            .unwrap();
        let before = hash_file(&source).unwrap();
        let thumb = root.join(".keepframe/thumbnails/test.jpg");
        thumbnail_from(&source, &thumb).unwrap();
        assert!(thumb.exists());
        assert_eq!(before, hash_file(&source).unwrap());
        fs::remove_file(&thumb).unwrap();
        thumbnail_from(&source, &thumb).unwrap();
        assert!(thumb.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn working_png_preserves_dimensions_and_declares_srgb() {
        let root = std::env::temp_dir().join(format!("keepframe-working-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.png");
        image::RgbImage::from_pixel(640, 480, image::Rgb([30, 80, 55]))
            .save(&source)
            .unwrap();

        let prepared = prepare_full_resolution_image(&source, Some((640, 480)), &root).unwrap();
        assert_eq!((prepared.width(), prepared.height()), (640, 480));
        let encoded = encode_srgb_png(&prepared).unwrap();
        assert!(encoded.windows(4).any(|chunk| chunk == b"sRGB"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undersized_working_image_is_rejected_instead_of_exported() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::new(800, 600));
        let error =
            verify_full_resolution(&image, Some((4000, 3000)), Path::new("camera-source.cr3"))
                .unwrap_err()
                .to_string();
        assert!(error.contains("No preview was substituted"));
        assert!(error.contains("800x600"));
    }

    #[test]
    fn invalid_raw_fails_without_using_an_embedded_preview() {
        assert!(libraw_decoder_path().is_file());
        let root =
            std::env::temp_dir().join(format!("keepframe-raw-failure-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("invalid.dng");
        fs::write(&source, b"not a raw photograph").unwrap();

        let error = prepare_full_resolution_image(&source, None, &root)
            .unwrap_err()
            .to_string();
        assert!(error.contains("LibRaw could not decode"));
        assert!(!error.contains("preview was found"));
        assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("raw-")));

        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn completed_import_removes_only_external_source() {
        let base = std::env::temp_dir().join(format!("keepframe-move-test-{}", Uuid::new_v4()));
        let library = base.join("library");
        let external = base.join("camera.jpg");
        let managed = library.join("Originals/2026/09/02/managed.jpg");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&external, b"verified pixels").unwrap();
        fs::write(&managed, b"verified pixels").unwrap();
        let digest = hash_file(&external).unwrap();
        safety::delete_verified_external_source(&external, &managed, &digest, &library).unwrap();
        assert!(!external.exists());
        assert!(managed.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn duplicate_requires_a_present_matching_managed_file() {
        let root =
            std::env::temp_dir().join(format!("keepframe-duplicate-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let managed = root.join("Originals/2026/09/02/managed.jpg");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&managed, b"verified photograph").unwrap();
        let hash = hash_file(&managed).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at)VALUES('asset','managed.jpg','2026-09-02T12:00:00Z','thumb.jpg','2026-09-02T12:00:00Z')", []).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'jpg','managed',19,0)", params![managed.to_string_lossy(), hash]).unwrap();

        assert!(verified_representation_path(&connection, &hash).unwrap().is_some());
        fs::write(&managed, b"damaged").unwrap();
        assert!(verified_representation_path(&connection, &hash).unwrap().is_none());
        fs::remove_file(&managed).unwrap();
        assert!(verified_representation_path(&connection, &hash).unwrap().is_none());
        assert!(path_is_catalogued(&connection, &managed).unwrap());

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trash_targets_must_be_files_inside_the_master_library() {
        let base =
            std::env::temp_dir().join(format!("keepframe-delete-target-test-{}", Uuid::new_v4()));
        let library = base.join("library");
        let managed = library.join("Originals/2026/09/02/managed.jpg");
        let outside = base.join("outside.jpg");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&managed, b"managed photograph").unwrap();
        fs::write(&outside, b"outside photograph").unwrap();
        assert_eq!(
            contained_library_file(&library, &managed).unwrap(),
            Some(managed.canonicalize().unwrap())
        );
        assert!(contained_library_file(&library, &outside).is_err());
        assert_eq!(
            contained_library_file(&library, &library.join("missing.jpg")).unwrap(),
            None
        );

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn decisions_tags_and_locations_are_undoable_in_order() {
        let root = std::env::temp_dir().join(format!("keepframe-undo-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let mut connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,thumbnail_path,created_at)VALUES('asset','photo.jpg','2026-09-02T12:00:00Z','thumb.jpg','2026-09-02T12:00:00Z')", []).unwrap();

        update_tags_in(
            &mut connection,
            "asset",
            vec!["People/Hazel".into(), "Beach".into()],
        )
        .unwrap();
        update_location_in(&mut connection, "asset", 56.21, -2.93).unwrap();
        set_decision_in(&mut connection, "asset", "keep").unwrap();
        assert_eq!(asset_tags(&connection, "asset").unwrap().len(), 2);
        assert_eq!(
            connection
                .query_row("SELECT decision FROM assets WHERE id='asset'", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "keep"
        );

        assert!(undo_last_action_in(&mut connection).unwrap());
        assert_eq!(
            connection
                .query_row("SELECT decision FROM assets WHERE id='asset'", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "undecided"
        );
        assert!(undo_last_action_in(&mut connection).unwrap());
        let location: (Option<f64>, Option<f64>) = connection
            .query_row(
                "SELECT latitude,longitude FROM assets WHERE id='asset'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(location, (None, None));
        assert!(undo_last_action_in(&mut connection).unwrap());
        assert!(asset_tags(&connection, "asset").unwrap().is_empty());
        assert!(!undo_last_action_in(&mut connection).unwrap());

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn catalogue_queries_filter_and_page_in_sql() {
        let root = std::env::temp_dir().join(format!("keepframe-query-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        for (id, filename, decision, captured, camera) in [
            ("a", "alpha.jpg", "keep", "2026-08-03T10:00:00Z", "Nikon Z8"),
            (
                "b",
                "bravo.jpg",
                "discard",
                "2025-07-02T10:00:00Z",
                "Canon R5",
            ),
            (
                "c",
                "charlie.jpg",
                "keep",
                "2024-06-01T10:00:00Z",
                "Sony A7",
            ),
        ] {
            connection.execute(
                "INSERT INTO assets(id,filename,decision,captured_at,camera,thumbnail_path,created_at)VALUES(?1,?2,?3,?4,?5,?6,?4)",
                params![id, filename, decision, captured, camera, format!("{id}.thumb.jpg")],
            ).unwrap();
            connection.execute(
                "INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES(?1,?2,?3,?4,'jpg',?2,1,0)",
                params![format!("r-{id}"), id, format!("{id}.jpg"), format!("hash-{id}")],
            ).unwrap();
        }
        connection
            .execute("INSERT INTO tags(id,name)VALUES('coast','Coast')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO asset_tags(asset_id,tag_id)VALUES('a','coast')",
                [],
            )
            .unwrap();

        let keep = AssetFilter {
            decision: "keep".into(),
            search: String::new(),
            year: None,
            tag: None,
            date_from: None,
            date_to: None,
            camera: None,
            tagged: None,
            located: None,
            trashed: None,
        };
        let first = query_assets_in(&connection, &keep, 0, 1).unwrap();
        assert_eq!(first.total, 2);
        assert_eq!(first.items[0].id, "a");
        assert!(first.has_more);
        let second = query_assets_in(&connection, &keep, 1, 1).unwrap();
        assert_eq!(second.items[0].id, "c");
        assert!(!second.has_more);

        let tag_search = AssetFilter {
            decision: "all".into(),
            search: "coast".into(),
            year: Some(2026),
            tag: Some("COAST".into()),
            date_from: None,
            date_to: None,
            camera: None,
            tagged: None,
            located: None,
            trashed: None,
        };
        let result = query_assets_in(&connection, &tag_search, 0, 50).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].tags, vec!["Coast"]);

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn local_service_health_requires_the_edit_capability_and_reports_busy_state() {
        let health = parse_local_ai_health(&json!({
            "image_runtime": { "busy": true, "loading": false, "generating": false },
            "edit_models": [{ "key": "qwen-image-edit", "label": "Qwen-Image-Edit", "available": true }]
        }));
        assert!(health.service_reachable);
        assert!(health.local_ai_available);
        assert!(health.local_ai_busy);
        assert_eq!(health.local_ai_model.as_deref(), Some("Qwen-Image-Edit"));
        assert_eq!(health.local_ai_state, "available");

        let unavailable = parse_local_ai_health(&json!({ "edit_models": [] }));
        assert!(!unavailable.local_ai_available);
        assert_eq!(unavailable.local_ai_state, "model_missing");
    }

    #[test]
    fn unavailable_loopback_service_fails_closed_without_claiming_readiness() {
        let health = tauri::async_runtime::block_on(local_ai_health("http://127.0.0.1:9"));
        assert!(!health.service_reachable);
        assert!(!health.local_ai_available);
        assert!(!health.local_ai_busy);
        assert_eq!(health.local_ai_state, "service_not_running");
        assert!(health.local_ai_detail.contains("No local image service"));
    }

    #[test]
    fn ai_results_must_be_distinct_decodable_and_plausibly_sized() {
        let root = std::env::temp_dir().join(format!("keepframe-ai-output-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.png");
        let same = root.join("same.png");
        let changed = root.join("changed.png");
        let tiny = root.join("tiny.png");
        image::DynamicImage::new_rgb8(128, 96).save(&source).unwrap();
        fs::copy(&source, &same).unwrap();
        let mut altered = image::RgbImage::new(128, 96);
        altered.put_pixel(0, 0, image::Rgb([12, 34, 56]));
        altered.save(&changed).unwrap();
        image::DynamicImage::new_rgb8(4, 4).save(&tiny).unwrap();
        let source_hash = hash_file(&source).unwrap();
        assert!(validate_ai_output(&same, &source_hash, Some((128, 96))).is_err());
        assert!(validate_ai_output(&tiny, &source_hash, Some((128, 96))).is_err());
        assert!(validate_ai_output(&changed, &source_hash, Some((128, 96))).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn manual_handoff_creates_a_waiting_job_and_imports_a_safe_traceable_version() {
        let root = std::env::temp_dir().join(format!("keepframe-handoff-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/2026/09/08/original.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        let mut original_pixels = image::RgbImage::new(128, 96);
        original_pixels.put_pixel(0, 0, image::Rgb([20, 40, 60]));
        original_pixels.save(&original).unwrap();
        let original_hash = hash_file(&original).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,width,height,thumbnail_path,created_at)VALUES('asset','original.png','2026-09-08T12:00:00Z',128,96,?1,'2026-09-08T12:00:00Z')", [original.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','original',1,0)", params![original.to_string_lossy(),original_hash]).unwrap();
        drop(connection);

        let prepared = prepare_cloud_handoff_in(&root, "asset", "chatgpt", "Improve the lighting naturally.").unwrap();
        assert_eq!(image::open(&prepared).unwrap().dimensions(), (128, 96));
        assert_eq!(fs::read_to_string(prepared.with_extension("prompt.txt")).unwrap(), "Improve the lighting naturally.");
        let connection = open_db(&root).unwrap();
        let waiting: String = connection.query_row("SELECT state FROM jobs WHERE asset_id='asset'", [], |row| row.get(0)).unwrap();
        assert_eq!(waiting, "waiting_external");
        drop(connection);

        let malformed = root.join("returned-malformed.png");
        fs::write(&malformed, b"not an image").unwrap();
        assert!(import_returned_edit_in(&root, "asset", &malformed, "chatgpt", "Improve the lighting naturally.", Some(test_recipe("asset"))).is_err());
        assert!(import_returned_edit_in(&root, "asset", &original, "chatgpt", "Improve the lighting naturally.", Some(test_recipe("asset"))).is_err());
        let tiny = root.join("returned-tiny.png");
        image::DynamicImage::new_rgb8(4, 4).save(&tiny).unwrap();
        assert!(import_returned_edit_in(&root, "asset", &tiny, "chatgpt", "Improve the lighting naturally.", Some(test_recipe("asset"))).is_err());

        let returned = root.join("returned-edited.png");
        let mut returned_pixels = image::RgbImage::new(128, 96);
        returned_pixels.put_pixel(127, 95, image::Rgb([200, 180, 160]));
        returned_pixels.save(&returned).unwrap();
        let job = import_returned_edit_in(&root, "asset", &returned, "chatgpt", "Improve the lighting naturally.", Some(test_recipe("asset"))).unwrap();
        assert_eq!(job.state, "succeeded");
        assert_eq!(job.recipe.unwrap().asset_id, "asset");
        let connection = open_db(&root).unwrap();
        let (job_state, batch_state): (String, String) = connection.query_row("SELECT j.state,b.state FROM jobs j JOIN batches b ON b.id=j.batch_id WHERE j.id=?1", [&job.id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!((job_state, batch_state), ("succeeded".into(), "succeeded".into()));
        let (provider, prompt, recipe_json, source_hash, version_state): (String, String, String, String, String) = connection.query_row("SELECT provider,prompt,recipe_json,source_hash,state FROM versions WHERE asset_id='asset'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(provider, "chatgpt");
        assert_eq!(prompt, "Improve the lighting naturally.");
        assert!(recipe_json.contains("restore_old_photo"));
        assert_eq!(source_hash, original_hash);
        assert_eq!(version_state, "candidate");
        assert_eq!(hash_file(&original).unwrap(), original_hash);
        assert_eq!(transition_job_in(&connection, &job.id, "accept").unwrap(), "accepted");
        let preferred: String = connection.query_row("SELECT preferred_version_id FROM assets WHERE id='asset'", [], |row| row.get(0)).unwrap();
        assert!(!preferred.is_empty());
        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn local_outputs_are_accepted_only_inside_the_intended_edits_folder() {
        let root = std::env::temp_dir().join(format!("keepframe-output-test-{}", Uuid::new_v4()));
        let intended = root.join("Edits/asset");
        let outside_dir = root.join("outside");
        fs::create_dir_all(&intended).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        let inside = intended.join("result.png");
        let outside = outside_dir.join("result.png");
        image::DynamicImage::new_rgb8(2, 2).save(&inside).unwrap();
        image::DynamicImage::new_rgb8(2, 2).save(&outside).unwrap();

        assert_eq!(
            validate_local_output_path(&inside, &intended).unwrap(),
            inside.canonicalize().unwrap()
        );
        assert!(validate_local_output_path(&outside, &intended).is_err());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replacement_becomes_preferred_without_changing_catalogue_metadata_or_original() {
        let root =
            std::env::temp_dir().join(format!("keepframe-replacement-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/2026/09/02/original.png");
        let replacement = root.join("incoming.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(3, 2).save(&original).unwrap();
        image::DynamicImage::new_rgb8(5, 4)
            .save(&replacement)
            .unwrap();
        let original_hash = hash_file(&original).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,decision,captured_at,camera,latitude,longitude,thumbnail_path,created_at)VALUES('asset','original.png','keep','2026-09-02T12:00:00Z','Test Camera',56.2,-2.9,'thumb.png','2026-09-02T12:00:00Z')", []).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','original',1,0)", params![original.to_string_lossy(),original_hash]).unwrap();
        connection
            .execute("INSERT INTO tags(id,name)VALUES('tag','Family')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO asset_tags(asset_id,tag_id)VALUES('asset','tag')",
                [],
            )
            .unwrap();
        drop(connection);

        let version = import_replacement_in(&root, "asset", &replacement).unwrap();
        let connection = open_db(&root).unwrap();
        let metadata: (String,String,Option<String>,Option<f64>,Option<f64>,Option<String>) = connection.query_row("SELECT decision,captured_at,camera,latitude,longitude,preferred_version_id FROM assets WHERE id='asset'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap();
        assert_eq!(&metadata.0, "keep");
        assert_eq!(&metadata.1, "2026-09-02T12:00:00Z");
        assert_eq!(metadata.2.as_deref(), Some("Test Camera"));
        assert_eq!((metadata.3, metadata.4), (Some(56.2), Some(-2.9)));
        assert_eq!(metadata.5.as_deref(), Some(version.id.as_str()));
        assert_eq!(asset_tags(&connection, "asset").unwrap(), vec!["Family", "replaced"]);
        assert_eq!(hash_file(&original).unwrap(), original_hash);
        assert!(Path::new(&version.image_url).starts_with(root.join("Edits")));

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn export_copies_a_non_raw_original_without_overwriting_an_existing_export() {
        let root = std::env::temp_dir().join(format!("keepframe-export-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/2026/09/02/original.png");
        let destination = root.join("Exports-test");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::create_dir_all(&destination).unwrap();
        image::DynamicImage::new_rgb8(3, 2).save(&original).unwrap();
        let original_hash = hash_file(&original).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,width,height,thumbnail_path,created_at)VALUES('asset','original.png','2026-09-02T12:00:00Z',3,2,'thumb.png','2026-09-02T12:00:00Z')", []).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','original',1,0)", params![original.to_string_lossy(),original_hash]).unwrap();
        drop(connection);

        let first = export_asset_image_in(&root, "asset", &destination).unwrap();
        let second = export_asset_image_in(&root, "asset", &destination).unwrap();
        assert_eq!(first.file_name().unwrap(), "original-export.png");
        assert_eq!(second.file_name().unwrap(), "original-export-2.png");
        assert_eq!(hash_file(&first).unwrap(), original_hash);
        assert_eq!(hash_file(&second).unwrap(), original_hash);

        fs::remove_dir_all(&root).unwrap();
    }
}

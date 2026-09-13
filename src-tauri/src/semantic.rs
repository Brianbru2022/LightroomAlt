//! Local-first semantic discovery.
//!
//! Everything in this module is advisory and derived. The only durable user
//! state is an explicit dismissal/acceptance record; vectors can be deleted and
//! rebuilt without changing a photograph or catalogue organisation object.

use crate::{KeepframeError, Result};
use chrono::{DateTime, Utc};
use image::DynamicImage;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

pub(crate) const MODEL_ID: &str = "google/siglip-base-patch16-224";
pub(crate) const MODEL_REVISION: &str = "7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed";
pub(crate) const MODEL_SHA256: &str =
    "421489ed67220ff3cf58fefe883271f5dd6fd1bca1f2d24157453b5cc8f82f88";
pub(crate) const MODEL_BYTES: u64 = 812_672_320;
pub(crate) const INSTALL_BYTES: u64 = 815_871_927;
pub(crate) const DIMENSION: usize = 768;
pub(crate) const INPUT_RESOLUTION: u32 = 224;
pub(crate) const PREPROCESSING_VERSION: i64 = 1;
pub(crate) const PROVIDER: &str = "Keepframe local SigLIP semantic provider";
pub(crate) const PROVIDER_VERSION: &str = "1.0.0";
pub(crate) const LICENCE: &str = "Apache-2.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticIndexStatus {
    pub available: bool,
    pub installed: bool,
    pub runtime_available: bool,
    pub loaded: bool,
    pub busy: bool,
    pub paused: bool,
    pub provider: String,
    pub provider_version: String,
    pub model: String,
    pub model_revision: String,
    pub licence: String,
    pub source: String,
    pub approximate_bytes: u64,
    pub storage_path: String,
    pub execution_provider: String,
    pub input_resolution: u32,
    pub embedding_dimensions: usize,
    pub total_sources: i64,
    pub indexed_sources: i64,
    pub queued_sources: i64,
    pub failed_sources: i64,
    pub stale_sources: i64,
    pub storage_bytes: i64,
    pub detail: String,
}

pub(crate) struct SemanticRuntimeStatus {
    pub installed: bool,
    pub runtime_available: bool,
    pub loaded: bool,
    pub busy: bool,
    pub paused: bool,
    pub execution_provider: String,
    pub storage_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticFilter {
    pub rating_min: Option<i64>,
    pub decision: Option<String>,
    pub file_type: Option<String>,
    pub edited: Option<bool>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub collection_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticSearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub semantic_weight: Option<f32>,
    #[serde(default)]
    pub filter: SemanticFilter,
}

fn default_limit() -> usize {
    80
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticSearchResult {
    pub asset_id: String,
    pub source_id: String,
    pub score: f32,
    pub semantic_score: f32,
    pub metadata_score: f32,
    pub strength: String,
    pub explanation: Vec<String>,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryGroup {
    pub id: String,
    pub kind: String,
    pub source_ids: Vec<String>,
    pub asset_ids: Vec<String>,
    pub score: f32,
    pub explanation: String,
    pub dismissed: bool,
}

#[derive(Debug, Clone)]
struct Candidate {
    asset_id: String,
    source_id: String,
    filename: String,
    title: String,
    caption: String,
    tags: String,
    captured_at: String,
    missing: bool,
    vector: Vec<f32>,
    perceptual_hash: u64,
    source_hash: String,
}

fn invalid(message: impl Into<String>) -> KeepframeError {
    KeepframeError::Message(message.into())
}

pub(crate) fn encode_vector(vector: &[f32]) -> Result<Vec<u8>> {
    if vector.len() != DIMENSION || vector.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            "The semantic provider returned an invalid embedding vector.",
        ));
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm < 0.000_001 {
        return Err(invalid(
            "The semantic provider returned an empty embedding vector.",
        ));
    }
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&(value / norm).to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn decode_vector(bytes: &[u8], dimension: i64) -> Result<Vec<f32>> {
    if dimension != DIMENSION as i64 || bytes.len() != DIMENSION * 4 {
        return Err(invalid(
            "A semantic embedding has an incompatible dimension or byte length.",
        ));
    }
    let values = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(invalid("A semantic embedding contains a non-finite value."));
    }
    Ok(values)
}

pub(crate) fn perceptual_hash(image: &DynamicImage) -> u64 {
    let small = image
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut hash = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            if small.get_pixel(x + 1, y)[0] >= small.get_pixel(x, y)[0] {
                hash |= 1;
            }
        }
    }
    hash
}

pub(crate) fn upsert_embedding(
    connection: &mut Connection,
    source_id: &str,
    source_hash: &str,
    vector: &[f32],
    perceptual_hash: u64,
) -> Result<()> {
    let bytes = encode_vector(vector)?;
    let hash_bytes = perceptual_hash.to_be_bytes();
    let tx = connection.transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1)",
        [source_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(invalid(
            "The source disappeared before its embedding could be saved.",
        ));
    }
    tx.execute(
        "DELETE FROM semantic_embeddings WHERE source_id=?1 AND (model_id<>?2 OR model_revision<>?3 OR preprocessing_version<>?4)",
        params![source_id, MODEL_ID, MODEL_REVISION, PREPROCESSING_VERSION],
    )?;
    tx.execute(
        "INSERT INTO semantic_embeddings(source_id,model_id,model_revision,preprocessing_version,source_hash,dimension,vector,perceptual_hash,indexed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(source_id,model_id,model_revision,preprocessing_version) DO UPDATE SET
            source_hash=excluded.source_hash,dimension=excluded.dimension,vector=excluded.vector,
            perceptual_hash=excluded.perceptual_hash,indexed_at=excluded.indexed_at",
        params![source_id, MODEL_ID, MODEL_REVISION, PREPROCESSING_VERSION, source_hash, DIMENSION as i64, bytes, hash_bytes.as_slice(), Utc::now().to_rfc3339()],
    )?;
    tx.execute(
        "DELETE FROM semantic_index_queue WHERE source_id=?1",
        [source_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn mark_queue_failure(
    connection: &Connection,
    source_id: &str,
    error: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE semantic_index_queue SET state='failed',attempts=attempts+1,error=?2,updated_at=?3 WHERE source_id=?1",
        params![source_id, truncate(error, 480), Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn mark_queue_running(connection: &Connection, source_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE semantic_index_queue SET state='running',error=NULL,updated_at=?2 WHERE source_id=?1",
        params![source_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn requeue_running(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE semantic_index_queue SET state='queued',updated_at=?1 WHERE state='running'",
        [Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn queue_sources(
    connection: &Connection,
    source_ids: &[String],
    priority: i64,
) -> Result<usize> {
    let now = Utc::now().to_rfc3339();
    let mut changed = 0;
    for source_id in source_ids.iter().collect::<HashSet<_>>() {
        changed += connection.execute(
            "INSERT INTO semantic_index_queue(source_id,priority,state,attempts,error,requested_at,updated_at)
             SELECT id,?2,'queued',0,NULL,?3,?3 FROM sources WHERE id=?1
             ON CONFLICT(source_id) DO UPDATE SET priority=MAX(priority,excluded.priority),state='queued',error=NULL,updated_at=excluded.updated_at",
            params![source_id, priority, now],
        )?;
    }
    Ok(changed)
}

/// Make model/preprocessing upgrades self-invalidating without touching user
/// metadata or repeatedly re-queuing independently failed sources.
pub(crate) fn ensure_current_queue(connection: &Connection) -> Result<usize> {
    let now = Utc::now().to_rfc3339();
    Ok(connection.execute(
        "INSERT OR IGNORE INTO semantic_index_queue(source_id,priority,state,attempts,error,requested_at,updated_at)
         SELECT s.id,0,'queued',0,NULL,?4,?4 FROM sources s
         WHERE s.trashed_at IS NULL AND EXISTS(SELECT 1 FROM representations r WHERE r.source_id=s.id)
         AND NOT EXISTS(
            SELECT 1 FROM semantic_embeddings e WHERE e.source_id=s.id
            AND e.model_id=?1 AND e.model_revision=?2 AND e.preprocessing_version=?3
            AND e.dimension=?5 AND length(e.vector)=?6
         )",
        params![MODEL_ID, MODEL_REVISION, PREPROCESSING_VERSION, now, DIMENSION as i64, (DIMENSION * 4) as i64],
    )?)
}

pub(crate) fn next_queued_source(
    connection: &Connection,
) -> Result<Option<(String, String, String)>> {
    connection
        .query_row(
            "SELECT q.source_id,s.thumbnail_path,(SELECT r.sha256 FROM representations r WHERE r.source_id=s.id ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.id LIMIT 1)
             FROM semantic_index_queue q JOIN sources s ON s.id=q.source_id
             WHERE q.state='queued' AND s.trashed_at IS NULL AND s.missing_state='available'
             AND EXISTS(SELECT 1 FROM representations r WHERE r.source_id=s.id)
             ORDER BY q.priority DESC,q.requested_at,q.source_id LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn rebuild_index(connection: &mut Connection) -> Result<usize> {
    let tx = connection.transaction()?;
    tx.execute("DELETE FROM semantic_embeddings", [])?;
    tx.execute("DELETE FROM semantic_index_queue", [])?;
    let now = Utc::now().to_rfc3339();
    let count = tx.execute(
        "INSERT INTO semantic_index_queue(source_id,priority,state,requested_at,updated_at)
         SELECT s.id,0,'queued',?1,?1 FROM sources s WHERE s.trashed_at IS NULL
         AND s.missing_state='available' AND EXISTS(SELECT 1 FROM representations r WHERE r.source_id=s.id)",
        [&now],
    )?;
    tx.commit()?;
    Ok(count)
}

pub(crate) fn status(
    connection: &Connection,
    runtime: SemanticRuntimeStatus,
) -> Result<SemanticIndexStatus> {
    let total_sources = connection.query_row(
        "SELECT count(*) FROM sources WHERE trashed_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    let indexed_sources = connection.query_row(
        "SELECT count(*) FROM semantic_embeddings e JOIN sources s ON s.id=e.source_id
         WHERE e.model_id=?1 AND e.model_revision=?2 AND e.preprocessing_version=?3
         AND e.dimension=?4 AND length(e.vector)=?5 AND s.trashed_at IS NULL
         AND e.source_hash=COALESCE((SELECT r.sha256 FROM representations r WHERE r.source_id=s.id ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.id LIMIT 1),'')",
        params![MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION,DIMENSION as i64,(DIMENSION*4) as i64],
        |row| row.get(0),
    )?;
    let queued_sources = connection.query_row(
        "SELECT count(*) FROM semantic_index_queue WHERE state IN('queued','running','paused')",
        [],
        |row| row.get(0),
    )?;
    let failed_sources = connection.query_row(
        "SELECT count(*) FROM semantic_index_queue WHERE state='failed'",
        [],
        |row| row.get(0),
    )?;
    let stale_sources = connection.query_row(
        "SELECT count(*) FROM semantic_embeddings e WHERE e.model_id<>?1 OR e.model_revision<>?2 OR e.preprocessing_version<>?3 OR e.dimension<>?4 OR length(e.vector)<>?5",
        params![MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION,DIMENSION as i64,(DIMENSION*4) as i64],
        |row| row.get(0),
    )?;
    let storage_bytes = connection.query_row(
        "SELECT COALESCE(sum(length(vector)+length(perceptual_hash)),0) FROM semantic_embeddings",
        [],
        |row| row.get(0),
    )?;
    let detail = if !runtime.installed {
        "Semantic model not installed. Metadata search and the catalogue remain fully available."
            .into()
    } else if !runtime.runtime_available {
        "The semantic model is installed, but the local Python inference runtime is unavailable."
            .into()
    } else if runtime.paused {
        format!("Semantic indexing paused: {indexed_sources} of {total_sources} sources indexed.")
    } else if runtime.busy || queued_sources > 0 {
        format!("Semantic indexing: {indexed_sources} of {total_sources} sources indexed.")
    } else if indexed_sources == total_sources {
        format!("Semantic index ready: {indexed_sources} sources indexed locally.")
    } else {
        format!("Semantic index incomplete: {indexed_sources} of {total_sources} sources indexed.")
    };
    Ok(SemanticIndexStatus {
        available: runtime.installed && runtime.runtime_available,
        installed: runtime.installed,
        runtime_available: runtime.runtime_available,
        loaded: runtime.loaded,
        busy: runtime.busy,
        paused: runtime.paused,
        provider: PROVIDER.into(),
        provider_version: PROVIDER_VERSION.into(),
        model: MODEL_ID.into(),
        model_revision: MODEL_REVISION.into(),
        licence: LICENCE.into(),
        source: format!("https://huggingface.co/{MODEL_ID}/tree/{MODEL_REVISION}"),
        approximate_bytes: INSTALL_BYTES,
        storage_path: runtime.storage_path,
        execution_provider: runtime.execution_provider,
        input_resolution: INPUT_RESOLUTION,
        embedding_dimensions: DIMENSION,
        total_sources,
        indexed_sources,
        queued_sources,
        failed_sources,
        stale_sources,
        storage_bytes,
        detail,
    })
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

fn load_candidates(connection: &Connection, filter: &SemanticFilter) -> Result<Vec<Candidate>> {
    if filter
        .rating_min
        .is_some_and(|rating| !(0..=5).contains(&rating))
    {
        return Err(invalid(
            "Semantic rating filter must be between zero and five.",
        ));
    }
    if filter
        .decision
        .as_deref()
        .is_some_and(|value| !matches!(value, "keep" | "undecided" | "discard"))
    {
        return Err(invalid("Semantic decision filter is unsupported."));
    }
    let collection_members = if let Some(collection_id) = filter.collection_id.as_deref() {
        let mut statement =
            connection.prepare("SELECT item_id FROM collection_items WHERE collection_id=?1")?;
        let members = statement
            .query_map([collection_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        members
    } else {
        HashSet::new()
    };
    let mut statement = connection.prepare(
        "SELECT a.id,a.source_id,a.filename,COALESCE(a.title,''),COALESCE(a.caption,''),
                COALESCE((SELECT group_concat(t.name,' ') FROM asset_tags at JOIN tags t ON t.id=at.tag_id WHERE at.asset_id=a.id),''),
                a.decision,a.rating,a.captured_at,COALESCE((SELECT r.extension FROM representations r WHERE r.source_id=s.id ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.id LIMIT 1),''),COALESCE(s.missing_state,'available'),
                EXISTS(SELECT 1 FROM develop_recipes d WHERE d.asset_id=a.id),e.dimension,e.vector,e.perceptual_hash,e.source_hash
         FROM assets a JOIN sources s ON s.id=a.source_id
         JOIN semantic_embeddings e ON e.source_id=s.id AND e.model_id=?1 AND e.model_revision=?2 AND e.preprocessing_version=?3
         WHERE a.trashed_at IS NULL AND s.trashed_at IS NULL
         ORDER BY a.source_id,a.is_primary DESC,a.version_index,a.id",
    )?;
    let rows = statement.query_map(
        params![MODEL_ID, MODEL_REVISION, PREPROCESSING_VERSION],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, bool>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Vec<u8>>(13)?,
                row.get::<_, Vec<u8>>(14)?,
                row.get::<_, String>(15)?,
            ))
        },
    )?;
    let mut candidates = Vec::new();
    let mut chosen_sources = HashSet::new();
    for row in rows {
        let (
            asset_id,
            source_id,
            filename,
            title,
            caption,
            tags,
            decision,
            rating,
            captured_at,
            file_type,
            missing,
            edited,
            dimension,
            bytes,
            hash_bytes,
            source_hash,
        ) = row?;
        if chosen_sources.contains(&source_id)
            || filter.rating_min.is_some_and(|minimum| rating < minimum)
            || filter
                .decision
                .as_ref()
                .is_some_and(|wanted| wanted != &decision)
            || filter
                .file_type
                .as_ref()
                .is_some_and(|wanted| !file_type.eq_ignore_ascii_case(wanted))
            || filter.edited.is_some_and(|wanted| edited != wanted)
            || filter
                .date_from
                .as_ref()
                .is_some_and(|from| captured_at < *from)
            || filter.date_to.as_ref().is_some_and(|to| captured_at > *to)
            || (filter.collection_id.is_some() && !collection_members.contains(&asset_id))
        {
            continue;
        }
        let vector = decode_vector(&bytes, dimension)?;
        let perceptual_hash = if hash_bytes.len() == 8 {
            u64::from_be_bytes(hash_bytes.try_into().expect("length checked"))
        } else {
            return Err(invalid(
                "A semantic perceptual hash has an invalid byte length.",
            ));
        };
        chosen_sources.insert(source_id.clone());
        candidates.push(Candidate {
            asset_id,
            source_id,
            filename,
            title,
            caption,
            tags,
            captured_at,
            missing: missing != "available",
            vector,
            perceptual_hash,
            source_hash,
        });
    }
    Ok(candidates)
}

fn metadata_score(candidate: &Candidate, query: &str) -> f32 {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|term| term.len() > 1)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return 0.0;
    }
    let haystack = format!(
        "{} {} {} {}",
        candidate.filename, candidate.title, candidate.caption, candidate.tags
    )
    .to_lowercase();
    terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count() as f32
        / terms.len() as f32
}

fn strength(score: f32) -> String {
    if score >= 0.72 {
        "Strong match"
    } else if score >= 0.48 {
        "Moderate match"
    } else {
        "Possible match"
    }
    .into()
}

pub(crate) fn search(
    connection: &Connection,
    query_vector: &[f32],
    request: &SemanticSearchRequest,
) -> Result<Vec<SemanticSearchResult>> {
    let query = request.query.trim();
    if query.is_empty() || query.chars().count() > 240 {
        return Err(invalid(
            "Enter a semantic query between 1 and 240 characters.",
        ));
    }
    let query_vector = decode_vector(&encode_vector(query_vector)?, DIMENSION as i64)?;
    let semantic_weight = request.semantic_weight.unwrap_or(0.85).clamp(0.5, 1.0);
    let limit = request.limit.clamp(1, 240);
    let mut results = load_candidates(connection, &request.filter)?
        .into_iter()
        .map(|candidate| {
            let semantic_score =
                ((cosine(&query_vector, &candidate.vector) + 1.0) / 2.0).clamp(0.0, 1.0);
            let metadata_score = metadata_score(&candidate, query);
            let score = semantic_score * semantic_weight + metadata_score * (1.0 - semantic_weight);
            let mut explanation = vec!["Local source-image semantic similarity".into()];
            if metadata_score > 0.0 {
                explanation.push("Keyword, title, caption or filename match".into());
            }
            if request.filter.rating_min.is_some()
                || request.filter.decision.is_some()
                || request.filter.edited.is_some()
            {
                explanation.push("Deterministic catalogue filter".into());
            }
            SemanticSearchResult {
                asset_id: candidate.asset_id,
                source_id: candidate.source_id,
                score,
                semantic_score,
                metadata_score,
                strength: strength(score),
                explanation,
                missing: candidate.missing,
            }
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    results.truncate(limit);
    Ok(results)
}

pub(crate) fn find_similar(
    connection: &Connection,
    item_id: &str,
    limit: usize,
) -> Result<Vec<SemanticSearchResult>> {
    let source_id: String = connection
        .query_row(
            "SELECT source_id FROM assets WHERE id=?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("That catalogue item no longer exists."))?;
    let (dimension,bytes):(i64,Vec<u8>)=connection.query_row("SELECT dimension,vector FROM semantic_embeddings WHERE source_id=?1 AND model_id=?2 AND model_revision=?3 AND preprocessing_version=?4",params![source_id,MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION],|row|Ok((row.get(0)?,row.get(1)?))).optional()?.ok_or_else(||invalid("The selected source has not been indexed yet."))?;
    let needle = decode_vector(&bytes, dimension)?;
    let mut results = load_candidates(connection, &SemanticFilter::default())?
        .into_iter()
        .filter(|candidate| candidate.source_id != source_id)
        .map(|candidate| {
            let score = ((cosine(&needle, &candidate.vector) + 1.0) / 2.0).clamp(0.0, 1.0);
            SemanticSearchResult {
                asset_id: candidate.asset_id,
                source_id: candidate.source_id,
                score,
                semantic_score: score,
                metadata_score: 0.0,
                strength: strength(score),
                explanation: vec!["Source-image cosine similarity".into()],
                missing: candidate.missing,
            }
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    results.truncate(limit.clamp(1, 240));
    Ok(results)
}

fn suggestion_id(kind: &str, source_ids: &[String]) -> String {
    let mut sorted = source_ids.to_vec();
    sorted.sort();
    format!(
        "{:x}",
        Sha256::digest(
            format!("{kind}|{MODEL_ID}|{MODEL_REVISION}|{}", sorted.join("|")).as_bytes()
        )
    )
}

pub(crate) fn duplicate_groups(connection: &Connection) -> Result<Vec<DiscoveryGroup>> {
    let candidates = load_candidates(connection, &SemanticFilter::default())?;
    let mut groups = Vec::new();
    let mut exact: HashMap<String, Vec<&Candidate>> = HashMap::new();
    for candidate in &candidates {
        exact
            .entry(candidate.source_hash.clone())
            .or_default()
            .push(candidate);
    }
    for members in exact.values().filter(|members| members.len() > 1) {
        let source_ids = members
            .iter()
            .map(|item| item.source_id.clone())
            .collect::<Vec<_>>();
        groups.push(make_group(
            connection,
            "exact_duplicate",
            source_ids,
            members.iter().map(|item| item.asset_id.clone()).collect(),
            1.0,
            "Identical protected-source SHA-256",
        ));
    }
    // Locality buckets prevent a naive catalogue-wide all-pairs pass. Multiple
    // projections keep the filter conservative while allowing small dHash
    // changes in any one region to meet in at least one bucket.
    let mut buckets: HashMap<(u8, u16), Vec<&Candidate>> = HashMap::new();
    for candidate in &candidates {
        for projection in 0..4_u8 {
            let shift = projection as u32 * 16;
            buckets
                .entry((
                    projection,
                    ((candidate.perceptual_hash >> shift) & 0xffff) as u16,
                ))
                .or_default()
                .push(candidate);
        }
    }
    let mut considered = HashSet::new();
    let mut neighbours: HashMap<String, HashSet<String>> = HashMap::new();
    for members in buckets.values() {
        for index in 0..members.len() {
            for right_index in index + 1..members.len() {
                let left = members[index];
                let right = members[right_index];
                let key = if left.source_id < right.source_id {
                    (left.source_id.clone(), right.source_id.clone())
                } else {
                    (right.source_id.clone(), left.source_id.clone())
                };
                if !considered.insert(key) || left.source_hash == right.source_hash {
                    continue;
                }
                let hamming = (left.perceptual_hash ^ right.perceptual_hash).count_ones();
                let visual = ((cosine(&left.vector, &right.vector) + 1.0) / 2.0).clamp(0.0, 1.0);
                if hamming <= 6 && visual >= 0.985 {
                    neighbours
                        .entry(left.source_id.clone())
                        .or_default()
                        .insert(right.source_id.clone());
                    neighbours
                        .entry(right.source_id.clone())
                        .or_default()
                        .insert(left.source_id.clone());
                }
            }
        }
    }
    let by_source = candidates
        .iter()
        .map(|candidate| (candidate.source_id.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let mut used = HashSet::new();
    for candidate in &candidates {
        if used.contains(&candidate.source_id) {
            continue;
        }
        let mut component = vec![candidate.source_id.clone()];
        let mut cursor = 0;
        while cursor < component.len() {
            if let Some(next) = neighbours.get(&component[cursor]) {
                for source in next {
                    if !component.contains(source) {
                        component.push(source.clone());
                    }
                }
            }
            cursor += 1;
        }
        if component.len() > 1 {
            component.sort();
            for source in &component {
                used.insert(source.clone());
            }
            let asset_ids = component
                .iter()
                .filter_map(|source| {
                    by_source
                        .get(source.as_str())
                        .map(|candidate| candidate.asset_id.clone())
                })
                .collect();
            groups.push(make_group(
                connection,
                "near_duplicate",
                component,
                asset_ids,
                0.985,
                "Conservative bucketed perceptual-hash and embedding agreement",
            ));
        }
    }
    Ok(groups)
}

pub(crate) fn burst_suggestions(connection: &Connection) -> Result<Vec<DiscoveryGroup>> {
    let mut candidates = load_candidates(connection, &SemanticFilter::default())?;
    candidates.sort_by(|a, b| {
        a.captured_at
            .cmp(&b.captured_at)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    let mut groups = Vec::new();
    let mut index = 0;
    while index < candidates.len() {
        let mut members = vec![&candidates[index]];
        let mut next = index + 1;
        while next < candidates.len() {
            let previous = DateTime::parse_from_rfc3339(&candidates[next - 1].captured_at).ok();
            let current = DateTime::parse_from_rfc3339(&candidates[next].captured_at).ok();
            let seconds = previous
                .zip(current)
                .map(|(a, b)| (b - a).num_seconds().abs())
                .unwrap_or(i64::MAX);
            let score = ((cosine(&candidates[index].vector, &candidates[next].vector) + 1.0) / 2.0)
                .clamp(0.0, 1.0);
            if seconds <= 3 && score >= 0.94 {
                members.push(&candidates[next]);
                next += 1;
            } else {
                break;
            }
        }
        if members.len() > 1 {
            let source_ids = members
                .iter()
                .map(|item| item.source_id.clone())
                .collect::<Vec<_>>();
            groups.push(make_group(
                connection,
                "burst",
                source_ids,
                members.iter().map(|item| item.asset_id.clone()).collect(),
                0.94,
                "Capture times within three seconds and strong visual similarity",
            ));
        }
        index = next.max(index + 1);
    }
    Ok(groups)
}

fn make_group(
    connection: &Connection,
    kind: &str,
    source_ids: Vec<String>,
    asset_ids: Vec<String>,
    score: f32,
    explanation: &str,
) -> DiscoveryGroup {
    let id = suggestion_id(kind, &source_ids);
    let dismissed = connection
        .query_row(
            "SELECT decision='dismissed' FROM semantic_suggestion_decisions WHERE identity_hash=?1",
            [&id],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false);
    DiscoveryGroup {
        id,
        kind: kind.into(),
        source_ids,
        asset_ids,
        score,
        explanation: explanation.into(),
        dismissed,
    }
}

pub(crate) fn record_suggestion_decision(
    connection: &Connection,
    group: &DiscoveryGroup,
    decision: &str,
) -> Result<()> {
    if !matches!(decision, "dismissed" | "accepted") {
        return Err(invalid(
            "Suggestion decision must be accepted or dismissed.",
        ));
    }
    if group.id != suggestion_id(&group.kind, &group.source_ids) {
        return Err(invalid("Suggestion identity did not match its members."));
    }
    connection.execute("INSERT INTO semantic_suggestion_decisions(identity_hash,kind,decision,model_id,model_revision,member_source_ids_json,decided_at)VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(identity_hash) DO UPDATE SET decision=excluded.decision,decided_at=excluded.decided_at",params![group.id,group.kind,decision,MODEL_ID,MODEL_REVISION,serde_json::to_string(&group.source_ids)?,Utc::now().to_rfc3339()])?;
    Ok(())
}

pub(crate) fn propose_smart_collection(input: &str) -> Result<Value> {
    let text = input.trim();
    if text.is_empty() || text.chars().count() > 240 {
        return Err(invalid("Enter a short Smart Collection request."));
    }
    let lower = text.to_lowercase();
    let mut rules = Vec::new();
    for rating in (1..=5).rev() {
        if lower.contains(&format!("{rating} star")) || lower.contains(&format!("{rating}+")) {
            let operator = if lower.contains("or higher")
                || lower.contains("at least")
                || lower.contains(&format!("{rating}+"))
            {
                "gte"
            } else if lower.contains("or lower") || lower.contains("at most") {
                "lte"
            } else {
                "equals"
            };
            rules.push(json!({"field":"rating","operator":operator,"value":rating}));
            break;
        }
    }
    if lower.contains("edited") {
        rules.push(json!({"field":"edited","operator":"is","value":!lower.contains("unedited")}));
    }
    for (phrase, value) in [
        ("picked", "keep"),
        ("kept", "keep"),
        ("rejected", "discard"),
        ("unflagged", "undecided"),
    ] {
        if lower.contains(phrase) {
            rules.push(json!({"field":"flag","operator":"is","value":value}));
            break;
        }
    }
    if lower.contains("virtual version") {
        rules.push(json!({"field":"versionStatus","operator":"is","value":"virtual"}));
    } else if lower.contains("primary") {
        rules.push(json!({"field":"versionStatus","operator":"is","value":"primary"}));
    }
    if let Some(year) = lower.split(|c: char| !c.is_ascii_digit()).find(|part| {
        part.len() == 4
            && part
                .parse::<i32>()
                .is_ok_and(|year| (1900..=2200).contains(&year))
    }) {
        rules.push(json!({"field":"captureDate","operator":"between","value":format!("{year}-01-01T00:00:00Z"),"secondValue":format!("{year}-12-31T23:59:59Z")}));
    }
    if let Some(camera) = extract_after(&lower, "camera ")
        .or_else(|| extract_after(&lower, "with the "))
        .or_else(|| extract_after(&lower, "taken with "))
    {
        if camera.len() >= 2 {
            rules.push(json!({"field":"camera","operator":"contains","value":camera}));
        }
    }
    if rules.is_empty() {
        return Err(invalid(
            "That request does not map safely to supported deterministic Smart Collection fields.",
        ));
    }
    Ok(
        json!({"name":truncate(text,64),"matchMode":"all","rules":rules,"explanation":"Deterministic local parser; review and edit every rule before saving.","requiresExplicitSave":true}),
    )
}

fn extract_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    text.split_once(marker)
        .map(|(_, tail)| {
            tail.split(" in ")
                .next()
                .unwrap_or(tail)
                .trim_matches(|c: char| c == ',' || c == '.' || c.is_whitespace())
        })
        .filter(|value| !value.is_empty())
}
fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

pub(crate) fn health_issues(connection: &Connection) -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let orphan:i64=connection.query_row("SELECT count(*) FROM semantic_embeddings e LEFT JOIN sources s ON s.id=e.source_id WHERE s.id IS NULL",[],|row|row.get(0))?;
    let bad:i64=connection.query_row("SELECT count(*) FROM semantic_embeddings WHERE dimension<>?1 OR length(vector)<>?2 OR length(perceptual_hash)<>8",params![DIMENSION as i64,(DIMENSION*4) as i64],|row|row.get(0))?;
    let stale:i64=connection.query_row("SELECT count(*) FROM semantic_embeddings WHERE model_id<>?1 OR model_revision<>?2 OR preprocessing_version<>?3",params![MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION],|row|row.get(0))?;
    let missing:i64=connection.query_row("SELECT count(*) FROM sources s WHERE s.trashed_at IS NULL AND s.missing_state='available' AND EXISTS(SELECT 1 FROM representations r WHERE r.source_id=s.id) AND NOT EXISTS(SELECT 1 FROM semantic_embeddings e WHERE e.source_id=s.id AND e.model_id=?1 AND e.model_revision=?2 AND e.preprocessing_version=?3 AND e.dimension=?4 AND length(e.vector)=?5)",params![MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION,DIMENSION as i64,(DIMENSION*4) as i64],|row|row.get(0))?;
    let mismatch:i64=connection.query_row("SELECT count(*) FROM semantic_embeddings e WHERE e.source_hash<>COALESCE((SELECT r.sha256 FROM representations r WHERE r.source_id=e.source_id ORDER BY CASE WHEN lower(r.extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN r.is_raw=1 THEN 1 ELSE 2 END,r.id LIMIT 1),'')",[],|row|row.get(0))?;
    if orphan > 0 {
        issues.push(format!("{orphan} orphan semantic embeddings"));
    }
    if bad > 0 {
        issues.push(format!("{bad} corrupt semantic embedding payloads"));
    }
    if stale > 0 {
        issues.push(format!(
            "{stale} embeddings from an incompatible model revision"
        ));
    }
    if missing > 0 {
        issues.push(format!(
            "{missing} sources missing a current semantic embedding"
        ));
    }
    if mismatch > 0 {
        issues.push(format!("{mismatch} source-hash embedding mismatches"));
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn schema() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE sources(id TEXT PRIMARY KEY,trashed_at TEXT,missing_state TEXT NOT NULL DEFAULT 'available',thumbnail_path TEXT);CREATE TABLE assets(id TEXT PRIMARY KEY,source_id TEXT NOT NULL REFERENCES sources(id),filename TEXT,decision TEXT,rating INTEGER,title TEXT,caption TEXT,captured_at TEXT,is_primary INTEGER,version_index INTEGER,trashed_at TEXT);CREATE TABLE representations(id TEXT PRIMARY KEY,source_id TEXT REFERENCES sources(id),sha256 TEXT,extension TEXT,is_raw INTEGER);CREATE INDEX idx_test_representations_source ON representations(source_id,is_raw,id);CREATE TABLE tags(id TEXT PRIMARY KEY,name TEXT);CREATE TABLE asset_tags(asset_id TEXT,tag_id TEXT);CREATE INDEX idx_test_asset_tags_asset ON asset_tags(asset_id);CREATE TABLE develop_recipes(asset_id TEXT PRIMARY KEY);CREATE TABLE collection_items(collection_id TEXT,item_id TEXT);CREATE TABLE semantic_embeddings(source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,model_id TEXT NOT NULL,model_revision TEXT NOT NULL,preprocessing_version INTEGER NOT NULL,source_hash TEXT NOT NULL,dimension INTEGER NOT NULL,vector BLOB NOT NULL,perceptual_hash BLOB NOT NULL,indexed_at TEXT NOT NULL,PRIMARY KEY(source_id,model_id,model_revision,preprocessing_version));CREATE TABLE semantic_index_queue(source_id TEXT PRIMARY KEY REFERENCES sources(id),priority INTEGER,state TEXT,attempts INTEGER DEFAULT 0,error TEXT,requested_at TEXT,updated_at TEXT);CREATE TABLE semantic_suggestion_decisions(identity_hash TEXT PRIMARY KEY,kind TEXT,decision TEXT,model_id TEXT,model_revision TEXT,member_source_ids_json TEXT,decided_at TEXT);").unwrap();
        connection
    }
    fn vector(seed: usize) -> Vec<f32> {
        let mut value = vec![0.0; DIMENSION];
        value[seed % DIMENSION] = 1.0;
        value
    }
    fn insert(connection: &mut Connection, id: &str, seed: usize, hash: &str, time: &str) {
        connection
            .execute(
                "INSERT INTO sources(id,thumbnail_path)VALUES(?1,'thumb')",
                [id],
            )
            .unwrap();
        connection.execute("INSERT INTO assets VALUES(?1,?1,?2,'keep',4,'','','2026-01-01T00:00:00Z',1,1,NULL)",params![id,format!("{id}.jpg")]).unwrap();
        connection
            .execute(
                "UPDATE assets SET captured_at=?2 WHERE id=?1",
                params![id, time],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO representations VALUES(?1,?1,?2,'jpg',0)",
                params![id, hash],
            )
            .unwrap();
        upsert_embedding(connection, id, hash, &vector(seed), 0).unwrap();
    }
    #[test]
    fn vectors_are_normalised_binary_and_reject_wrong_dimensions() {
        let encoded = encode_vector(&vector(1)).unwrap();
        assert_eq!(encoded.len(), DIMENSION * 4);
        assert!((decode_vector(&encoded, DIMENSION as i64).unwrap()[1] - 1.0).abs() < 0.001);
        assert!(encode_vector(&[1.0]).is_err());
        assert!(decode_vector(&encoded, 512).is_err());
    }
    #[test]
    fn embedding_identity_is_source_level_and_model_upgrade_removes_stale_rows() {
        let mut c = schema();
        insert(&mut c, "source", 2, "hash", "2026-01-01T00:00:00Z");
        c.execute("INSERT INTO assets VALUES('version','source','source.jpg','discard',1,'','','2026-01-01T00:00:00Z',0,2,NULL)",[]).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM semantic_embeddings", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        c.execute("UPDATE semantic_embeddings SET model_revision='old'", [])
            .unwrap();
        upsert_embedding(&mut c, "source", "hash", &vector(3), 1).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM semantic_embeddings WHERE model_revision=?1",
                [MODEL_REVISION],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn indexing_queue_is_incremental_deduplicated_and_upgrade_aware() {
        let mut c = schema();
        insert(&mut c, "source", 2, "hash", "2026-01-01T00:00:00Z");
        assert_eq!(ensure_current_queue(&c).unwrap(), 0);
        c.execute("UPDATE semantic_embeddings SET model_revision='old'", [])
            .unwrap();
        assert_eq!(ensure_current_queue(&c).unwrap(), 1);
        assert_eq!(ensure_current_queue(&c).unwrap(), 0);
        queue_sources(&c, &["source".into(), "source".into()], 50).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM semantic_index_queue WHERE source_id='source' AND priority=50",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn text_and_image_search_are_ranked_tie_stable_filtered_and_collapse_siblings() {
        let mut c = schema();
        insert(&mut c, "a", 1, "ha", "2026-01-01T00:00:00Z");
        insert(&mut c, "b", 2, "hb", "2026-01-01T00:00:04Z");
        c.execute("INSERT INTO assets VALUES('a-v','a','a.jpg','keep',5,'dog beach','','2026-01-01T00:00:00Z',0,2,NULL)",[]).unwrap();
        let request = SemanticSearchRequest {
            query: "dog beach".into(),
            limit: 20,
            semantic_weight: Some(0.8),
            filter: SemanticFilter {
                rating_min: Some(5),
                ..Default::default()
            },
        };
        let results = search(&c, &vector(1), &request).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_id, "a");
        let similar = find_similar(&c, "a", 20).unwrap();
        assert_eq!(similar.len(), 1);
        assert_eq!(similar[0].source_id, "b");
    }
    #[test]
    fn near_duplicate_and_burst_suggestions_are_conservative_dismissible_and_do_not_mutate_assets()
    {
        let mut c = schema();
        insert(&mut c, "a", 1, "ha", "2026-01-01T00:00:00Z");
        insert(&mut c, "b", 1, "hb", "2026-01-01T00:00:02Z");
        let groups = duplicate_groups(&c).unwrap();
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        record_suggestion_decision(&c, group, "dismissed").unwrap();
        assert!(duplicate_groups(&c).unwrap()[0].dismissed);
        assert_eq!(burst_suggestions(&c).unwrap().len(), 1);
        assert_eq!(
            c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn exact_duplicates_are_hash_authoritative_and_same_colour_is_not_enough() {
        let mut c = schema();
        insert(&mut c, "a", 1, "same", "2026-01-01T00:00:00Z");
        insert(&mut c, "b", 400, "same", "2026-01-01T00:10:00Z");
        insert(&mut c, "c", 500, "different", "2026-01-01T00:20:00Z");
        let groups = duplicate_groups(&c).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, "exact_duplicate");
        assert_eq!(groups[0].source_ids.len(), 2);
    }
    #[test]
    fn deterministic_smart_proposals_emit_allow_listed_rules_and_never_sql() {
        let proposal =
            propose_smart_collection("4 stars or higher taken with the Nikon in 2025").unwrap();
        assert_eq!(proposal["requiresExplicitSave"], true);
        assert_eq!(proposal["rules"][0]["field"], "rating");
        assert!(!proposal.to_string().to_lowercase().contains("select "));
        assert!(propose_smart_collection("surprise me").is_err());
    }
    #[test]
    fn source_hash_mismatch_and_corrupt_dimensions_are_health_issues() {
        let mut c = schema();
        insert(&mut c, "a", 1, "ha", "2026-01-01T00:00:00Z");
        c.execute("UPDATE representations SET sha256='changed'", [])
            .unwrap();
        assert!(health_issues(&c)
            .unwrap()
            .iter()
            .any(|issue| issue.contains("source-hash")));
    }
    #[test]
    fn embedding_preparation_never_changes_source_pixels() {
        let path =
            std::env::temp_dir().join(format!("keepframe-semantic-{}.png", uuid::Uuid::new_v4()));
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            48,
            image::Rgb([32, 96, 160]),
        ));
        image.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let decoded = image::open(&path).unwrap();
        let _hash = perceptual_hash(&decoded);
        let after = std::fs::read(&path).unwrap();
        assert_eq!(Sha256::digest(&before), Sha256::digest(&after));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    #[ignore = "manual Milestone 14 20k-vector benchmark; run scripts/benchmark-semantic-discovery.ps1"]
    fn semantic_retrieval_twenty_thousand_sources() {
        let mut c = schema();
        let started = std::time::Instant::now();
        {
            let tx = c.transaction().unwrap();
            for index in 0..20_000 {
                let id = format!("s{index:05}");
                let hash = format!("h{index}");
                let bytes = encode_vector(&vector(index % DIMENSION)).unwrap();
                tx.execute(
                    "INSERT INTO sources(id,thumbnail_path)VALUES(?1,'thumb')",
                    [&id],
                )
                .unwrap();
                tx.execute("INSERT INTO assets VALUES(?1,?1,?2,'keep',4,'','','2026-01-01T00:00:00Z',1,1,NULL)",params![id,format!("s{index:05}.jpg")]).unwrap();
                tx.execute(
                    "INSERT INTO representations VALUES(?1,?1,?2,'jpg',0)",
                    params![id, hash],
                )
                .unwrap();
                let phash = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                tx.execute("INSERT INTO semantic_embeddings VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'2026-01-01T00:00:00Z')",params![id,MODEL_ID,MODEL_REVISION,PREPROCESSING_VERSION,hash,DIMENSION as i64,bytes,phash.to_be_bytes().as_slice()]).unwrap();
            }
            tx.commit().unwrap();
        }
        let build = started.elapsed();
        let started = std::time::Instant::now();
        let results = search(
            &c,
            &vector(71),
            &SemanticSearchRequest {
                query: "mountain".into(),
                limit: 80,
                semantic_weight: Some(1.0),
                filter: SemanticFilter::default(),
            },
        )
        .unwrap();
        let search_20k = started.elapsed();
        let started = std::time::Instant::now();
        let similar = find_similar(&c, "s00071", 80).unwrap();
        let similar_20k = started.elapsed();
        let started = std::time::Instant::now();
        assert!(duplicate_groups(&c).unwrap().is_empty());
        let duplicates_20k = started.elapsed();
        c.execute(
            "DELETE FROM semantic_embeddings WHERE source_id>='s10000'",
            [],
        )
        .unwrap();
        c.execute("DELETE FROM representations WHERE source_id>='s10000'", [])
            .unwrap();
        c.execute("DELETE FROM assets WHERE source_id>='s10000'", [])
            .unwrap();
        c.execute("DELETE FROM sources WHERE id>='s10000'", [])
            .unwrap();
        let started = std::time::Instant::now();
        let results_10k = search(
            &c,
            &vector(71),
            &SemanticSearchRequest {
                query: "mountain".into(),
                limit: 80,
                semantic_weight: Some(1.0),
                filter: SemanticFilter::default(),
            },
        )
        .unwrap();
        let search_10k = started.elapsed();
        eprintln!("M14_SEMANTIC_BENCHMARK sources=20000 dimensions={DIMENSION} build_ms={} search_10000_ms={} search_20000_ms={} find_similar_20000_ms={} duplicate_scan_20000_ms={} storage_bytes={}",build.as_millis(),search_10k.as_millis(),search_20k.as_millis(),similar_20k.as_millis(),duplicates_20k.as_millis(),61_600_000);
        assert_eq!(results.len(), 80);
        assert_eq!(results_10k.len(), 80);
        assert_eq!(similar.len(), 80);
        assert!(search_20k < std::time::Duration::from_secs(5));
        assert!(duplicates_20k < std::time::Duration::from_secs(5));
    }
}

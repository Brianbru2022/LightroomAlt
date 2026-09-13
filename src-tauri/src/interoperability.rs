//! Local, durable interchange and library-resilience helpers.
//!
//! These routines deliberately never change image pixels.  Sidecars and catalogue
//! exports are staged before promotion, and any relink is hash-confirmed.

use super::{hash_file, normalise_tags, DevelopRecipe, KeepframeError, Result};
use chrono::Utc;
use quick_xml::{events::Event, Reader};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub(crate) const PORTABLE_CATALOGUE_SCHEMA_VERSION: i64 = 5;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SidecarExportSummary {
    pub requested: usize,
    pub written: usize,
    pub preserved_existing: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SidecarImportResult {
    pub tags_imported: bool,
    pub location_imported: bool,
    pub triage_imported: bool,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IntegrityFinding {
    pub asset_id: Option<String>,
    pub filename: Option<String>,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IntegrityReport {
    pub scanned_assets: usize,
    pub missing_originals: usize,
    pub missing_derived_versions: usize,
    pub modified_originals: usize,
    pub missing_ai_derivatives: usize,
    pub modified_ai_derivatives: usize,
    pub orphan_ai_derivatives: usize,
    pub unsupported_ai_provenance: usize,
    pub untracked_managed_files: usize,
    pub sidecar_conflicts: usize,
    pub findings: Vec<IntegrityFinding>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelinkCandidate {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug)]
struct SidecarAsset {
    id: String,
    filename: String,
    decision: String,
    captured_at: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    embedded_latitude: Option<f64>,
    embedded_longitude: Option<f64>,
    manual_latitude: Option<f64>,
    manual_longitude: Option<f64>,
    location_source: String,
    original_path: PathBuf,
    tags: Vec<String>,
}

#[derive(Debug, Default)]
struct SidecarMetadata {
    tags: Vec<String>,
    hierarchy: Vec<String>,
    decision: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    location_source: Option<String>,
}

fn sidecar_path(original: &Path) -> PathBuf {
    original.with_extension("xmp")
}

fn xml_safe(value: &str) -> String {
    value
        .chars()
        .filter(|character| matches!(*character, '\t' | '\n' | '\r') || *character >= ' ')
        .flat_map(|character| match character {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '\"' => "&quot;".chars().collect(),
            '\'' => "&apos;".chars().collect(),
            _ => vec![character],
        })
        .collect()
}

fn validate_xml(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut depth = 0i32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) if depth == 0 => return Ok(()),
            Ok(Event::Eof) => {
                return Err(KeepframeError::Message(
                    "XMP ended before all elements were closed.".into(),
                ))
            }
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) if depth > 0 => depth -= 1,
            Ok(Event::End(_)) => {
                return Err(KeepframeError::Message(
                    "XMP contains an unexpected closing element.".into(),
                ))
            }
            Ok(_) => {}
            Err(error) => {
                return Err(KeepframeError::Message(format!(
                    "Generated XMP failed XML validation: {error}"
                )))
            }
        }
        buffer.clear();
    }
}

fn read_asset(connection: &Connection, asset_id: &str) -> Result<SidecarAsset> {
    let mut asset = connection.query_row(
        "SELECT a.id,s.filename,a.decision,s.captured_at,s.latitude,s.longitude,s.embedded_latitude,s.embedded_longitude,s.manual_latitude,s.manual_longitude,COALESCE(s.location_source,'none'),r.path FROM assets a JOIN sources s ON s.id=a.source_id JOIN representations r ON r.id=(SELECT r2.id FROM representations r2 WHERE r2.source_id=a.source_id ORDER BY r2.is_raw ASC,r2.path ASC LIMIT 1) WHERE a.id=?1 AND a.is_primary=1 AND a.trashed_at IS NULL AND NOT EXISTS(SELECT 1 FROM ai_derivatives d WHERE d.derived_source_id=s.id)",
        [asset_id],
        |row| Ok(SidecarAsset { id: row.get(0)?, filename: row.get(1)?, decision: row.get(2)?, captured_at: row.get(3)?, latitude: row.get(4)?, longitude: row.get(5)?, embedded_latitude: row.get(6)?, embedded_longitude: row.get(7)?, manual_latitude: row.get(8)?, manual_longitude: row.get(9)?, location_source: row.get(10)?, original_path: PathBuf::from(row.get::<_, String>(11)?), tags: Vec::new() }),
    ).optional()?.ok_or_else(|| KeepframeError::Message("The selected catalogue item has no exportable original representation.".into()))?;
    asset.tags = connection.prepare("SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=?1 ORDER BY t.name COLLATE NOCASE")?
        .query_map([asset_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(asset)
}

fn xmp_for(asset: &SidecarAsset) -> Vec<u8> {
    let portable_label = match asset.decision.as_str() {
        "keep" => "Keep",
        "discard" => "Discard",
        _ => "Undecided",
    };
    let tags = asset
        .tags
        .iter()
        .map(|tag| format!("<rdf:li>{}</rdf:li>", xml_safe(tag)))
        .collect::<String>();
    let hierarchy = asset
        .tags
        .iter()
        .filter(|tag| tag.contains('/'))
        .map(|tag| format!("<rdf:li>{}</rdf:li>", xml_safe(tag)))
        .collect::<String>();
    let location = match (asset.latitude, asset.longitude) {
        (Some(latitude), Some(longitude)) => format!("<exif:GPSLatitude>{latitude:.8}</exif:GPSLatitude><exif:GPSLongitude>{longitude:.8}</exif:GPSLongitude>"),
        _ => String::new(),
    };
    let provenance = [
        asset
            .embedded_latitude
            .map(|value| format!(" keepframe:embeddedLatitude=\"{value:.8}\"")),
        asset
            .embedded_longitude
            .map(|value| format!(" keepframe:embeddedLongitude=\"{value:.8}\"")),
        asset
            .manual_latitude
            .map(|value| format!(" keepframe:manualLatitude=\"{value:.8}\"")),
        asset
            .manual_longitude
            .map(|value| format!(" keepframe:manualLongitude=\"{value:.8}\"")),
    ]
    .into_iter()
    .flatten()
    .collect::<String>();
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:lr="http://ns.adobe.com/lightroom/1.0/" xmlns:keepframe="https://keepframe.app/ns/1.0/" xmp:CreateDate="{}" xmp:Label="{}" keepframe:triage="{}" keepframe:locationSource="{}" keepframe:assetId="{}"{}><dc:subject><rdf:Bag>{}</rdf:Bag></dc:subject><lr:hierarchicalSubject><rdf:Bag>{}</rdf:Bag></lr:hierarchicalSubject>{}</rdf:Description></rdf:RDF></x:xmpmeta>"#,
        xml_safe(&asset.captured_at), portable_label, xml_safe(&asset.decision), xml_safe(&asset.location_source), xml_safe(&asset.id), provenance, tags, hierarchy, location).into_bytes()
}

fn staged_write(destination: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    validate_xml(bytes)?;
    let temporary = destination.with_extension("xmp.partial");
    fs::write(&temporary, bytes).map_err(|error| {
        KeepframeError::Message(format!(
            "Could not stage XMP sidecar {}: {error}",
            temporary.display()
        ))
    })?;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            KeepframeError::Message(format!(
                "Could not validate staged XMP sidecar {}: {error}",
                temporary.display()
            ))
        })?
        .sync_all()?;
    if destination.exists() && !replace {
        let _ = fs::remove_file(&temporary);
        return Err(KeepframeError::Message(
            "An XMP sidecar already exists; choose replace explicitly to update it.".into(),
        ));
    }
    if destination.exists() {
        let backup = destination.with_extension("xmp.keepframe-backup");
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        fs::rename(destination, &backup)?;
        if let Err(error) = fs::rename(&temporary, destination) {
            let _ = fs::rename(&backup, destination);
            return Err(KeepframeError::Io(error));
        }
        fs::remove_file(backup)?;
    } else {
        fs::rename(&temporary, destination).map_err(|error| {
            KeepframeError::Message(format!(
                "Could not promote staged XMP sidecar {}: {error}",
                destination.display()
            ))
        })?;
    }
    Ok(())
}

pub(crate) fn export_sidecars(
    connection: &mut Connection,
    asset_ids: &[String],
    replace: bool,
) -> Result<SidecarExportSummary> {
    let mut unique = HashSet::new();
    let ids = asset_ids
        .iter()
        .filter(|id| unique.insert(id.as_str()))
        .collect::<Vec<_>>();
    let mut result = SidecarExportSummary {
        requested: ids.len(),
        written: 0,
        preserved_existing: 0,
        failed: Vec::new(),
    };
    for id in ids {
        let asset = match read_asset(connection, id) {
            Ok(asset) => asset,
            Err(error) => {
                result.failed.push(format!("{id}: {error}"));
                continue;
            }
        };
        if !asset.original_path.is_file() {
            result.failed.push(format!(
                "{}: original representation is unavailable",
                asset.filename
            ));
            continue;
        }
        let destination = sidecar_path(&asset.original_path);
        if destination.exists() && !replace {
            result.preserved_existing += 1;
            continue;
        }
        let contents = xmp_for(&asset);
        match staged_write(&destination, &contents, replace) {
            Ok(()) => {
                let content_hash = hash_file(&destination)?;
                connection.execute("INSERT INTO sidecar_exports(asset_id,path,content_hash,exported_at)VALUES(?1,?2,?3,?4) ON CONFLICT(asset_id) DO UPDATE SET path=excluded.path,content_hash=excluded.content_hash,exported_at=excluded.exported_at", params![asset.id,destination.to_string_lossy(),content_hash,Utc::now().to_rfc3339()])?;
                result.written += 1;
            }
            Err(error) => result.failed.push(format!("{}: {error}", asset.filename)),
        }
    }
    Ok(result)
}

fn local_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name)
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn parse_coordinate(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_xmp(bytes: &[u8]) -> Result<SidecarMetadata> {
    validate_xml(bytes).map_err(|error| {
        KeepframeError::Message(format!("The XMP sidecar is malformed: {error}"))
    })?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut metadata = SidecarMetadata::default();
    let mut in_subject = false;
    let mut in_hierarchy = false;
    let mut current = String::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
                let raw_name = String::from_utf8_lossy(event.name().as_ref()).to_ascii_lowercase();
                let name = local_name(event.name().as_ref());
                if raw_name.ends_with("dc:subject") {
                    in_subject = true;
                }
                if raw_name.ends_with("hierarchicalsubject") {
                    in_hierarchy = true;
                }
                for attribute in event.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|error| {
                        KeepframeError::Message(format!("Malformed XMP attribute: {error}"))
                    })?;
                    let key = String::from_utf8_lossy(attribute.key.as_ref()).to_ascii_lowercase();
                    let value = attribute
                        .unescape_value()
                        .map_err(|error| {
                            KeepframeError::Message(format!("Malformed XMP value: {error}"))
                        })?
                        .into_owned();
                    if key.ends_with("triage") {
                        metadata.decision = Some(value.clone());
                    }
                    if key.ends_with("locationsource") {
                        metadata.location_source = Some(value.clone());
                    }
                    if key.ends_with("gpslatitude") {
                        metadata.latitude = parse_coordinate(&value);
                    }
                    if key.ends_with("gpslongitude") {
                        metadata.longitude = parse_coordinate(&value);
                    }
                    if key.ends_with("label") && metadata.decision.is_none() {
                        metadata.decision = match value.to_ascii_lowercase().as_str() {
                            "keep" => Some("keep".into()),
                            "discard" => Some("discard".into()),
                            "undecided" => Some("undecided".into()),
                            _ => None,
                        };
                    }
                }
                current = name;
            }
            Ok(Event::Text(event)) => {
                let value = event
                    .unescape()
                    .map_err(|error| {
                        KeepframeError::Message(format!("Malformed XMP text: {error}"))
                    })?
                    .into_owned();
                if in_hierarchy && current == "li" {
                    metadata.hierarchy.push(value);
                } else if in_subject && current == "li" {
                    metadata.tags.push(value);
                } else if current == "gpslatitude" {
                    metadata.latitude = parse_coordinate(&value);
                } else if current == "gpslongitude" {
                    metadata.longitude = parse_coordinate(&value);
                }
            }
            Ok(Event::End(event)) => {
                let raw_name = String::from_utf8_lossy(event.name().as_ref()).to_ascii_lowercase();
                if raw_name.ends_with("dc:subject") {
                    in_subject = false;
                }
                if raw_name.ends_with("hierarchicalsubject") {
                    in_hierarchy = false;
                }
                current.clear();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(KeepframeError::Message(format!(
                    "The XMP sidecar is malformed: {error}"
                )))
            }
        }
        buffer.clear();
    }
    Ok(metadata)
}

pub(crate) fn import_sidecar(
    connection: &mut Connection,
    asset_id: &str,
) -> Result<SidecarImportResult> {
    let asset = read_asset(connection, asset_id)?;
    let path = sidecar_path(&asset.original_path);
    let metadata = parse_xmp(&fs::read(&path).map_err(|_| {
        KeepframeError::Message("No readable XMP sidecar exists beside this original.".into())
    })?)?;
    let mut result = SidecarImportResult {
        tags_imported: false,
        location_imported: false,
        triage_imported: false,
        conflicts: Vec::new(),
    };
    let old_tags = asset.tags;
    let tags = normalise_tags(
        metadata
            .tags
            .into_iter()
            .chain(metadata.hierarchy)
            .collect(),
    );
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if !tags.is_empty() {
        if old_tags.is_empty() {
            super::replace_asset_tags(&tx, asset_id, &tags)?;
            result.tags_imported = true;
        } else if old_tags != tags {
            result
                .conflicts
                .push("Catalogue tags already exist; sidecar tags were not applied.".into());
        }
    }
    if let (Some(latitude), Some(longitude)) = (metadata.latitude, metadata.longitude) {
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            result
                .conflicts
                .push("Sidecar coordinates are outside the valid range.".into());
        } else if asset.latitude.is_none() {
            let manual = metadata.location_source.as_deref() == Some("manual");
            if manual {
                tx.execute("UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=?2,manual_longitude=?3,location_source='manual' WHERE id=?1", params![asset_id,latitude,longitude])?;
            } else {
                tx.execute("UPDATE assets SET latitude=?2,longitude=?3,embedded_latitude=?2,embedded_longitude=?3,location_source='embedded' WHERE id=?1", params![asset_id,latitude,longitude])?;
            }
            result.location_imported = true;
        } else if asset.latitude != Some(latitude) || asset.longitude != Some(longitude) {
            result.conflicts.push(
                "Catalogue location already exists; sidecar location was not applied.".into(),
            );
        }
    }
    if let Some(decision) = metadata
        .decision
        .filter(|value| matches!(value.as_str(), "keep" | "undecided" | "discard"))
    {
        if asset.decision == "undecided" && decision != "undecided" {
            tx.execute(
                "UPDATE assets SET decision=?2 WHERE id=?1",
                params![asset_id, decision],
            )?;
            result.triage_imported = true;
        } else if asset.decision != decision {
            result.conflicts.push(
                "Catalogue triage state already exists; sidecar triage was not applied.".into(),
            );
        }
    }
    if result.tags_imported || result.location_imported || result.triage_imported {
        tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'xmp_import',?2,?3,?4)", params![asset_id,json!({"tags":old_tags,"latitude":asset.latitude,"longitude":asset.longitude,"decision":asset.decision}).to_string(),json!({"tagsImported":result.tags_imported,"locationImported":result.location_imported,"triageImported":result.triage_imported}).to_string(),Utc::now().to_rfc3339()])?;
    }
    tx.commit()?;
    Ok(result)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableVersion {
    id: String,
    kind: String,
    provider: Option<String>,
    state: String,
    created_at: String,
    source_hash: Option<String>,
    output_hash: Option<String>,
    relative_path: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableAsset {
    id: String,
    source_id: String,
    is_primary: bool,
    version_name: Option<String>,
    version_index: i64,
    filename: String,
    decision: String,
    rating: i64,
    title: Option<String>,
    caption: Option<String>,
    copyright: Option<String>,
    creator: Option<String>,
    captured_at: String,
    date_fallback: bool,
    camera: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    tags: Vec<String>,
    location: serde_json::Value,
    original: serde_json::Value,
    develop_recipe: Option<serde_json::Value>,
    derived_versions: Vec<PortableVersion>,
    ai_derivative: Option<PortableAiDerivative>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PortableAiDerivative {
    id: String,
    parent_source_id: String,
    root_source_id: String,
    derived_source_id: String,
    source_asset_id: String,
    operation: String,
    provider: String,
    provider_version: String,
    model_id: String,
    model_revision: String,
    model_sha256: String,
    execution_provider: String,
    parameters: Value,
    scale: i64,
    tile_size: i64,
    overlap: i64,
    source_sha256: String,
    output_sha256: String,
    output_width: i64,
    output_height: i64,
    pixel_format: String,
    bit_depth: i64,
    managed_relative_path: String,
    provenance_version: i64,
    created_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableCatalogue {
    format: String,
    schema_version: i64,
    assets: Vec<PortableAsset>,
    organisation: Value,
}

fn relative_or_absolute(root: &Path, path: &str) -> Option<String> {
    Path::new(path)
        .strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn rebase_managed_path(root: &Path, path: &Path, anchors: &[&str]) -> Option<PathBuf> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let index = components.iter().position(|component| {
        anchors
            .iter()
            .any(|anchor| component.to_string_lossy().eq_ignore_ascii_case(anchor))
    })?;
    let candidate = components[index..]
        .iter()
        .fold(root.to_path_buf(), |joined, component| {
            joined.join(component)
        });
    candidate.is_file().then_some(candidate)
}

/// Rebase only paths that can be verified inside a moved managed library.  This
/// never searches arbitrary folders or accepts a non-matching hash.
pub(crate) fn recover_moved_managed_paths(
    connection: &mut Connection,
    root: &Path,
) -> Result<usize> {
    let mut updates = 0usize;
    let representations = connection
        .prepare("SELECT id,path,sha256 FROM representations")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for (id, path, hash) in representations {
        if Path::new(&path).is_file() {
            continue;
        }
        let Some(candidate) = rebase_managed_path(
            root,
            Path::new(&path),
            &["Originals", "Edits", "derivatives"],
        ) else {
            continue;
        };
        if hash_file(&candidate)? == hash {
            tx.execute(
                "UPDATE representations SET path=?2 WHERE id=?1",
                params![id, candidate.to_string_lossy()],
            )?;
            updates += 1;
        }
    }
    let versions = tx
        .prepare("SELECT id,path,output_hash FROM versions WHERE output_hash IS NOT NULL")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, path, hash) in versions {
        if Path::new(&path).is_file() {
            continue;
        }
        let Some(candidate) = rebase_managed_path(root, Path::new(&path), &["Edits"]) else {
            continue;
        };
        if hash_file(&candidate)? == hash {
            tx.execute(
                "UPDATE versions SET path=?2 WHERE id=?1",
                params![id, candidate.to_string_lossy()],
            )?;
            updates += 1;
        }
    }
    let thumbnails = tx
        .prepare("SELECT id,thumbnail_path FROM assets")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, path) in thumbnails {
        if !Path::new(&path).is_file() {
            if let Some(candidate) = rebase_managed_path(root, Path::new(&path), &[".keepframe"]) {
                tx.execute(
                    "UPDATE assets SET thumbnail_path=?2 WHERE id=?1",
                    params![id, candidate.to_string_lossy()],
                )?;
                updates += 1;
            }
        }
    }
    tx.commit()?;
    Ok(updates)
}

fn portable_organisation(connection: &Connection) -> Result<Value> {
    let sets=connection.prepare("SELECT id,name,position FROM collection_sets ORDER BY position,id")?.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"position":row.get::<_,i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let collections=connection.prepare("SELECT id,name,kind,set_id,match_mode,rules_json,position FROM collections ORDER BY position,id")?.query_map([],|row|{let rules:String=row.get(5)?;Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"kind":row.get::<_,String>(2)?,"setId":row.get::<_,Option<String>>(3)?,"matchMode":row.get::<_,String>(4)?,"rules":serde_json::from_str::<Value>(&rules).unwrap_or(Value::Null),"position":row.get::<_,i64>(6)?}))})?.collect::<rusqlite::Result<Vec<_>>>()?;
    let memberships=connection.prepare("SELECT collection_id,item_id,position,added_at FROM collection_items ORDER BY collection_id,position,item_id")?.query_map([],|row|Ok(json!({"collectionId":row.get::<_,String>(0)?,"itemId":row.get::<_,String>(1)?,"position":row.get::<_,i64>(2)?,"addedAt":row.get::<_,String>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let stacks=connection.prepare("SELECT id,name,collapsed,top_item_id,created_at,updated_at FROM stacks ORDER BY created_at,id")?.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,Option<String>>(1)?,"collapsed":row.get::<_,i64>(2)?!=0,"topItemId":row.get::<_,String>(3)?,"createdAt":row.get::<_,String>(4)?,"updatedAt":row.get::<_,String>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let stack_items=connection.prepare("SELECT stack_id,item_id,position FROM stack_items ORDER BY stack_id,position,item_id")?.query_map([],|row|Ok(json!({"stackId":row.get::<_,String>(0)?,"itemId":row.get::<_,String>(1)?,"position":row.get::<_,i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let source_groups=connection.prepare("SELECT id,versions_collapsed FROM sources ORDER BY id")?.query_map([],|row|Ok(json!({"sourceId":row.get::<_,String>(0)?,"versionsCollapsed":row.get::<_,i64>(1)?!=0})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    // Only the explicit user decision is portable. Embeddings, queues and
    // scores remain excluded rebuildable data.
    let semantic_decisions=connection.prepare("SELECT identity_hash,kind,decision,model_id,model_revision,member_source_ids_json,decided_at FROM semantic_suggestion_decisions ORDER BY decided_at,identity_hash")?.query_map([],|row|{let members:String=row.get(5)?;Ok(json!({"identityHash":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"decision":row.get::<_,String>(2)?,"modelId":row.get::<_,String>(3)?,"modelRevision":row.get::<_,String>(4)?,"memberSourceIds":serde_json::from_str::<Value>(&members).unwrap_or(Value::Null),"decidedAt":row.get::<_,String>(6)?}))})?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"collectionSets":sets,"collections":collections,"memberships":memberships,"stacks":stacks,"stackItems":stack_items,"sourceGroups":source_groups,"semanticSuggestionDecisions":semantic_decisions}),
    )
}

pub(crate) fn export_portable_catalogue(
    connection: &Connection,
    root: &Path,
    destination: &Path,
) -> Result<PathBuf> {
    if !destination.is_dir() {
        return Err(KeepframeError::Message(
            "Choose an existing folder for the portable catalogue export.".into(),
        ));
    }
    let output = destination.join("keepframe-portable-catalogue-v5.json");
    let temporary = output.with_extension("json.partial");
    let mut tag_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut tags = connection.prepare("SELECT at.asset_id,t.name FROM asset_tags at JOIN tags t ON t.id=at.tag_id ORDER BY at.asset_id,t.name COLLATE NOCASE")?;
    for row in tags.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (asset, tag) = row?;
        tag_map.entry(asset).or_default().push(tag);
    }
    let mut version_map: HashMap<String, Vec<PortableVersion>> = HashMap::new();
    let mut versions = connection.prepare("SELECT asset_id,id,kind,provider,state,created_at,source_hash,output_hash,path FROM versions ORDER BY asset_id,created_at,id")?;
    for row in versions.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            PortableVersion {
                id: row.get(1)?,
                kind: row.get(2)?,
                provider: row.get(3)?,
                state: row.get(4)?,
                created_at: row.get(5)?,
                source_hash: row.get(6)?,
                output_hash: row.get(7)?,
                relative_path: relative_or_absolute(root, &row.get::<_, String>(8)?),
            },
        ))
    })? {
        let (asset, version) = row?;
        version_map.entry(asset).or_default().push(version);
    }
    let mut recipe_map = HashMap::new();
    let mut recipes =
        connection.prepare("SELECT asset_id,recipe_json FROM develop_recipes ORDER BY asset_id")?;
    for row in recipes.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (asset, recipe) = row?;
        let value = serde_json::from_str::<DevelopRecipe>(&recipe)
            .map_err(|error| {
                KeepframeError::Message(format!(
                    "Portable catalogue rejected an invalid Develop recipe for {asset}: {error}"
                ))
            })?
            .validate()
            .map_err(|error| {
                KeepframeError::Message(format!(
                    "Portable catalogue rejected an invalid Develop recipe for {asset}: {error}"
                ))
            })?;
        recipe_map.insert(asset, serde_json::to_value(value)?);
    }
    let mut derivative_map = HashMap::new();
    let mut derivatives = connection.prepare("SELECT id,parent_source_id,root_source_id,derived_source_id,source_asset_id,operation,provider,provider_version,model_id,model_revision,model_sha256,execution_provider,parameters_json,scale,tile_size,overlap,source_sha256,output_sha256,output_width,output_height,pixel_format,bit_depth,managed_relative_path,provenance_version,created_at FROM ai_derivatives ORDER BY created_at,id")?;
    for row in derivatives.query_map([], |row| {
        let parameters: String = row.get(12)?;
        Ok(PortableAiDerivative {
            id: row.get(0)?,
            parent_source_id: row.get(1)?,
            root_source_id: row.get(2)?,
            derived_source_id: row.get(3)?,
            source_asset_id: row.get(4)?,
            operation: row.get(5)?,
            provider: row.get(6)?,
            provider_version: row.get(7)?,
            model_id: row.get(8)?,
            model_revision: row.get(9)?,
            model_sha256: row.get(10)?,
            execution_provider: row.get(11)?,
            parameters: serde_json::from_str(&parameters).unwrap_or(Value::Null),
            scale: row.get(13)?,
            tile_size: row.get(14)?,
            overlap: row.get(15)?,
            source_sha256: row.get(16)?,
            output_sha256: row.get(17)?,
            output_width: row.get(18)?,
            output_height: row.get(19)?,
            pixel_format: row.get(20)?,
            bit_depth: row.get(21)?,
            managed_relative_path: row.get(22)?,
            provenance_version: row.get(23)?,
            created_at: row.get(24)?,
        })
    })? {
        let derivative = row?;
        derivative_map.insert(derivative.derived_source_id.clone(), derivative);
    }
    let file = fs::File::create(&temporary)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(format!("{{\"format\":\"keepframe-portable-catalogue\",\"schemaVersion\":{PORTABLE_CATALOGUE_SCHEMA_VERSION},\"exportedAt\":\"{}\",\"assets\":[", Utc::now().to_rfc3339()).as_bytes())?;
    let mut assets = connection.prepare("SELECT a.id,s.filename,a.decision,s.captured_at,s.date_fallback,s.camera,s.width,s.height,s.latitude,s.longitude,s.embedded_latitude,s.embedded_longitude,s.manual_latitude,s.manual_longitude,COALESCE(s.location_source,'none'),r.path,r.sha256,r.byte_size,r.extension,r.is_raw,a.source_id,a.is_primary,a.version_name,a.version_index,a.rating,a.title,a.caption,a.copyright,a.creator FROM assets a JOIN sources s ON s.id=a.source_id LEFT JOIN representations r ON r.id=(SELECT r2.id FROM representations r2 WHERE r2.source_id=a.source_id ORDER BY r2.is_raw ASC,r2.path ASC LIMIT 1) ORDER BY s.captured_at DESC,a.source_id,a.version_index,a.id")?;
    let mut first = true;
    for row in assets.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<f64>>(8)?,
            row.get::<_, Option<f64>>(9)?,
            row.get::<_, Option<f64>>(10)?,
            row.get::<_, Option<f64>>(11)?,
            row.get::<_, Option<f64>>(12)?,
            row.get::<_, Option<f64>>(13)?,
            row.get::<_, String>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, Option<String>>(16)?,
            row.get::<_, Option<i64>>(17)?,
            row.get::<_, Option<String>>(18)?,
            row.get::<_, Option<i64>>(19)?,
            row.get::<_, String>(20)?,
            row.get::<_, i64>(21)?,
            row.get::<_, Option<String>>(22)?,
            row.get::<_, i64>(23)?,
            row.get::<_, i64>(24)?,
            row.get::<_, Option<String>>(25)?,
            row.get::<_, Option<String>>(26)?,
            row.get::<_, Option<String>>(27)?,
            row.get::<_, Option<String>>(28)?,
        ))
    })? {
        let (
            id,
            filename,
            decision,
            captured_at,
            date_fallback,
            camera,
            width,
            height,
            latitude,
            longitude,
            embedded_latitude,
            embedded_longitude,
            manual_latitude,
            manual_longitude,
            location_source,
            path,
            sha256,
            byte_size,
            extension,
            is_raw,
            source_id,
            is_primary,
            version_name,
            version_index,
            rating,
            title,
            caption,
            copyright,
            creator,
        ) = row?;
        let ai_derivative = derivative_map.remove(&source_id);
        let record = PortableAsset {
            id: id.clone(),
            source_id,
            is_primary: is_primary != 0,
            version_name,
            version_index,
            filename,
            decision,
            rating,
            title,
            caption,
            copyright,
            creator,
            captured_at,
            date_fallback: date_fallback != 0,
            camera,
            width,
            height,
            tags: tag_map.remove(&id).unwrap_or_default(),
            location: json!({"latitude":latitude,"longitude":longitude,"source":location_source,"embeddedLatitude":embedded_latitude,"embeddedLongitude":embedded_longitude,"manualLatitude":manual_latitude,"manualLongitude":manual_longitude}),
            original: json!({"path":path,"managedRelativePath":path.as_deref().and_then(|path| relative_or_absolute(root,path)),"sha256":sha256,"byteSize":byte_size,"extension":extension,"isRaw":is_raw.map(|value| value != 0)}),
            develop_recipe: recipe_map.remove(&id),
            derived_versions: version_map.remove(&id).unwrap_or_default(),
            ai_derivative,
        };
        if !first {
            writer.write_all(b",")?;
        }
        first = false;
        serde_json::to_writer(&mut writer, &record)?;
    }
    writer.write_all(b"],\"organisation\":")?;
    serde_json::to_writer(&mut writer, &portable_organisation(connection)?)?;
    writer.write_all(b"}")?;
    writer.flush()?;
    drop(writer);
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    serde_json::from_reader::<_, serde_json::Value>(fs::File::open(&temporary)?).map_err(
        |error| KeepframeError::Message(format!("Portable catalogue validation failed: {error}")),
    )?;
    if output.exists() {
        let _ = fs::remove_file(&output);
    }
    fs::rename(&temporary, &output)?;
    Ok(output)
}

fn portable_text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(|| {
        KeepframeError::Message(format!("Portable catalogue organisation is missing {key}."))
    })
}
fn portable_array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value.get(key).and_then(Value::as_array).ok_or_else(|| {
        KeepframeError::Message(format!("Portable catalogue organisation is missing {key}."))
    })
}

/// Restore catalogue-native organisation onto the same physical library. A
/// verified backup is created by the command wrapper before this is called;
/// no source or derived image file is written or removed here.
pub(crate) fn import_portable_catalogue(connection: &mut Connection, path: &Path) -> Result<usize> {
    if !path.is_file() || fs::metadata(path)?.len() > 64 * 1024 * 1024 {
        return Err(KeepframeError::Message(
            "Choose a portable catalogue JSON file no larger than 64 MiB.".into(),
        ));
    }
    let portable: PortableCatalogue = serde_json::from_reader(fs::File::open(path)?)?;
    if portable.format != "keepframe-portable-catalogue"
        || portable.schema_version != PORTABLE_CATALOGUE_SCHEMA_VERSION
    {
        return Err(KeepframeError::Message(format!(
            "This importer requires portable catalogue schema {PORTABLE_CATALOGUE_SCHEMA_VERSION}."
        )));
    }
    let organisation = portable.organisation;
    let sets = portable_array(&organisation, "collectionSets")?;
    let collections = portable_array(&organisation, "collections")?;
    let memberships = portable_array(&organisation, "memberships")?;
    let stacks = portable_array(&organisation, "stacks")?;
    let stack_items = portable_array(&organisation, "stackItems")?;
    let source_groups = portable_array(&organisation, "sourceGroups")?;
    let semantic_decisions = portable_array(&organisation, "semanticSuggestionDecisions")?;
    for collection in collections {
        if portable_text(collection, "kind")? == "smart" {
            let rules = serde_json::from_value::<Vec<super::organisation::SmartRule>>(
                collection.get("rules").cloned().ok_or_else(|| {
                    KeepframeError::Message("Smart Collection rules are missing.".into())
                })?,
            )?;
            super::organisation::compile_rules(&rules, portable_text(collection, "matchMode")?)?;
        }
    }
    for value in semantic_decisions {
        let kind = portable_text(value, "kind")?;
        let decision = portable_text(value, "decision")?;
        if !matches!(
            kind,
            "exact_duplicate" | "near_duplicate" | "burst" | "similar_series"
        ) || !matches!(decision, "dismissed" | "accepted")
            || value
                .get("memberSourceIds")
                .and_then(Value::as_array)
                .is_none()
        {
            return Err(KeepframeError::Message(
                "Portable semantic suggestion decision is invalid.".into(),
            ));
        }
    }
    let imported_ids = portable
        .assets
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    for item in &portable.assets {
        let source_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1)",
            [&item.source_id],
            |row| row.get(0),
        )?;
        if !source_exists {
            return Err(KeepframeError::Message(format!("Source {} is not present in this physical library; organisation import was not started.",item.source_id)));
        }
        if item.is_primary {
            let matches: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assets WHERE id=?1 AND source_id=?2 AND is_primary=1)",
                params![item.id, item.source_id],
                |row| row.get(0),
            )?;
            if !matches {
                return Err(KeepframeError::Message(format!(
                    "Primary item {} does not match this library.",
                    item.id
                )));
            }
        }
        if let Some(derivative) = &item.ai_derivative {
            let managed = Path::new(&derivative.managed_relative_path);
            let safe_relative = !managed.is_absolute()
                && managed
                    .components()
                    .all(|component| !matches!(component, std::path::Component::ParentDir))
                && derivative
                    .managed_relative_path
                    .replace('\\', "/")
                    .starts_with(".keepframe/derivatives/ai/");
            let lineage_exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1) AND EXISTS(SELECT 1 FROM sources WHERE id=?2) AND EXISTS(SELECT 1 FROM assets WHERE id=?3) AND EXISTS(SELECT 1 FROM representations WHERE source_id=?4 AND sha256=?5)",
                params![derivative.parent_source_id,derivative.root_source_id,derivative.source_asset_id,derivative.derived_source_id,derivative.output_sha256],
                |row| row.get(0),
            )?;
            if derivative.derived_source_id != item.source_id
                || !matches!(
                    derivative.operation.as_str(),
                    "denoise" | "super_resolution"
                )
                || !matches!(derivative.scale, 1 | 2 | 4)
                || derivative.model_sha256.len() != 64
                || derivative.source_sha256.len() != 64
                || derivative.output_sha256.len() != 64
                || derivative.bit_depth != 8
                || derivative.provenance_version != 1
                || !derivative.parameters.is_object()
                || !safe_relative
                || !lineage_exists
            {
                return Err(KeepframeError::Message(format!(
                    "AI derivative metadata for {} does not match this physical library.",
                    item.id
                )));
            }
        }
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch("DELETE FROM collection_items;DELETE FROM collections;DELETE FROM collection_sets;DELETE FROM stack_items;DELETE FROM stacks;DELETE FROM semantic_suggestion_decisions;")?;
    let existing_virtuals = tx
        .prepare("SELECT id FROM assets WHERE is_primary=0")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in existing_virtuals {
        if !imported_ids.contains(&id) {
            tx.execute("DELETE FROM assets WHERE id=?1", [id])?;
        }
    }
    for item in &portable.assets {
        if !item.is_primary {
            tx.execute("INSERT OR IGNORE INTO assets(id,filename,decision,rating,title,caption,copyright,creator,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,created_at,source_id,is_primary,version_name,version_index) SELECT ?1,s.filename,?3,?4,?5,?6,?7,?8,s.captured_at,s.date_fallback,s.camera,s.width,s.height,s.latitude,s.longitude,s.embedded_latitude,s.embedded_longitude,s.manual_latitude,s.manual_longitude,s.location_source,s.missing_state,s.last_verified_at,s.thumbnail_path,?9,s.id,0,?10,?11 FROM sources s WHERE s.id=?2",params![item.id,item.source_id,item.decision,item.rating,item.title,item.caption,item.copyright,item.creator,Utc::now().to_rfc3339(),item.version_name,item.version_index])?;
        }
        tx.execute("UPDATE assets SET decision=?2,rating=?3,title=?4,caption=?5,copyright=?6,creator=?7,version_name=?8,version_index=?9 WHERE id=?1 AND source_id=?10",params![item.id,item.decision,item.rating,item.title,item.caption,item.copyright,item.creator,item.version_name,item.version_index,item.source_id])?;
        if let Some(value) = &item.develop_recipe {
            let recipe = serde_json::from_value::<DevelopRecipe>(value.clone())?.validate()?;
            tx.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES(?1,?2,?3,?4) ON CONFLICT(asset_id) DO UPDATE SET schema_version=excluded.schema_version,recipe_json=excluded.recipe_json,updated_at=excluded.updated_at",params![item.id,recipe.schema_version,serde_json::to_string(&recipe)?,Utc::now().to_rfc3339()])?;
        } else {
            tx.execute("DELETE FROM develop_recipes WHERE asset_id=?1", [&item.id])?;
        }
        super::replace_asset_tags(&tx, &item.id, &item.tags)?;
    }
    for derivative in portable
        .assets
        .iter()
        .filter_map(|item| item.ai_derivative.as_ref())
    {
        tx.execute(
            "DELETE FROM ai_derivatives WHERE derived_source_id=?1",
            [&derivative.derived_source_id],
        )?;
        tx.execute(
            "INSERT INTO ai_derivatives(id,parent_source_id,root_source_id,derived_source_id,source_asset_id,operation,provider,provider_version,model_id,model_revision,model_sha256,execution_provider,parameters_json,scale,tile_size,overlap,source_sha256,output_sha256,output_width,output_height,pixel_format,bit_depth,managed_relative_path,provenance_version,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            params![derivative.id,derivative.parent_source_id,derivative.root_source_id,derivative.derived_source_id,derivative.source_asset_id,derivative.operation,derivative.provider,derivative.provider_version,derivative.model_id,derivative.model_revision,derivative.model_sha256,derivative.execution_provider,serde_json::to_string(&derivative.parameters)?,derivative.scale,derivative.tile_size,derivative.overlap,derivative.source_sha256,derivative.output_sha256,derivative.output_width,derivative.output_height,derivative.pixel_format,derivative.bit_depth,derivative.managed_relative_path,derivative.provenance_version,derivative.created_at],
        )?;
        tx.execute(
            "DELETE FROM semantic_index_queue WHERE source_id=?1",
            [&derivative.derived_source_id],
        )?;
    }
    for value in sets {
        tx.execute("INSERT INTO collection_sets(id,name,position,created_at,updated_at)VALUES(?1,?2,?3,?4,?4)",params![portable_text(value,"id")?,portable_text(value,"name")?,value.get("position").and_then(Value::as_i64).unwrap_or(0),Utc::now().to_rfc3339()])?;
    }
    for value in collections {
        let rules = value.get("rules").cloned().unwrap_or_else(|| json!([]));
        tx.execute("INSERT INTO collections(id,name,kind,set_id,match_mode,rules_json,position,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![portable_text(value,"id")?,portable_text(value,"name")?,portable_text(value,"kind")?,value.get("setId").and_then(Value::as_str),portable_text(value,"matchMode")?,serde_json::to_string(&rules)?,value.get("position").and_then(Value::as_i64).unwrap_or(0),Utc::now().to_rfc3339()])?;
    }
    for value in memberships {
        tx.execute("INSERT INTO collection_items(collection_id,item_id,position,added_at)VALUES(?1,?2,?3,?4)",params![portable_text(value,"collectionId")?,portable_text(value,"itemId")?,value.get("position").and_then(Value::as_i64).unwrap_or(0),value.get("addedAt").and_then(Value::as_str).unwrap_or("portable-import")])?;
    }
    for value in stacks {
        tx.execute("INSERT INTO stacks(id,name,collapsed,top_item_id,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6)",params![portable_text(value,"id")?,value.get("name").and_then(Value::as_str),value.get("collapsed").and_then(Value::as_bool).unwrap_or(true),portable_text(value,"topItemId")?,value.get("createdAt").and_then(Value::as_str).unwrap_or("portable-import"),value.get("updatedAt").and_then(Value::as_str).unwrap_or("portable-import")])?;
    }
    for value in stack_items {
        tx.execute(
            "INSERT INTO stack_items(stack_id,item_id,position)VALUES(?1,?2,?3)",
            params![
                portable_text(value, "stackId")?,
                portable_text(value, "itemId")?,
                value.get("position").and_then(Value::as_i64).unwrap_or(0)
            ],
        )?;
    }
    for value in source_groups {
        tx.execute(
            "UPDATE sources SET versions_collapsed=?2 WHERE id=?1",
            params![
                portable_text(value, "sourceId")?,
                value
                    .get("versionsCollapsed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ],
        )?;
    }
    for value in semantic_decisions {
        tx.execute(
            "INSERT INTO semantic_suggestion_decisions(identity_hash,kind,decision,model_id,model_revision,member_source_ids_json,decided_at)VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                portable_text(value, "identityHash")?,
                portable_text(value, "kind")?,
                portable_text(value, "decision")?,
                portable_text(value, "modelId")?,
                portable_text(value, "modelRevision")?,
                serde_json::to_string(value.get("memberSourceIds").ok_or_else(|| KeepframeError::Message("Portable semantic suggestion members are missing.".into()))?)?,
                portable_text(value, "decidedAt")?
            ],
        )?;
    }
    tx.commit()?;
    Ok(portable.assets.len())
}

pub(crate) fn rescan_library(connection: &mut Connection, root: &Path) -> Result<IntegrityReport> {
    let mut report = IntegrityReport {
        scanned_assets: 0,
        missing_originals: 0,
        missing_derived_versions: 0,
        modified_originals: 0,
        missing_ai_derivatives: 0,
        modified_ai_derivatives: 0,
        orphan_ai_derivatives: 0,
        unsupported_ai_provenance: 0,
        untracked_managed_files: 0,
        sidecar_conflicts: 0,
        findings: Vec::new(),
    };
    let mut known_paths = HashSet::new();
    let mut asset_rows = connection
        .prepare("SELECT s.id,s.filename,a.id FROM sources s JOIN assets a ON a.source_id=s.id AND a.is_primary=1 WHERE s.trashed_at IS NULL ORDER BY s.id")?;
    let assets = asset_rows
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(asset_rows);
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for (source_id, filename, primary_item_id) in assets {
        report.scanned_assets += 1;
        let mut state = "available";
        let mut has_representation = false;
        let derivative: Option<(String, String, String, String)> = tx.query_row(
            "SELECT parent_source_id,root_source_id,model_id,model_revision FROM ai_derivatives WHERE derived_source_id=?1",
            [&source_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        if let Some((parent, root_source, model, revision)) = derivative.as_ref() {
            let parent_exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1 AND trashed_at IS NULL)",
                [parent],
                |row| row.get(0),
            )?;
            if !parent_exists {
                report.orphan_ai_derivatives += 1;
                report.findings.push(IntegrityFinding {
                    asset_id: Some(primary_item_id.clone()),
                    filename: Some(filename.clone()),
                    kind: "ai_derivative_parent_missing".into(),
                    detail: format!("Parent {parent}; root {root_source}"),
                });
            }
            let supported = matches!(
                (model.as_str(), revision.as_str()),
                (
                    "scunet_color_real_psnr",
                    "52e440a80a655b01e0b41e9dd9bfe599bc11625e"
                ) | (
                    "RealESRGAN_x4plus",
                    "a4abfb2979a7bbff3f69f58f58ae324608821e27"
                )
            );
            if !supported {
                report.unsupported_ai_provenance += 1;
                report.findings.push(IntegrityFinding {
                    asset_id: Some(primary_item_id.clone()),
                    filename: Some(filename.clone()),
                    kind: "ai_derivative_provenance_unsupported".into(),
                    detail: format!("{model}@{revision}"),
                });
            }
        }
        let mut representations =
            tx.prepare("SELECT path,sha256,byte_size FROM representations WHERE source_id=?1")?;
        for row in representations.query_map([&source_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            has_representation = true;
            let (path, hash, byte_size) = row?;
            known_paths.insert(PathBuf::from(&path));
            let candidate = PathBuf::from(&path);
            if !candidate.is_file() {
                state = if derivative.is_some() {
                    "derived_missing"
                } else {
                    "original_missing"
                };
                if derivative.is_some() {
                    report.missing_ai_derivatives += 1;
                } else {
                    report.missing_originals += 1;
                }
                report.findings.push(IntegrityFinding {
                    asset_id: Some(primary_item_id.clone()),
                    filename: Some(filename.clone()),
                    kind: if derivative.is_some() {
                        "ai_derivative_missing".into()
                    } else {
                        "original_missing".into()
                    },
                    detail: path,
                });
            } else if fs::metadata(&candidate)?.len() as i64 != byte_size
                || hash_file(&candidate)? != hash
            {
                state = "modified";
                if derivative.is_some() {
                    report.modified_ai_derivatives += 1;
                } else {
                    report.modified_originals += 1;
                }
                report.findings.push(IntegrityFinding {
                    asset_id: Some(primary_item_id.clone()),
                    filename: Some(filename.clone()),
                    kind: if derivative.is_some() {
                        "ai_derivative_modified".into()
                    } else {
                        "original_modified".into()
                    },
                    detail: candidate.to_string_lossy().into(),
                });
            }
        }
        if !has_representation {
            state = "orphaned";
            report.findings.push(IntegrityFinding {
                asset_id: Some(primary_item_id.clone()),
                filename: Some(filename.clone()),
                kind: "orphaned_catalogue_record".into(),
                detail: "No original representation is registered for this catalogue item.".into(),
            });
        }
        let mut versions = tx.prepare(
            "SELECT v.path FROM versions v JOIN assets a ON a.id=v.asset_id WHERE a.source_id=?1",
        )?;
        for row in versions.query_map([&source_id], |row| row.get::<_, String>(0))? {
            let path = row?;
            known_paths.insert(PathBuf::from(&path));
            if !Path::new(&path).is_file() {
                if state == "available" {
                    state = "derived_missing";
                }
                report.missing_derived_versions += 1;
                report.findings.push(IntegrityFinding {
                    asset_id: Some(primary_item_id.clone()),
                    filename: Some(filename.clone()),
                    kind: "derived_missing".into(),
                    detail: path,
                });
            }
        }
        tx.execute(
            "UPDATE sources SET missing_state=?2,last_verified_at=?3 WHERE id=?1",
            params![source_id, state, Utc::now().to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE assets SET missing_state=?2,last_verified_at=?3 WHERE source_id=?1",
            params![source_id, state, Utc::now().to_rfc3339()],
        )?;
    }
    let mut sidecars = tx.prepare("SELECT x.asset_id,s.filename,x.path,x.content_hash FROM sidecar_exports x JOIN assets a ON a.id=x.asset_id JOIN sources s ON s.id=a.source_id")?;
    for row in sidecars.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })? {
        let (asset_id, filename, path, hash) = row?;
        if !Path::new(&path).is_file() || hash_file(Path::new(&path)).ok().as_deref() != Some(&hash)
        {
            report.sidecar_conflicts += 1;
            report.findings.push(IntegrityFinding {
                asset_id: Some(asset_id),
                filename: Some(filename),
                kind: "sidecar_conflict".into(),
                detail: path,
            });
        }
    }
    for directory in [
        root.join("Originals"),
        root.join("Edits"),
        root.join(".keepframe").join("derivatives").join("ai"),
    ] {
        if directory.is_dir() {
            for entry in walkdir::WalkDir::new(directory)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_type().is_file())
            {
                let path = entry.path().to_path_buf();
                if !known_paths.contains(&path) {
                    report.untracked_managed_files += 1;
                    report.findings.push(IntegrityFinding {
                        asset_id: None,
                        filename: path.file_name().map(|name| name.to_string_lossy().into()),
                        kind: "untracked_managed_file".into(),
                        detail: path.to_string_lossy().into(),
                    });
                }
            }
        }
    }
    drop(sidecars);
    tx.commit()?;
    Ok(report)
}

pub(crate) fn relink_candidates(
    connection: &Connection,
    asset_id: &str,
    directory: &Path,
) -> Result<Vec<RelinkCandidate>> {
    if !directory.is_dir() {
        return Err(KeepframeError::Message(
            "Choose an existing folder to search for a confirmed relink.".into(),
        ));
    }
    let mut representations = connection.prepare("SELECT path,sha256,byte_size FROM representations WHERE source_id=(SELECT source_id FROM assets WHERE id=?1) ORDER BY is_raw ASC,path ASC")?;
    let missing = representations
        .query_map([asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(path, _, _)| !Path::new(path).is_file());
    let Some((_missing_path, expected_hash, expected_size)) = missing else {
        return Ok(Vec::new());
    };
    let mut matches = Vec::new();
    for entry in walkdir::WalkDir::new(directory)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let metadata = entry.metadata().map_err(|error| {
            KeepframeError::Message(format!("Could not inspect a relink candidate: {error}"))
        })?;
        if metadata.len() as i64 == expected_size && hash_file(entry.path())? == expected_hash {
            matches.push(RelinkCandidate {
                path: entry.path().to_string_lossy().into(),
                sha256: expected_hash.clone(),
            });
        }
    }
    Ok(matches)
}

pub(crate) fn relink_asset(
    connection: &mut Connection,
    asset_id: &str,
    replacement: &Path,
) -> Result<()> {
    if !replacement.is_file() {
        return Err(KeepframeError::Message(
            "The selected relink target is not a readable file.".into(),
        ));
    }
    let mut representations = connection.prepare("SELECT id,path,sha256,byte_size FROM representations WHERE source_id=(SELECT source_id FROM assets WHERE id=?1) ORDER BY is_raw ASC,path ASC")?;
    let target = representations
        .query_map([asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, path, _, _)| !Path::new(path).is_file())
        .ok_or_else(|| {
            KeepframeError::Message(
                "This asset has no missing original representation to relink.".into(),
            )
        })?;
    drop(representations);
    if fs::metadata(replacement)?.len() as i64 != target.3 || hash_file(replacement)? != target.2 {
        return Err(KeepframeError::Message("Relink was rejected because the chosen file does not exactly match the catalogue hash.".into()));
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE representations SET path=?2 WHERE id=?1",
        params![target.0, replacement.to_string_lossy()],
    )?;
    tx.execute(
        "UPDATE sources SET missing_state='available',last_verified_at=?2 WHERE id=(SELECT source_id FROM assets WHERE id=?1)",
        params![asset_id, Utc::now().to_rfc3339()],
    )?;
    tx.execute("UPDATE assets SET missing_state='available',last_verified_at=?2 WHERE source_id=(SELECT source_id FROM assets WHERE id=?1)",params![asset_id,Utc::now().to_rfc3339()])?;
    tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'relink',?2,?3,?4)", params![asset_id,json!({"path":target.1}).to_string(),json!({"path":replacement}).to_string(),Utc::now().to_rfc3339()])?;
    Ok(tx.commit()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;
    use uuid::Uuid;

    fn fixture() -> (PathBuf, Connection, String, PathBuf) {
        let root = std::env::temp_dir().join(format!("keepframe-m5-test-{}", Uuid::new_v4()));
        super::super::initialise_layout(&root).unwrap();
        let source = root.join("Originals/2026/01/01/photo.jpg");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"protected-original-for-interoperability-tests").unwrap();
        let hash = hash_file(&source).unwrap();
        let thumbnail = root.join(".keepframe/thumbnails/photo.jpg");
        let connection = super::super::open_db(&root).unwrap();
        let asset_id = "asset-m5".to_string();
        connection.execute("INSERT INTO assets(id,filename,decision,captured_at,date_fallback,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,thumbnail_path,created_at)VALUES(?1,'photo.jpg','undecided','2026-01-02T03:04:05Z',0,55.9500,-3.1900,55.9500,-3.1900,NULL,NULL,'embedded',?2,'2026-01-02T03:04:05Z')", params![asset_id,thumbnail.to_string_lossy()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('rep-m5',?1,?2,?3,'jpg','photo',?4,0)", params![asset_id,source.to_string_lossy(),hash,fs::metadata(&source).unwrap().len() as i64]).unwrap();
        (root, connection, asset_id, source)
    }

    #[test]
    fn xmp_export_is_staged_and_preserves_existing_sidecars() {
        let (root, mut connection, asset_id, source) = fixture();
        let first =
            export_sidecars(&mut connection, std::slice::from_ref(&asset_id), false).unwrap();
        assert_eq!(first.written, 1, "{first:?}");
        let sidecar = sidecar_path(&source);
        let content = fs::read_to_string(&sidecar).unwrap();
        assert!(content.contains("<dc:subject>"));
        assert!(content.contains("keepframe:locationSource=\"embedded\""));
        fs::write(&sidecar, b"<?xml version=\"1.0\"?><manual/>").unwrap();
        let preserved = export_sidecars(&mut connection, &[asset_id], false).unwrap();
        assert_eq!(preserved.preserved_existing, 1);
        assert!(fs::read_to_string(&sidecar).unwrap().contains("manual"));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_xmp_is_rejected_without_catalogue_change() {
        let (root, mut connection, asset_id, source) = fixture();
        fs::write(sidecar_path(&source), b"<rdf:RDF><bad>").unwrap();
        assert!(import_sidecar(&mut connection, &asset_id).is_err());
        let state: String = connection
            .query_row(
                "SELECT decision FROM assets WHERE id=?1",
                [&asset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "undecided");
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn conservative_xmp_import_fills_only_empty_catalogue_fields() {
        let (root, mut connection, asset_id, source) = fixture();
        connection.execute("UPDATE assets SET latitude=NULL,longitude=NULL,embedded_latitude=NULL,embedded_longitude=NULL,location_source='none' WHERE id=?1", [&asset_id]).unwrap();
        fs::write(sidecar_path(&source), br#"<?xml version="1.0"?><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:keepframe="https://keepframe.app/ns/1.0/" keepframe:triage="keep" keepframe:locationSource="manual"><dc:subject xmlns:dc="http://purl.org/dc/elements/1.1/"><rdf:Bag><rdf:li>People/Hazel</rdf:li></rdf:Bag></dc:subject><exif:GPSLatitude xmlns:exif="http://ns.adobe.com/exif/1.0/">56.0000</exif:GPSLatitude><exif:GPSLongitude xmlns:exif="http://ns.adobe.com/exif/1.0/">-3.0000</exif:GPSLongitude></rdf:Description></rdf:RDF>"#).unwrap();
        let result = import_sidecar(&mut connection, &asset_id).unwrap();
        assert!(result.tags_imported && result.location_imported && result.triage_imported);
        let state: (String, Option<f64>, String) = connection
            .query_row(
                "SELECT decision,manual_latitude,location_source FROM assets WHERE id=?1",
                [&asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, ("keep".into(), Some(56.0), "manual".into()));
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn virtual_versions_never_claim_the_single_source_xmp_sidecar() {
        let (root, mut connection, primary, source) = fixture();
        let virtual_id = super::super::organisation::create_version_in(
            &mut connection,
            super::super::organisation::CreateVersionRequest {
                item_id: primary,
                mode: "default".into(),
                name: Some("B&W".into()),
            },
        )
        .unwrap();
        let report = export_sidecars(&mut connection, &[virtual_id], true).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.failed.len(), 1);
        assert!(!source.with_extension("xmp").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn portable_catalogue_is_versioned_and_keeps_manual_location_provenance() {
        let (root, connection, asset_id, _source) = fixture();
        connection.execute("UPDATE assets SET latitude=56.0,longitude=-3.0,manual_latitude=56.0,manual_longitude=-3.0,location_source='manual' WHERE id=?1", [&asset_id]).unwrap();
        let coverage =
            image::GrayImage::from_fn(16, 16, |x, _| image::Luma([if x < 8 { 255 } else { 0 }]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(coverage)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        use base64::Engine as _;
        use sha2::Digest as _;
        let payload = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let checksum = format!("{:x}", sha2::Sha256::digest(&bytes));
        let recipe = crate::DevelopRecipe {
            schema_version: 3,
            advanced: crate::advanced_develop::AdvancedDevelopSettings::default(),
            settings: crate::BasicAdjustments::neutral(),
            masks: vec![crate::DevelopMask {
                id: "semantic".into(),
                name: "Sky".into(),
                enabled: true,
                inverted: false,
                opacity: 1.0,
                feather: 0.0,
                geometry: crate::MaskGeometry::Semantic {
                    width: 16,
                    height: 16,
                    coverage_png: payload.clone(),
                    checksum,
                    provenance: Box::new(crate::MaskProvenance {
                        provider: "fixture".into(),
                        provider_version: "1".into(),
                        model: "fixture".into(),
                        model_revision: "1".into(),
                        model_sha256: "a".repeat(64),
                        category: "sky".into(),
                        execution_provider: "mock".into(),
                    }),
                    refinements: Vec::new(),
                },
                adjustments: crate::LocalAdjustments::default(),
            }],
        };
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES(?1,2,?2,'2026-01-02T03:04:05Z')", params![asset_id,serde_json::to_string(&recipe).unwrap()]).unwrap();
        let destination = root.join("portable");
        fs::create_dir_all(&destination).unwrap();
        let output = export_portable_catalogue(&connection, &root, &destination).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], PORTABLE_CATALOGUE_SCHEMA_VERSION);
        assert!(value.get("semanticEmbeddings").is_none());
        assert_eq!(value["assets"][0]["location"]["source"], "manual");
        assert_eq!(
            value["assets"][0]["developRecipe"]["masks"][0]["geometry"]["kind"],
            "semantic"
        );
        assert_eq!(
            value["assets"][0]["developRecipe"]["masks"][0]["geometry"]["coveragePng"],
            payload
        );
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rescan_marks_missing_and_relink_requires_an_exact_hash() {
        let (root, mut connection, asset_id, source) = fixture();
        let sibling = super::super::organisation::create_version_in(
            &mut connection,
            super::super::organisation::CreateVersionRequest {
                item_id: asset_id.clone(),
                mode: "default".into(),
                name: Some("Missing sibling".into()),
            },
        )
        .unwrap();
        let bytes = fs::read(&source).unwrap();
        fs::remove_file(&source).unwrap();
        let report = rescan_library(&mut connection, &root).unwrap();
        assert_eq!(report.missing_originals, 1);
        assert_eq!(
            connection
                .query_row(
                    "SELECT missing_state FROM assets WHERE id=?1",
                    [&sibling],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "original_missing"
        );
        let candidates_root = root.join("relink-candidates");
        fs::create_dir_all(&candidates_root).unwrap();
        fs::write(candidates_root.join("one.jpg"), &bytes).unwrap();
        fs::write(candidates_root.join("two.jpg"), &bytes).unwrap();
        let candidates = relink_candidates(&connection, &asset_id, &candidates_root).unwrap();
        assert_eq!(
            candidates.len(),
            2,
            "ambiguous hash matches are returned for an explicit user choice"
        );
        assert!(relink_asset(
            &mut connection,
            &asset_id,
            &candidates_root.join("not-a-match.jpg")
        )
        .is_err());
        relink_asset(&mut connection, &asset_id, Path::new(&candidates[0].path)).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT missing_state FROM assets WHERE id=?1",
                    [&asset_id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "available"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT missing_state FROM assets WHERE id=?1",
                    [sibling],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "available"
        );
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn moved_managed_library_paths_rebase_only_after_hash_confirmation() {
        let (root, mut connection, _asset_id, source) = fixture();
        let old_path = PathBuf::from(r"Z:\FormerLibrary\Originals\2026\01\01\photo.jpg");
        connection
            .execute(
                "UPDATE representations SET path=?1 WHERE id='rep-m5'",
                [old_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        assert_eq!(
            recover_moved_managed_paths(&mut connection, &root).unwrap(),
            1
        );
        let rebased: String = connection
            .query_row(
                "SELECT path FROM representations WHERE id='rep-m5'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(PathBuf::from(rebased), source);
        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_export_and_rescan_scale_from_two_to_ten_thousand_metadata_records() {
        for count in [2_000usize, 10_000usize] {
            let root =
                std::env::temp_dir().join(format!("keepframe-m5-scale-{count}-{}", Uuid::new_v4()));
            super::super::initialise_layout(&root).unwrap();
            let mut connection = super::super::open_db(&root).unwrap();
            let tx = connection.transaction().unwrap();
            for index in 0..count {
                tx.execute("INSERT INTO assets(id,filename,decision,captured_at,date_fallback,thumbnail_path,created_at)VALUES(?1,?2,'undecided','2026-01-02T03:04:05Z',0,'thumb.jpg','2026-01-02T03:04:05Z')", params![format!("asset-{index:05}"),format!("photo-{index:05}.jpg")]).unwrap();
            }
            tx.commit().unwrap();
            let destination = root.join("portable");
            fs::create_dir_all(&destination).unwrap();
            let export_started = Instant::now();
            let output = export_portable_catalogue(&connection, &root, &destination).unwrap();
            let export_elapsed = export_started.elapsed();
            let rescan_started = Instant::now();
            let report = rescan_library(&mut connection, &root).unwrap();
            let rescan_elapsed = rescan_started.elapsed();
            eprintln!("M5 scale {count}: export={export_elapsed:?}, rescan={rescan_elapsed:?}");
            assert_eq!(report.scanned_assets, count);
            assert!(serde_json::from_reader::<_, serde_json::Value>(
                fs::File::open(output).unwrap()
            )
            .unwrap()["assets"]
                .as_array()
                .is_some_and(|assets| assets.len() == count));
            assert!(
                export_elapsed.as_secs_f32() < 15.0,
                "portable export unexpectedly slow at {count}: {export_elapsed:?}"
            );
            assert!(
                rescan_elapsed.as_secs_f32() < 15.0,
                "rescan unexpectedly slow at {count}: {rescan_elapsed:?}"
            );
            drop(connection);
            let _ = fs::remove_dir_all(root);
        }
    }
}

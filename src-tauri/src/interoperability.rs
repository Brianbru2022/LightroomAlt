//! Local, durable interchange and library-resilience helpers.
//!
//! These routines deliberately never change image pixels.  Sidecars and catalogue
//! exports are staged before promotion, and any relink is hash-confirmed.

use super::{hash_file, normalise_tags, KeepframeError, Result};
use chrono::Utc;
use quick_xml::{events::Event, Reader};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use std::{collections::{HashMap, HashSet}, fs, io::{BufWriter, Write}, path::{Path, PathBuf}};

pub(crate) const PORTABLE_CATALOGUE_SCHEMA_VERSION: i64 = 1;

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

fn sidecar_path(original: &Path) -> PathBuf { original.with_extension("xmp") }

fn xml_safe(value: &str) -> String {
    value.chars().filter(|character| matches!(*character, '\t' | '\n' | '\r') || *character >= ' ').flat_map(|character| match character {
        '&' => "&amp;".chars().collect::<Vec<_>>(),
        '<' => "&lt;".chars().collect(),
        '>' => "&gt;".chars().collect(),
        '\"' => "&quot;".chars().collect(),
        '\'' => "&apos;".chars().collect(),
        _ => vec![character],
    }).collect()
}

fn validate_xml(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut depth = 0i32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) if depth == 0 => return Ok(()),
            Ok(Event::Eof) => return Err(KeepframeError::Message("XMP ended before all elements were closed.".into())),
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) if depth > 0 => depth -= 1,
            Ok(Event::End(_)) => return Err(KeepframeError::Message("XMP contains an unexpected closing element.".into())),
            Ok(_) => {},
            Err(error) => return Err(KeepframeError::Message(format!("Generated XMP failed XML validation: {error}"))),
        }
        buffer.clear();
    }
}

fn read_asset(connection: &Connection, asset_id: &str) -> Result<SidecarAsset> {
    let mut asset = connection.query_row(
        "SELECT a.id,a.filename,a.decision,a.captured_at,a.latitude,a.longitude,a.embedded_latitude,a.embedded_longitude,a.manual_latitude,a.manual_longitude,COALESCE(a.location_source,'none'),r.path FROM assets a JOIN representations r ON r.id=(SELECT r2.id FROM representations r2 WHERE r2.asset_id=a.id ORDER BY r2.is_raw ASC,r2.path ASC LIMIT 1) WHERE a.id=?1 AND a.trashed_at IS NULL",
        [asset_id],
        |row| Ok(SidecarAsset { id: row.get(0)?, filename: row.get(1)?, decision: row.get(2)?, captured_at: row.get(3)?, latitude: row.get(4)?, longitude: row.get(5)?, embedded_latitude: row.get(6)?, embedded_longitude: row.get(7)?, manual_latitude: row.get(8)?, manual_longitude: row.get(9)?, location_source: row.get(10)?, original_path: PathBuf::from(row.get::<_, String>(11)?), tags: Vec::new() }),
    ).optional()?.ok_or_else(|| KeepframeError::Message("The selected catalogue item has no exportable original representation.".into()))?;
    asset.tags = connection.prepare("SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=?1 ORDER BY t.name COLLATE NOCASE")?
        .query_map([asset_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(asset)
}

fn xmp_for(asset: &SidecarAsset) -> Vec<u8> {
    let portable_label = match asset.decision.as_str() { "keep" => "Keep", "discard" => "Discard", _ => "Undecided" };
    let tags = asset.tags.iter().map(|tag| format!("<rdf:li>{}</rdf:li>", xml_safe(tag))).collect::<String>();
    let hierarchy = asset.tags.iter().filter(|tag| tag.contains('/')).map(|tag| format!("<rdf:li>{}</rdf:li>", xml_safe(tag))).collect::<String>();
    let location = match (asset.latitude, asset.longitude) {
        (Some(latitude), Some(longitude)) => format!("<exif:GPSLatitude>{latitude:.8}</exif:GPSLatitude><exif:GPSLongitude>{longitude:.8}</exif:GPSLongitude>"),
        _ => String::new(),
    };
    let provenance = [
        asset.embedded_latitude.map(|value| format!(" keepframe:embeddedLatitude=\"{value:.8}\"")),
        asset.embedded_longitude.map(|value| format!(" keepframe:embeddedLongitude=\"{value:.8}\"")),
        asset.manual_latitude.map(|value| format!(" keepframe:manualLatitude=\"{value:.8}\"")),
        asset.manual_longitude.map(|value| format!(" keepframe:manualLongitude=\"{value:.8}\"")),
    ].into_iter().flatten().collect::<String>();
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:lr="http://ns.adobe.com/lightroom/1.0/" xmlns:keepframe="https://keepframe.app/ns/1.0/" xmp:CreateDate="{}" xmp:Label="{}" keepframe:triage="{}" keepframe:locationSource="{}" keepframe:assetId="{}"{}><dc:subject><rdf:Bag>{}</rdf:Bag></dc:subject><lr:hierarchicalSubject><rdf:Bag>{}</rdf:Bag></lr:hierarchicalSubject>{}</rdf:Description></rdf:RDF></x:xmpmeta>"#,
        xml_safe(&asset.captured_at), portable_label, xml_safe(&asset.decision), xml_safe(&asset.location_source), xml_safe(&asset.id), provenance, tags, hierarchy, location).into_bytes()
}

fn staged_write(destination: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    validate_xml(bytes)?;
    let temporary = destination.with_extension("xmp.partial");
    fs::write(&temporary, bytes).map_err(|error| KeepframeError::Message(format!("Could not stage XMP sidecar {}: {error}", temporary.display())))?;
    fs::OpenOptions::new().read(true).write(true).open(&temporary).map_err(|error| KeepframeError::Message(format!("Could not validate staged XMP sidecar {}: {error}", temporary.display())))?.sync_all()?;
    if destination.exists() && !replace { let _ = fs::remove_file(&temporary); return Err(KeepframeError::Message("An XMP sidecar already exists; choose replace explicitly to update it.".into())); }
    if destination.exists() {
        let backup = destination.with_extension("xmp.keepframe-backup");
        if backup.exists() { fs::remove_file(&backup)?; }
        fs::rename(destination, &backup)?;
        if let Err(error) = fs::rename(&temporary, destination) {
            let _ = fs::rename(&backup, destination);
            return Err(KeepframeError::Io(error));
        }
        fs::remove_file(backup)?;
    } else { fs::rename(&temporary, destination).map_err(|error| KeepframeError::Message(format!("Could not promote staged XMP sidecar {}: {error}", destination.display())))?; }
    Ok(())
}

pub(crate) fn export_sidecars(connection: &mut Connection, asset_ids: &[String], replace: bool) -> Result<SidecarExportSummary> {
    let mut unique = HashSet::new();
    let ids = asset_ids.iter().filter(|id| unique.insert(id.as_str())).collect::<Vec<_>>();
    let mut result = SidecarExportSummary { requested: ids.len(), written: 0, preserved_existing: 0, failed: Vec::new() };
    for id in ids {
        let asset = match read_asset(connection, id) { Ok(asset) => asset, Err(error) => { result.failed.push(format!("{id}: {error}")); continue; } };
        if !asset.original_path.is_file() { result.failed.push(format!("{}: original representation is unavailable", asset.filename)); continue; }
        let destination = sidecar_path(&asset.original_path);
        if destination.exists() && !replace { result.preserved_existing += 1; continue; }
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

fn local_name(name: &[u8]) -> String { String::from_utf8_lossy(name).rsplit(':').next().unwrap_or_default().to_ascii_lowercase() }

fn parse_coordinate(value: &str) -> Option<f64> { value.trim().parse::<f64>().ok().filter(|value| value.is_finite()) }

fn parse_xmp(bytes: &[u8]) -> Result<SidecarMetadata> {
    validate_xml(bytes).map_err(|error| KeepframeError::Message(format!("The XMP sidecar is malformed: {error}")))?;
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
                if raw_name.ends_with("dc:subject") { in_subject = true; }
                if raw_name.ends_with("hierarchicalsubject") { in_hierarchy = true; }
                for attribute in event.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|error| KeepframeError::Message(format!("Malformed XMP attribute: {error}")))?;
                    let key = String::from_utf8_lossy(attribute.key.as_ref()).to_ascii_lowercase();
                    let value = attribute.unescape_value().map_err(|error| KeepframeError::Message(format!("Malformed XMP value: {error}")))?.into_owned();
                    if key.ends_with("triage") { metadata.decision = Some(value.clone()); }
                    if key.ends_with("locationsource") { metadata.location_source = Some(value.clone()); }
                    if key.ends_with("gpslatitude") { metadata.latitude = parse_coordinate(&value); }
                    if key.ends_with("gpslongitude") { metadata.longitude = parse_coordinate(&value); }
                    if key.ends_with("label") && metadata.decision.is_none() { metadata.decision = match value.to_ascii_lowercase().as_str() { "keep" => Some("keep".into()), "discard" => Some("discard".into()), "undecided" => Some("undecided".into()), _ => None }; }
                }
                current = name;
            }
            Ok(Event::Text(event)) => {
                let value = event.unescape().map_err(|error| KeepframeError::Message(format!("Malformed XMP text: {error}")))?.into_owned();
                if in_hierarchy && current == "li" { metadata.hierarchy.push(value); }
                else if in_subject && current == "li" { metadata.tags.push(value); }
                else if current == "gpslatitude" { metadata.latitude = parse_coordinate(&value); }
                else if current == "gpslongitude" { metadata.longitude = parse_coordinate(&value); }
            }
            Ok(Event::End(event)) => {
                let raw_name = String::from_utf8_lossy(event.name().as_ref()).to_ascii_lowercase();
                if raw_name.ends_with("dc:subject") { in_subject = false; }
                if raw_name.ends_with("hierarchicalsubject") { in_hierarchy = false; }
                current.clear();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {},
            Err(error) => return Err(KeepframeError::Message(format!("The XMP sidecar is malformed: {error}"))),
        }
        buffer.clear();
    }
    Ok(metadata)
}

pub(crate) fn import_sidecar(connection: &mut Connection, asset_id: &str) -> Result<SidecarImportResult> {
    let asset = read_asset(connection, asset_id)?;
    let path = sidecar_path(&asset.original_path);
    let metadata = parse_xmp(&fs::read(&path).map_err(|_| KeepframeError::Message("No readable XMP sidecar exists beside this original.".into()))?)?;
    let mut result = SidecarImportResult { tags_imported: false, location_imported: false, triage_imported: false, conflicts: Vec::new() };
    let old_tags = asset.tags;
    let tags = normalise_tags(metadata.tags.into_iter().chain(metadata.hierarchy).collect());
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if !tags.is_empty() { if old_tags.is_empty() {
        super::replace_asset_tags(&tx, asset_id, &tags)?; result.tags_imported = true;
    } else if old_tags != tags { result.conflicts.push("Catalogue tags already exist; sidecar tags were not applied.".into()); } }
    if let (Some(latitude), Some(longitude)) = (metadata.latitude, metadata.longitude) {
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) { result.conflicts.push("Sidecar coordinates are outside the valid range.".into()); }
        else if asset.latitude.is_none() {
            let manual = metadata.location_source.as_deref() == Some("manual");
            if manual { tx.execute("UPDATE assets SET latitude=?2,longitude=?3,manual_latitude=?2,manual_longitude=?3,location_source='manual' WHERE id=?1", params![asset_id,latitude,longitude])?; }
            else { tx.execute("UPDATE assets SET latitude=?2,longitude=?3,embedded_latitude=?2,embedded_longitude=?3,location_source='embedded' WHERE id=?1", params![asset_id,latitude,longitude])?; }
            result.location_imported = true;
        } else if asset.latitude != Some(latitude) || asset.longitude != Some(longitude) { result.conflicts.push("Catalogue location already exists; sidecar location was not applied.".into()); }
    }
    if let Some(decision) = metadata.decision.filter(|value| matches!(value.as_str(), "keep" | "undecided" | "discard")) {
        if asset.decision == "undecided" && decision != "undecided" { tx.execute("UPDATE assets SET decision=?2 WHERE id=?1", params![asset_id,decision])?; result.triage_imported = true; }
        else if asset.decision != decision { result.conflicts.push("Catalogue triage state already exists; sidecar triage was not applied.".into()); }
    }
    if result.tags_imported || result.location_imported || result.triage_imported { tx.execute("INSERT INTO audit_log(entity_type,entity_id,action,old_json,new_json,created_at)VALUES('asset',?1,'xmp_import',?2,?3,?4)", params![asset_id,json!({"tags":old_tags,"latitude":asset.latitude,"longitude":asset.longitude,"decision":asset.decision}).to_string(),json!({"tagsImported":result.tags_imported,"locationImported":result.location_imported,"triageImported":result.triage_imported}).to_string(),Utc::now().to_rfc3339()])?; }
    tx.commit()?;
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortableVersion { id: String, kind: String, provider: Option<String>, state: String, created_at: String, source_hash: Option<String>, output_hash: Option<String>, relative_path: Option<String> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortableAsset { id: String, filename: String, decision: String, captured_at: String, date_fallback: bool, camera: Option<String>, width: Option<i64>, height: Option<i64>, tags: Vec<String>, location: serde_json::Value, original: serde_json::Value, derived_versions: Vec<PortableVersion> }

fn relative_or_absolute(root: &Path, path: &str) -> Option<String> { Path::new(path).strip_prefix(root).ok().map(|relative| relative.to_string_lossy().replace('\\', "/")) }

fn rebase_managed_path(root: &Path, path: &Path, anchors: &[&str]) -> Option<PathBuf> {
    let components = path.components().map(|component| component.as_os_str().to_owned()).collect::<Vec<_>>();
    let index = components.iter().position(|component| anchors.iter().any(|anchor| component.to_string_lossy().eq_ignore_ascii_case(anchor)))?;
    let candidate = components[index..].iter().fold(root.to_path_buf(), |joined, component| joined.join(component));
    candidate.is_file().then_some(candidate)
}

/// Rebase only paths that can be verified inside a moved managed library.  This
/// never searches arbitrary folders or accepts a non-matching hash.
pub(crate) fn recover_moved_managed_paths(connection: &mut Connection, root: &Path) -> Result<usize> {
    let mut updates = 0usize;
    let representations = connection.prepare("SELECT id,path,sha256 FROM representations")?.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for (id, path, hash) in representations {
        if Path::new(&path).is_file() { continue; }
        let Some(candidate) = rebase_managed_path(root, Path::new(&path), &["Originals", "Edits"]) else { continue; };
        if hash_file(&candidate)? == hash { tx.execute("UPDATE representations SET path=?2 WHERE id=?1", params![id,candidate.to_string_lossy()])?; updates += 1; }
    }
    let versions = tx.prepare("SELECT id,path,output_hash FROM versions WHERE output_hash IS NOT NULL")?.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, path, hash) in versions {
        if Path::new(&path).is_file() { continue; }
        let Some(candidate) = rebase_managed_path(root, Path::new(&path), &["Edits"]) else { continue; };
        if hash_file(&candidate)? == hash { tx.execute("UPDATE versions SET path=?2 WHERE id=?1", params![id,candidate.to_string_lossy()])?; updates += 1; }
    }
    let thumbnails = tx.prepare("SELECT id,thumbnail_path FROM assets")?.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, path) in thumbnails {
        if !Path::new(&path).is_file() {
            if let Some(candidate) = rebase_managed_path(root, Path::new(&path), &[".keepframe"]) {
                tx.execute("UPDATE assets SET thumbnail_path=?2 WHERE id=?1", params![id,candidate.to_string_lossy()])?;
                updates += 1;
            }
        }
    }
    tx.commit()?;
    Ok(updates)
}

pub(crate) fn export_portable_catalogue(connection: &Connection, root: &Path, destination: &Path) -> Result<PathBuf> {
    if !destination.is_dir() { return Err(KeepframeError::Message("Choose an existing folder for the portable catalogue export.".into())); }
    let output = destination.join("keepframe-portable-catalogue-v1.json");
    let temporary = output.with_extension("json.partial");
    let mut tag_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut tags = connection.prepare("SELECT at.asset_id,t.name FROM asset_tags at JOIN tags t ON t.id=at.tag_id ORDER BY at.asset_id,t.name COLLATE NOCASE")?;
    for row in tags.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? { let (asset, tag) = row?; tag_map.entry(asset).or_default().push(tag); }
    let mut version_map: HashMap<String, Vec<PortableVersion>> = HashMap::new();
    let mut versions = connection.prepare("SELECT asset_id,id,kind,provider,state,created_at,source_hash,output_hash,path FROM versions ORDER BY asset_id,created_at,id")?;
    for row in versions.query_map([], |row| Ok((row.get::<_, String>(0)?, PortableVersion { id: row.get(1)?,kind: row.get(2)?,provider: row.get(3)?,state: row.get(4)?,created_at: row.get(5)?,source_hash: row.get(6)?,output_hash: row.get(7)?,relative_path: relative_or_absolute(root, &row.get::<_, String>(8)?)})))? { let (asset, version) = row?; version_map.entry(asset).or_default().push(version); }
    let file = fs::File::create(&temporary)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(format!("{{\"format\":\"keepframe-portable-catalogue\",\"schemaVersion\":{PORTABLE_CATALOGUE_SCHEMA_VERSION},\"exportedAt\":\"{}\",\"assets\":[", Utc::now().to_rfc3339()).as_bytes())?;
    let mut assets = connection.prepare("SELECT a.id,a.filename,a.decision,a.captured_at,a.date_fallback,a.camera,a.width,a.height,a.latitude,a.longitude,a.embedded_latitude,a.embedded_longitude,a.manual_latitude,a.manual_longitude,COALESCE(a.location_source,'none'),r.path,r.sha256,r.byte_size,r.extension,r.is_raw FROM assets a LEFT JOIN representations r ON r.id=(SELECT r2.id FROM representations r2 WHERE r2.asset_id=a.id ORDER BY r2.is_raw ASC,r2.path ASC LIMIT 1) ORDER BY a.captured_at DESC,a.id")?;
    let mut first = true;
    for row in assets.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, i64>(4)?,row.get::<_, Option<String>>(5)?,row.get::<_, Option<i64>>(6)?,row.get::<_, Option<i64>>(7)?,row.get::<_, Option<f64>>(8)?,row.get::<_, Option<f64>>(9)?,row.get::<_, Option<f64>>(10)?,row.get::<_, Option<f64>>(11)?,row.get::<_, Option<f64>>(12)?,row.get::<_, Option<f64>>(13)?,row.get::<_, String>(14)?,row.get::<_, Option<String>>(15)?,row.get::<_, Option<String>>(16)?,row.get::<_, Option<i64>>(17)?,row.get::<_, Option<String>>(18)?,row.get::<_, Option<i64>>(19)?)))? {
        let (id,filename,decision,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,path,sha256,byte_size,extension,is_raw) = row?;
        let record = PortableAsset { id: id.clone(), filename, decision, captured_at, date_fallback: date_fallback != 0, camera, width, height, tags: tag_map.remove(&id).unwrap_or_default(), location: json!({"latitude":latitude,"longitude":longitude,"source":location_source,"embeddedLatitude":embedded_latitude,"embeddedLongitude":embedded_longitude,"manualLatitude":manual_latitude,"manualLongitude":manual_longitude}), original: json!({"path":path,"managedRelativePath":path.as_deref().and_then(|path| relative_or_absolute(root,path)),"sha256":sha256,"byteSize":byte_size,"extension":extension,"isRaw":is_raw.map(|value| value != 0)}), derived_versions: version_map.remove(&id).unwrap_or_default() };
        if !first { writer.write_all(b",")?; } first = false;
        serde_json::to_writer(&mut writer, &record)?;
    }
    writer.write_all(b"]}")?; writer.flush()?; drop(writer);
    fs::OpenOptions::new().read(true).write(true).open(&temporary)?.sync_all()?;
    serde_json::from_reader::<_, serde_json::Value>(fs::File::open(&temporary)?).map_err(|error| KeepframeError::Message(format!("Portable catalogue validation failed: {error}")))?;
    if output.exists() { let _ = fs::remove_file(&output); }
    fs::rename(&temporary, &output)?;
    Ok(output)
}

pub(crate) fn rescan_library(connection: &mut Connection, root: &Path) -> Result<IntegrityReport> {
    let mut report = IntegrityReport { scanned_assets: 0, missing_originals: 0, missing_derived_versions: 0, modified_originals: 0, untracked_managed_files: 0, sidecar_conflicts: 0, findings: Vec::new() };
    let mut known_paths = HashSet::new();
    let mut asset_rows = connection.prepare("SELECT id,filename FROM assets WHERE trashed_at IS NULL ORDER BY id")?;
    let assets = asset_rows.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    drop(asset_rows);
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for (asset_id, filename) in assets {
        report.scanned_assets += 1;
        let mut state = "available";
        let mut has_representation = false;
        let mut representations = tx.prepare("SELECT path,sha256,byte_size FROM representations WHERE asset_id=?1")?;
        for row in representations.query_map([&asset_id], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?)))? {
            has_representation = true;
            let (path, hash, byte_size) = row?; known_paths.insert(PathBuf::from(&path));
            let candidate = PathBuf::from(&path);
            if !candidate.is_file() { state = "original_missing"; report.missing_originals += 1; report.findings.push(IntegrityFinding { asset_id: Some(asset_id.clone()), filename: Some(filename.clone()), kind: "original_missing".into(), detail: path }); }
            else if fs::metadata(&candidate)?.len() as i64 != byte_size && hash_file(&candidate)? != hash { state = "modified"; report.modified_originals += 1; report.findings.push(IntegrityFinding { asset_id: Some(asset_id.clone()), filename: Some(filename.clone()), kind: "original_modified".into(), detail: candidate.to_string_lossy().into() }); }
        }
        if !has_representation { state = "orphaned"; report.findings.push(IntegrityFinding { asset_id: Some(asset_id.clone()), filename: Some(filename.clone()), kind: "orphaned_catalogue_record".into(), detail: "No original representation is registered for this catalogue item.".into() }); }
        let mut versions = tx.prepare("SELECT path FROM versions WHERE asset_id=?1")?;
        for row in versions.query_map([&asset_id], |row| row.get::<_, String>(0))? { let path = row?; known_paths.insert(PathBuf::from(&path)); if !Path::new(&path).is_file() { if state == "available" { state = "derived_missing"; } report.missing_derived_versions += 1; report.findings.push(IntegrityFinding { asset_id: Some(asset_id.clone()), filename: Some(filename.clone()), kind: "derived_missing".into(), detail: path }); } }
        tx.execute("UPDATE assets SET missing_state=?2,last_verified_at=?3 WHERE id=?1", params![asset_id,state,Utc::now().to_rfc3339()])?;
    }
    let mut sidecars = tx.prepare("SELECT s.asset_id,a.filename,s.path,s.content_hash FROM sidecar_exports s JOIN assets a ON a.id=s.asset_id")?;
    for row in sidecars.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?)))? { let (asset_id, filename, path, hash) = row?; if !Path::new(&path).is_file() || hash_file(Path::new(&path)).ok().as_deref() != Some(&hash) { report.sidecar_conflicts += 1; report.findings.push(IntegrityFinding { asset_id: Some(asset_id), filename: Some(filename), kind: "sidecar_conflict".into(), detail: path }); } }
    for directory in [root.join("Originals"), root.join("Edits")] { if directory.is_dir() { for entry in walkdir::WalkDir::new(directory).into_iter().filter_map(std::result::Result::ok).filter(|entry| entry.file_type().is_file()) { let path = entry.path().to_path_buf(); if !known_paths.contains(&path) { report.untracked_managed_files += 1; report.findings.push(IntegrityFinding { asset_id: None, filename: path.file_name().map(|name| name.to_string_lossy().into()), kind: "untracked_managed_file".into(), detail: path.to_string_lossy().into() }); } } } }
    drop(sidecars);
    tx.commit()?;
    Ok(report)
}

pub(crate) fn relink_candidates(connection: &Connection, asset_id: &str, directory: &Path) -> Result<Vec<RelinkCandidate>> {
    if !directory.is_dir() { return Err(KeepframeError::Message("Choose an existing folder to search for a confirmed relink.".into())); }
    let mut representations = connection.prepare("SELECT path,sha256,byte_size FROM representations WHERE asset_id=?1 ORDER BY is_raw ASC,path ASC")?;
    let missing = representations.query_map([asset_id], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?.into_iter().find(|(path, _, _)| !Path::new(path).is_file());
    let Some((_missing_path, expected_hash, expected_size)) = missing else { return Ok(Vec::new()); };
    let mut matches = Vec::new();
    for entry in walkdir::WalkDir::new(directory).into_iter().filter_map(std::result::Result::ok).filter(|entry| entry.file_type().is_file()) {
        let metadata = entry.metadata().map_err(|error| KeepframeError::Message(format!("Could not inspect a relink candidate: {error}")))?;
        if metadata.len() as i64 == expected_size && hash_file(entry.path())? == expected_hash { matches.push(RelinkCandidate { path: entry.path().to_string_lossy().into(), sha256: expected_hash.clone() }); }
    }
    Ok(matches)
}

pub(crate) fn relink_asset(connection: &mut Connection, asset_id: &str, replacement: &Path) -> Result<()> {
    if !replacement.is_file() { return Err(KeepframeError::Message("The selected relink target is not a readable file.".into())); }
    let mut representations = connection.prepare("SELECT id,path,sha256,byte_size FROM representations WHERE asset_id=?1 ORDER BY is_raw ASC,path ASC")?;
    let target = representations.query_map([asset_id], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, i64>(3)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?.into_iter().find(|(_, path, _, _)| !Path::new(path).is_file())
        .ok_or_else(|| KeepframeError::Message("This asset has no missing original representation to relink.".into()))?;
    drop(representations);
    if fs::metadata(replacement)?.len() as i64 != target.3 || hash_file(replacement)? != target.2 { return Err(KeepframeError::Message("Relink was rejected because the chosen file does not exactly match the catalogue hash.".into())); }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("UPDATE representations SET path=?2 WHERE id=?1", params![target.0,replacement.to_string_lossy()])?;
    tx.execute("UPDATE assets SET missing_state='available',last_verified_at=?2 WHERE id=?1", params![asset_id,Utc::now().to_rfc3339()])?;
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
        let first = export_sidecars(&mut connection, std::slice::from_ref(&asset_id), false).unwrap();
        assert_eq!(first.written, 1, "{first:?}");
        let sidecar = sidecar_path(&source);
        let content = fs::read_to_string(&sidecar).unwrap();
        assert!(content.contains("<dc:subject>"));
        assert!(content.contains("keepframe:locationSource=\"embedded\""));
        fs::write(&sidecar, b"<?xml version=\"1.0\"?><manual/>").unwrap();
        let preserved = export_sidecars(&mut connection, &[asset_id], false).unwrap();
        assert_eq!(preserved.preserved_existing, 1);
        assert!(fs::read_to_string(&sidecar).unwrap().contains("manual"));
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_xmp_is_rejected_without_catalogue_change() {
        let (root, mut connection, asset_id, source) = fixture();
        fs::write(sidecar_path(&source), b"<rdf:RDF><bad>").unwrap();
        assert!(import_sidecar(&mut connection, &asset_id).is_err());
        let state: String = connection.query_row("SELECT decision FROM assets WHERE id=?1", [&asset_id], |row| row.get(0)).unwrap();
        assert_eq!(state, "undecided");
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn conservative_xmp_import_fills_only_empty_catalogue_fields() {
        let (root, mut connection, asset_id, source) = fixture();
        connection.execute("UPDATE assets SET latitude=NULL,longitude=NULL,embedded_latitude=NULL,embedded_longitude=NULL,location_source='none' WHERE id=?1", [&asset_id]).unwrap();
        fs::write(sidecar_path(&source), br#"<?xml version="1.0"?><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:keepframe="https://keepframe.app/ns/1.0/" keepframe:triage="keep" keepframe:locationSource="manual"><dc:subject xmlns:dc="http://purl.org/dc/elements/1.1/"><rdf:Bag><rdf:li>People/Hazel</rdf:li></rdf:Bag></dc:subject><exif:GPSLatitude xmlns:exif="http://ns.adobe.com/exif/1.0/">56.0000</exif:GPSLatitude><exif:GPSLongitude xmlns:exif="http://ns.adobe.com/exif/1.0/">-3.0000</exif:GPSLongitude></rdf:Description></rdf:RDF>"#).unwrap();
        let result = import_sidecar(&mut connection, &asset_id).unwrap();
        assert!(result.tags_imported && result.location_imported && result.triage_imported);
        let state: (String, Option<f64>, String) = connection.query_row("SELECT decision,manual_latitude,location_source FROM assets WHERE id=?1", [&asset_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(state, ("keep".into(), Some(56.0), "manual".into()));
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_catalogue_is_versioned_and_keeps_manual_location_provenance() {
        let (root, connection, asset_id, _source) = fixture();
        connection.execute("UPDATE assets SET latitude=56.0,longitude=-3.0,manual_latitude=56.0,manual_longitude=-3.0,location_source='manual' WHERE id=?1", [&asset_id]).unwrap();
        let destination = root.join("portable"); fs::create_dir_all(&destination).unwrap();
        let output = export_portable_catalogue(&connection, &root, &destination).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], PORTABLE_CATALOGUE_SCHEMA_VERSION);
        assert_eq!(value["assets"][0]["location"]["source"], "manual");
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rescan_marks_missing_and_relink_requires_an_exact_hash() {
        let (root, mut connection, asset_id, source) = fixture();
        let bytes = fs::read(&source).unwrap(); fs::remove_file(&source).unwrap();
        let report = rescan_library(&mut connection, &root).unwrap();
        assert_eq!(report.missing_originals, 1);
        let candidates_root = root.join("relink-candidates"); fs::create_dir_all(&candidates_root).unwrap();
        fs::write(candidates_root.join("one.jpg"), &bytes).unwrap();
        fs::write(candidates_root.join("two.jpg"), &bytes).unwrap();
        let candidates = relink_candidates(&connection, &asset_id, &candidates_root).unwrap();
        assert_eq!(candidates.len(), 2, "ambiguous hash matches are returned for an explicit user choice");
        assert!(relink_asset(&mut connection, &asset_id, &candidates_root.join("not-a-match.jpg")).is_err());
        relink_asset(&mut connection, &asset_id, Path::new(&candidates[0].path)).unwrap();
        assert_eq!(connection.query_row("SELECT missing_state FROM assets WHERE id=?1", [&asset_id], |row| row.get::<_, String>(0)).unwrap(), "available");
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn moved_managed_library_paths_rebase_only_after_hash_confirmation() {
        let (root, mut connection, _asset_id, source) = fixture();
        let old_path = PathBuf::from(r"Z:\FormerLibrary\Originals\2026\01\01\photo.jpg");
        connection.execute("UPDATE representations SET path=?1 WHERE id='rep-m5'", [old_path.to_string_lossy().as_ref()]).unwrap();
        assert_eq!(recover_moved_managed_paths(&mut connection, &root).unwrap(), 1);
        let rebased: String = connection.query_row("SELECT path FROM representations WHERE id='rep-m5'", [], |row| row.get(0)).unwrap();
        assert_eq!(PathBuf::from(rebased), source);
        drop(connection); let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_export_and_rescan_scale_from_two_to_ten_thousand_metadata_records() {
        for count in [2_000usize, 10_000usize] {
            let root = std::env::temp_dir().join(format!("keepframe-m5-scale-{count}-{}", Uuid::new_v4()));
            super::super::initialise_layout(&root).unwrap();
            let mut connection = super::super::open_db(&root).unwrap();
            let tx = connection.transaction().unwrap();
            for index in 0..count {
                tx.execute("INSERT INTO assets(id,filename,decision,captured_at,date_fallback,thumbnail_path,created_at)VALUES(?1,?2,'undecided','2026-01-02T03:04:05Z',0,'thumb.jpg','2026-01-02T03:04:05Z')", params![format!("asset-{index:05}"),format!("photo-{index:05}.jpg")]).unwrap();
            }
            tx.commit().unwrap();
            let destination = root.join("portable"); fs::create_dir_all(&destination).unwrap();
            let export_started = Instant::now();
            let output = export_portable_catalogue(&connection, &root, &destination).unwrap();
            let export_elapsed = export_started.elapsed();
            let rescan_started = Instant::now();
            let report = rescan_library(&mut connection, &root).unwrap();
            let rescan_elapsed = rescan_started.elapsed();
            eprintln!("M5 scale {count}: export={export_elapsed:?}, rescan={rescan_elapsed:?}");
            assert_eq!(report.scanned_assets, count);
            assert!(serde_json::from_reader::<_, serde_json::Value>(fs::File::open(output).unwrap()).unwrap()["assets"].as_array().is_some_and(|assets| assets.len() == count));
            assert!(export_elapsed.as_secs_f32() < 15.0, "portable export unexpectedly slow at {count}: {export_elapsed:?}");
            assert!(rescan_elapsed.as_secs_f32() < 15.0, "rescan unexpectedly slow at {count}: {rescan_elapsed:?}");
            drop(connection); let _ = fs::remove_dir_all(root);
        }
    }
}

mod interoperability;mod migrations;
mod professional_export;
mod safety;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
use fs2::FileExt;
use image::{GenericImageView, ImageDecoder, ImageFormat, RgbImage};
use notify::{EventKind, RecursiveMode, Watcher};
use rayon::prelude::*;
use reqwest::multipart;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},mpsc, Arc, Mutex,},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;
use walkdir::WalkDir;

const SCHEMA_VERSION: i64 = 10;
const PRESET_FILE_MAX_BYTES: u64 = 128 * 1024;
const SEGMENTATION_MODEL_ID: &str = "microsoft/beit-base-finetuned-ade-640-640";
const SEGMENTATION_MODEL_REVISION: &str = "a8b6f5ef4acb2ea55d882989deaa02d39401e2b2";
const SEGMENTATION_MODEL_SHA256: &str =
    "e0747360d190bd7c0f53d2fe3b2ed560c304d3eefef94574af9c7e93aaf8e7a9";
const SEGMENTATION_MODEL_BYTES: u64 = 899_902_905;
const SEGMENTATION_PROVIDER: &str = "Keepframe BEiT semantic segmentation";
const SEGMENTATION_PROVIDER_VERSION: &str = "1.0.0";
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
    mask_generation: AtomicU64,
    preview_generation: Arc<AtomicU64>,
    renderer_caches: Arc<Mutex<RendererCaches>>,
    mask_install_cancel: AtomicBool,
    export_generation: Arc<AtomicU64>,
    folder_watcher: Mutex<Option<FolderWatcher>>,
    gpu_gate: Arc<tokio::sync::Mutex<()>>,
}

const DECODE_CACHE_BYTES: usize = 96 * 1024 * 1024;
const INTERMEDIATE_CACHE_BYTES: usize = 96 * 1024 * 1024;
const MASK_CACHE_BYTES: usize = 64 * 1024 * 1024;
const SEMANTIC_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
struct CacheEntry<T> { key: String, value: Arc<T>, bytes: usize }

#[derive(Debug)]
struct BoundedCache<T> {
    entries: VecDeque<CacheEntry<T>>,
    used_bytes: usize,
    limit_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl<T> BoundedCache<T> {
    fn new(limit_bytes: usize) -> Self {
        Self { entries: VecDeque::new(), used_bytes: 0, limit_bytes, hits: 0, misses: 0, evictions: 0 }
    }
    fn get(&mut self, key: &str) -> Option<Arc<T>> {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self.entries.remove(index).expect("cache index remains valid");
            let value = Arc::clone(&entry.value);
            self.entries.push_back(entry);
            self.hits += 1;
            Some(value)
        } else {
            self.misses += 1;
            None
        }
    }
    fn insert(&mut self, key: String, value: Arc<T>, bytes: usize) {
        if bytes > self.limit_bytes { return; }
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let old = self.entries.remove(index).expect("cache index remains valid");
            self.used_bytes = self.used_bytes.saturating_sub(old.bytes);
        }
        while self.used_bytes.saturating_add(bytes) > self.limit_bytes {
            let Some(old) = self.entries.pop_front() else { break; };
            self.used_bytes = self.used_bytes.saturating_sub(old.bytes);
            self.evictions += 1;
        }
        self.used_bytes += bytes;
        self.entries.push_back(CacheEntry { key, value, bytes });
    }
}

#[derive(Debug)]
struct RendererCaches {
    decoded: BoundedCache<image::DynamicImage>,
    intermediates: BoundedCache<RgbImage>,
    masks: BoundedCache<Vec<f32>>,
    semantics: BoundedCache<image::GrayImage>,
}

impl Default for RendererCaches {
    fn default() -> Self {
        Self {
            decoded: BoundedCache::new(DECODE_CACHE_BYTES),
            intermediates: BoundedCache::new(INTERMEDIATE_CACHE_BYTES),
            masks: BoundedCache::new(MASK_CACHE_BYTES),
            semantics: BoundedCache::new(SEMANTIC_CACHE_BYTES),
        }
    }
}
struct FolderWatcher {
    _watcher: notify::RecommendedWatcher,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchEventRecord {
    id: i64,
    folder_path: String,
    path: String,
    kind: String,
    observed_at: String,
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
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
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
    horizontal_flip: bool,
    vertical_flip: bool,
}

impl BasicAdjustments {
    fn neutral() -> Self {
        Self { crop_width: 1.0, crop_height: 1.0, ..Self::default() }
    }

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
        (self.crop_left, self.crop_top, if self.crop_width == 0.0 { 1.0 } else { self.crop_width }, if self.crop_height == 0.0 { 1.0 } else { self.crop_height },)
    }
}

impl Default for BasicAdjustments {
    fn default() -> Self {
        Self {
            exposure: 0.0, light_balance: 0.0, tint: 0.0, contrast: 0.0, highlights: 0.0,
            shadows: 0.0, whites: 0.0, blacks: 0.0, dynamic_range: 0.0, texture: 0.0,
            clarity: 0.0, dehaze: 0.0, colour_boost: 0.0, saturation: 0.0,
            curve_highlights: 0.0, curve_lights: 0.0, curve_darks: 0.0, curve_shadows: 0.0,
            crop_left: 0.0, crop_top: 0.0, crop_width: 1.0, crop_height: 1.0,
            rotate_quadrants: 0, straighten: 0.0, horizontal_flip: false, vertical_flip: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DevelopRecipe {
    schema_version: i64,
    settings: BasicAdjustments,
    #[serde(default)]
    masks: Vec<DevelopMask>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct IntelligentMaskHealth {
    available: bool,
    installed: bool,
    runtime_available: bool,
    loaded: bool,
    busy: bool,
    provider: String,
    provider_version: String,
    model: String,
    model_revision: String,
    licence: String,
    source: String,
    approximate_bytes: u64,
    storage_path: String,
    execution_provider: String,
    detail: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerSegmentationResult {
    width: u32,
    height: u32,
    confidence: f32,
    coverage_fraction: f32,
    coverage_png: String,
    checksum: String,
    execution_provider: String,
    provider: String,
    provider_version: String,
    model: String,
    model_revision: String,
    model_sha256: String,
    timings: Value,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IntelligentMaskProposal {
    request_id: u64,
    category: String,
    confidence: f32,
    coverage_fraction: f32,
    elapsed_ms: u128,
    mask: DevelopMask,
    timings: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
struct LocalAdjustments {
    exposure: f32, contrast: f32, highlights: f32, shadows: f32, whites: f32, blacks: f32,
    light_balance: f32, tint: f32, saturation: f32, clarity: f32, dehaze: f32, texture: f32,
}

impl LocalAdjustments {
    fn validate(self) -> Result<Self> {
        let values = [self.contrast,self.highlights,self.shadows,self.whites,self.blacks,self.light_balance,self.tint,self.saturation,self.clarity,self.dehaze,self.texture,];
        if !self.exposure.is_finite() || !(-3.0..=3.0).contains(&self.exposure) || values.iter().any(|value| !value.is_finite() || !(-100.0..=100.0).contains(value)) {
            return Err(KeepframeError::Message("A local adjustment is outside the supported range.".into(),));
        }
        Ok(self)
    }
    fn as_basic(self) -> BasicAdjustments { BasicAdjustments { exposure:self.exposure,contrast:self.contrast,highlights:self.highlights,shadows:self.shadows,whites:self.whites,blacks:self.blacks,light_balance:self.light_balance,tint:self.tint,saturation:self.saturation,clarity:self.clarity,dehaze:self.dehaze,texture:self.texture,..BasicAdjustments::neutral() } }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
struct MaskPoint { x: f32, y: f32, }
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct BrushStroke { points: Vec<MaskPoint>, radius: f32, feather: f32, flow: f32, erase: bool, }
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum MaskGeometry {
    Linear { start: MaskPoint, end: MaskPoint, },
    Radial { centre: MaskPoint, radius_x: f32, radius_y: f32, rotation: f32, },
    Brush { strokes: Vec<BrushStroke>, },
    Semantic {
        width: u32,
        height: u32,
        coverage_png: String,
        checksum: String,
        provenance: Box<MaskProvenance>,
        refinements: Vec<BrushStroke>,
    },
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct MaskProvenance {
    provider: String,
    provider_version: String,
    model: String,
    model_revision: String,
    model_sha256: String,
    category: String,
    execution_provider: String,
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DevelopMask { id: String, name: String, enabled: bool, inverted: bool, opacity: f32, feather: f32, geometry: MaskGeometry, adjustments: LocalAdjustments,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|value| value.is_ascii_hexdigit())
}
fn valid_strokes<F: Fn(&MaskPoint) -> bool>(strokes: &[BrushStroke], valid_point: &F) -> bool {
    strokes.len() <= 5000
        && strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum::<usize>()
            <= 200_000
        && strokes.iter().all(|stroke| {
            !stroke.points.is_empty()
                && stroke.points.iter().all(valid_point)
                && stroke.radius.is_finite()
                && (0.001..=0.5).contains(&stroke.radius)
                && stroke.feather.is_finite()
                && (0.0..=1.0).contains(&stroke.feather)
                && stroke.flow.is_finite()
                && (0.0..=1.0).contains(&stroke.flow)
        })
}
fn decode_semantic_payload(coverage_png: &str, checksum: &str) -> Result<image::GrayImage> {
    let bytes = BASE64.decode(coverage_png).map_err(|_| {
        KeepframeError::Message("An intelligent mask contains invalid base64 coverage data.".into())
    })?;
    if bytes.len() > 3_000_000
        || format!("{:x}", Sha256::digest(&bytes)) != checksum.to_ascii_lowercase()
    {
        return Err(KeepframeError::Message(
            "An intelligent mask failed its coverage checksum.".into(),
        ));
    }
    let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)?.to_luma8();
    Ok(image) }

const PRESET_CATEGORIES: &[&str] = &["whiteBalance", "tone", "presence", "colour"];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DevelopPreset {
    schema_version: i64,
    id: String,
    name: String,
    categories: Vec<String>,
    settings: BasicAdjustments,
    built_in: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetFile {
    schema_version: i64,
    name: String,
    categories: Vec<String>,
    settings: BasicAdjustments,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ImageStatistics {
    luminance_bins: Vec<u64>,
    red_bins: Vec<u64>,
    green_bins: Vec<u64>,
    blue_bins: Vec<u64>,
    samples: u64,
    average_luminance: f32,
    p01: f32,
    p50: f32,
    p99: f32,
    shadow_clip_fraction: f32,
    highlight_clip_fraction: f32,
    average_saturation: f32,
    red_green_blue: [f32; 3],
    dynamic_range: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoProposal {
    settings: BasicAdjustments,
    explanation: Vec<String>,
    confidence: f32,
    statistics: ImageStatistics,
    recommendations: Vec<String>,
}

impl DevelopRecipe {
    fn neutral() -> Self { Self { schema_version: 2, settings: BasicAdjustments::neutral(), masks: Vec::new(), } }
    fn validate(mut self) -> Result<Self> {
        if self.schema_version == 1 && self.masks.is_empty() { self.schema_version = 2; }
        if self.schema_version != 2 {
            return Err(KeepframeError::Message("This Develop recipe version is not supported by this build.".into(),));
        }
        self.settings = self.settings.validate()?;
        if self.masks.len() > 64 { return Err(KeepframeError::Message("A Develop recipe cannot contain more than 64 masks.".into(),)); }
        let mut ids = std::collections::HashSet::new();
        for mask in &mut self.masks {
            mask.name = mask.name.trim().to_string();
            if mask.id.trim().is_empty() || !ids.insert(mask.id.clone()) || mask.name.chars().count() > 80 || !mask.opacity.is_finite() || !(0.0..=1.0).contains(&mask.opacity) || !mask.feather.is_finite() || !(0.0..=1.0).contains(&mask.feather) { return Err(KeepframeError::Message("A mask definition is invalid.".into(),)); }
            mask.adjustments = mask.adjustments.validate()?;
            let valid_point = |point: &MaskPoint| { point.x.is_finite() && point.y.is_finite() && (0.0..=1.0).contains(&point.x) && (0.0..=1.0).contains(&point.y)
            };
            match &mask.geometry {
                MaskGeometry::Linear { start, end } if valid_point(start) && valid_point(end) && ((start.x-end.x).powi(2)+(start.y-end.y).powi(2)).sqrt() >= 0.001 => {}
                MaskGeometry::Radial { centre, radius_x, radius_y, rotation, } if valid_point(centre) && radius_x.is_finite() && radius_y.is_finite() && (0.005..=2.0).contains(radius_x) && (0.005..=2.0).contains(radius_y) && rotation.is_finite() && (-360.0..=360.0).contains(rotation) => {}
                MaskGeometry::Brush { strokes } if valid_strokes( strokes, &valid_point) => {}
                MaskGeometry::Semantic {
                    width,
                    height,
                    coverage_png,
                    checksum,
                    provenance,
                    refinements,
                } if (16..=2048).contains(width)
                    && (16..=2048).contains(height)
                    && coverage_png.len() <= 4_000_000 && is_sha256(checksum)
                    && is_sha256(&provenance.model_sha256)
                    && matches!(provenance.category.as_str(), "subject" | "people" | "sky")
                    && !provenance.provider.trim().is_empty() && !provenance.provider_version.trim().is_empty() && !provenance.model.trim ().is_empty() && !provenance.model_revision.trim ().is_empty() && !provenance.execution_provider.trim ().is_empty()&& valid_strokes(refinements, &valid_point) => {
                    let coverage = decode_semantic_payload(coverage_png, checksum)?;
                    if coverage.dimensions() != (*width, *height) {
                        return Err(KeepframeError::Message(
                            "An intelligent mask has inconsistent canonical dimensions.".into(),
                        ));}
                }
                _ => { return Err(KeepframeError::Message("A mask has invalid geometry or stroke data.".into(),))
                }
            }
        }
        Ok(self)
    }
    fn is_edited(&self) -> bool { self.settings != BasicAdjustments::neutral() || !self.masks.is_empty() }
}

impl DevelopPreset {
    fn validate(mut self, user_authored: bool) -> Result<Self> {
        if self.schema_version != 1 {
            return Err(KeepframeError::Message("This Develop preset version is not supported by this build.".into(),));
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > 80 || self.name.chars().any(char::is_control) {
            return Err(KeepframeError::Message("Preset names must be 1 to 80 printable characters.".into(),));
        }
        self.categories.sort();
        self.categories.dedup();
        if self.categories.is_empty() || self.categories.iter().any(|category| !PRESET_CATEGORIES.contains(&category.as_str())) {
            return Err(KeepframeError::Message("A preset contains an unsupported settings category.".into(),));
        }
        if user_authored && (self.built_in || self.id.trim().is_empty()) {
            return Err(KeepframeError::Message("User presets must have an application-generated id and cannot claim to be built in.".into()));
        }
        self.settings = self.settings.validate()?;
        Ok(self)
    }
}

fn built_in_presets() -> Vec<DevelopPreset> {
    let preset = |id: &str, name: &str, categories: &[&str], settings: BasicAdjustments| DevelopPreset {
        schema_version: 1, id: id.into(), name: name.into(), categories: categories.iter().map(|value| (*value).into()).collect(), settings, built_in: true,
    };
    vec![
        preset("natural", "Natural", &["tone", "colour"], BasicAdjustments::neutral(),),
        preset("clean", "Clean", &["tone", "presence", "colour"], BasicAdjustments { contrast: 4.0, highlights: -8.0, shadows: 6.0, clarity: 3.0, colour_boost: 3.0, ..BasicAdjustments::neutral() },),
        preset("warm", "Warm", &["whiteBalance", "tone", "colour"], BasicAdjustments { light_balance: 14.0, tint: 2.0, contrast: 4.0, colour_boost: 6.0, ..BasicAdjustments::neutral() },),
        preset("cool", "Cool", &["whiteBalance", "tone", "colour"], BasicAdjustments { light_balance: -12.0, tint: -2.0, highlights: -8.0, colour_boost: 3.0, ..BasicAdjustments::neutral() },),
        preset("high-contrast", "High Contrast", &["tone", "presence"], BasicAdjustments { contrast: 20.0, highlights: -8.0, shadows: -5.0, whites: 8.0, blacks: -10.0, clarity: 5.0, ..BasicAdjustments::neutral() },),
        preset("soft-contrast", "Soft Contrast", &["tone", "presence"], BasicAdjustments { contrast: -14.0, highlights: -12.0, shadows: 12.0, texture: -5.0, ..BasicAdjustments::neutral() },),
        preset("vivid", "Vivid", &["tone", "colour"], BasicAdjustments { contrast: 8.0, colour_boost: 20.0, saturation: 5.0, ..BasicAdjustments::neutral() },),
        preset("muted", "Muted", &["tone", "colour"], BasicAdjustments { contrast: -4.0, colour_boost: -8.0, saturation: -22.0, ..BasicAdjustments::neutral() },),
        preset("portrait", "Portrait", &["tone", "presence", "colour"], BasicAdjustments { contrast: -3.0, highlights: -10.0, shadows: 8.0, texture: -12.0, clarity: -5.0, colour_boost: 5.0, ..BasicAdjustments::neutral() },),
        preset("landscape", "Landscape", &["tone", "presence", "colour"], BasicAdjustments { contrast: 10.0, highlights: -10.0, dehaze: 7.0, clarity: 8.0, colour_boost: 15.0, ..BasicAdjustments::neutral() },),
        preset("black-and-white", "Black & White", &["tone", "presence", "colour"], BasicAdjustments { contrast: 10.0, clarity: 4.0, saturation: -100.0, ..BasicAdjustments::neutral() },),
        preset("high-key-bw", "High-Key B&W", &["tone", "presence", "colour"], BasicAdjustments { exposure: 0.45, contrast: -8.0, shadows: 18.0, blacks: 10.0, clarity: -3.0, saturation: -100.0, ..BasicAdjustments::neutral() },),
        preset("low-key-bw", "Low-Key B&W", &["tone", "presence", "colour"], BasicAdjustments { exposure: -0.45, contrast: 18.0, highlights: -15.0, blacks: -18.0, clarity: 6.0, saturation: -100.0, ..BasicAdjustments::neutral() },),
    ]
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
    missing_state: String,
    tags: Vec<String>,
    representation_count: i64,
    preferred_version_url: Option<String>,
    has_edits: bool,
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
      CREATE TABLE IF NOT EXISTS assets(id TEXT PRIMARY KEY, filename TEXT NOT NULL, decision TEXT NOT NULL DEFAULT 'undecided' CHECK(decision IN('keep','undecided','discard')), captured_at TEXT NOT NULL, date_fallback INTEGER NOT NULL DEFAULT 0, camera TEXT, width INTEGER, height INTEGER, latitude REAL, longitude REAL, embedded_latitude REAL, embedded_longitude REAL, manual_latitude REAL, manual_longitude REAL, location_source TEXT NOT NULL DEFAULT 'none', missing_state TEXT NOT NULL DEFAULT 'available', last_verified_at TEXT, thumbnail_path TEXT NOT NULL, preferred_version_id TEXT, created_at TEXT NOT NULL, trashed_at TEXT);
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
      CREATE INDEX IF NOT EXISTS idx_assets_missing_state ON assets(missing_state,captured_at DESC);
      CREATE TABLE IF NOT EXISTS sidecar_exports(asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,path TEXT NOT NULL,content_hash TEXT NOT NULL,exported_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS watch_folders(path TEXT PRIMARY KEY,enabled INTEGER NOT NULL DEFAULT 1,created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS watch_events(id INTEGER PRIMARY KEY AUTOINCREMENT,folder_path TEXT NOT NULL,path TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN('new_file','file_changed','file_removed')),observed_at TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'inbox',UNIQUE(folder_path,path,kind,state));
      CREATE TABLE IF NOT EXISTS imports(id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at TEXT NOT NULL, completed_at TEXT, discovered INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0, duplicates INTEGER NOT NULL DEFAULT 0, unsupported INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT 'running', mode TEXT NOT NULL DEFAULT 'copy', duplicate_policy TEXT NOT NULL DEFAULT 'retain', cancel_requested INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS import_items(id TEXT PRIMARY KEY, import_id TEXT NOT NULL REFERENCES imports(id), source_path TEXT NOT NULL, representation_id TEXT, state TEXT NOT NULL, error TEXT, staging_path TEXT, managed_path TEXT, source_hash TEXT, source_size INTEGER, source_modified TEXT, source_action TEXT);
      CREATE TABLE IF NOT EXISTS versions(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, kind TEXT NOT NULL, path TEXT NOT NULL, provider TEXT, prompt TEXT, recipe_json TEXT, source_hash TEXT, output_hash TEXT, state TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS edit_recipes(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, schema_version INTEGER NOT NULL, recipe_json TEXT NOT NULL, created_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS develop_recipes(asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS develop_presets(id TEXT PRIMARY KEY,name TEXT NOT NULL COLLATE NOCASE UNIQUE,schema_version INTEGER NOT NULL,preset_json TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS idx_develop_recipes_updated ON develop_recipes(updated_at DESC);
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
    migrations::apply_v6(&mut connection, existed, version)?;
    migrations::apply_v7(&mut connection, existed, version)?;
    migrations::apply_v8(&mut connection, existed, version)?;
    migrations::apply_v9(&mut connection, existed, version)?;
    migrations::apply_v10(&mut connection, existed, version)?;
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
        .query_map([], |row| { Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
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
    if let Some(configured) = std::env::var_os("KEEPFRAME_ANALYSIS_PYTHON")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Some(configured);
    }
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
        (
            "segmentation.py",
            include_str!("../../ai-worker/keepframe_worker/segmentation.py"),
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
fn segmentation_model_path() -> PathBuf {
    PathBuf::from(r"D:\AI Models\Keepframe\segmentation\beit-base-ade20k-640")
}
fn segmentation_model_installed() -> bool {
    let root = segmentation_model_path();
    root.join("config.json").is_file()
        && root.join("preprocessor_config.json").is_file()
        && fs::metadata(root.join("pytorch_model.bin"))
            .is_ok_and(|metadata| metadata.len() == SEGMENTATION_MODEL_BYTES)
        && fs::read_to_string(root.join("MODEL_SHA256.txt"))
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case(SEGMENTATION_MODEL_SHA256))
}
fn intelligent_mask_health_snapshot(
    loaded: bool,
    busy: bool,
    execution_provider: &str,
) -> IntelligentMaskHealth {
    let installed = segmentation_model_installed();
    let runtime_available = analysis_runtime_path().is_some();
    let available = installed && runtime_available;
    let detail = if !installed {
        "Intelligent masking model not installed. Manual masks remain available."
    } else if !runtime_available {
        "Model installed, but the local AI runtime is missing. Run scripts\\setup-ai-worker.ps1."
    } else if loaded {
        "Intelligent masking is loaded and ready."
    } else {
        "Intelligent masking is installed and will load locally when first used."
    };
    IntelligentMaskHealth {
        available,
        installed,
        runtime_available,
        loaded,
        busy,
        provider: SEGMENTATION_PROVIDER.into(),
        provider_version: SEGMENTATION_PROVIDER_VERSION.into(),
        model: SEGMENTATION_MODEL_ID.into(),
        model_revision: SEGMENTATION_MODEL_REVISION.into(),
        licence: "Apache-2.0".into(),
        source: format!(
            "https://huggingface.co/{SEGMENTATION_MODEL_ID}/tree/{SEGMENTATION_MODEL_REVISION}"
        ),
        approximate_bytes: SEGMENTATION_MODEL_BYTES,
        storage_path: segmentation_model_path().to_string_lossy().into(),
        execution_provider: execution_provider.into(),
        detail: detail.into(),
    }
}
fn start_analysis_worker(state: &AppState) {
    if state
        .analysis_worker
        .lock()
        .is_ok_and(|worker| worker.child.is_some())
    {
        return;
    }
    if !analysis_installed() && !segmentation_model_installed() {
        if let Ok(mut worker) = state.analysis_worker.lock() {
            worker.last_error = Some(analysis_installation_detail());
        }
        return;
    }
    let Some(worker_python) = analysis_runtime_path() else {
        if let Ok(mut worker) = state.analysis_worker.lock() {
            worker.last_error = Some(if segmentation_model_installed() {
                "The intelligent-masking model is installed, but the Keepframe Python runtime is missing. Run scripts\\setup-ai-worker.ps1.".into()
            } else {analysis_installation_detail()
            });
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
async fn get_service_health(force: Option<bool>, state: State<'_, AppState>,) -> Result<ServiceHealth> {
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

fn analysis_worker_endpoint(state: &AppState) -> Result<(String, String)> {
    start_analysis_worker(state);
    state
        .analysis_worker
        .lock()
        .map_err(|_| KeepframeError::Message("Local AI worker lock failed.".into()))
        .and_then(|worker| match (worker.url.clone(), worker.token.clone()) {
            (Some(url), Some(token)) => Ok((url, token)),
            _ => Err(KeepframeError::Message(
                worker.last_error.clone().unwrap_or_else(|| {
                    "The local intelligent-masking worker is unavailable.".into()
                }),
            )),
        })
}

#[tauri::command]
async fn get_intelligent_mask_health(state: State<'_, AppState>) -> Result<IntelligentMaskHealth> {
    if !segmentation_model_installed() || analysis_runtime_path().is_none() {
        return Ok(intelligent_mask_health_snapshot(
            false,
            false,
            "unavailable",
        ));
    }
    let (url, token) = analysis_worker_endpoint(&state)?;
    let client = reqwest::Client::new();
    for _ in 0..12 {
        if let Ok(response) = client
            .get(format!("{url}/health"))
            .header("X-Keepframe-Token", &token)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(value) = response.json::<Value>().await {
                    if let Some(segmentation) = value.get("segmentation") {
                        let loaded = segmentation
                            .get("loaded")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let busy = segmentation
                            .get("busy")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let provider = segmentation
                            .get("executionProvider")
                            .and_then(Value::as_str)
                            .unwrap_or("not loaded");
                        return Ok(intelligent_mask_health_snapshot(loaded, busy, provider));
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mut health = intelligent_mask_health_snapshot(false, false, "starting");
    health.available = false;
    health.detail = "The intelligent-masking worker is starting or failed its health check.".into();
    Ok(health)
}

#[tauri::command]
fn cancel_intelligent_mask(state: State<'_, AppState>) {
    state.mask_generation.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
fn cancel_intelligent_mask_install(state: State<'_, AppState>) {
    state.mask_install_cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn install_intelligent_mask_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<IntelligentMaskHealth> {
    if segmentation_model_installed() {
        return Ok(intelligent_mask_health_snapshot(false, false, "not loaded"));
    }
    state.mask_install_cancel.store(false, Ordering::SeqCst);
    let root = segmentation_model_path();
    fs::create_dir_all(&root)?;
    let files = [
        ("config.json", 6_966_u64, None),
        ("preprocessor_config.json", 276_u64, None),
        (
            "pytorch_model.bin",
            SEGMENTATION_MODEL_BYTES,
            Some(SEGMENTATION_MODEL_SHA256),
        ),
    ];
    let client = reqwest::Client::new();
    let mut overall = 0_u64;
    for (name, expected, hash) in files {
        let destination = root.join(name);
        let partial = root.join(format!("{name}.partial"));
        let url=format!("https://huggingface.co/{SEGMENTATION_MODEL_ID}/resolve/{SEGMENTATION_MODEL_REVISION}/{name}");
        let mut response = client.get(url).send().await?.error_for_status()?;
        let mut file = fs::File::create(&partial)?;
        let mut digest = Sha256::new();
        let mut received = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            if state.mask_install_cancel.load(Ordering::SeqCst) {
                drop(file);
                let _ = fs::remove_file(&partial);
                return Err(KeepframeError::Message(
                    "Intelligent masking model installation was cancelled.".into(),
                ));
            }
            file.write_all(&chunk)?;
            digest.update(&chunk);
            received += chunk.len() as u64;
            overall += chunk.len() as u64;
            let _=app.emit("intelligent-mask-install-progress",json!({"file":name,"receivedBytes":overall,"currentFileBytes":received,"fileBytes":expected,"totalBytes":SEGMENTATION_MODEL_BYTES+7_242}));
        }
        file.sync_all()?;
        drop(file);
        if received != expected {
            let _ = fs::remove_file(&partial);
            return Err(KeepframeError::Message(format!(
                "The downloaded {name} file was truncated."
            )));
        }
        if let Some(expected_hash) = hash {
            if format!("{:x}", digest.finalize()) != expected_hash {
                let _ = fs::remove_file(&partial);
                return Err(KeepframeError::Message(
                    "The downloaded model failed SHA-256 verification.".into(),
                ));
            }
        } else {
            serde_json::from_slice::<Value>(&fs::read(&partial)?)?;
        }
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&partial, &destination)?;
    }
    fs::write(
        root.join("MODEL_SHA256.txt"),
        format!("{SEGMENTATION_MODEL_SHA256}\n"),
    )?;
    Ok(intelligent_mask_health_snapshot(false, false, "not loaded"))
}

#[tauri::command]
async fn propose_intelligent_mask(
    asset_id: String,
    category: String,
    state: State<'_, AppState>,
) -> Result<IntelligentMaskProposal> {
    if !matches!(category.as_str(), "subject" | "people" | "sky") {
        return Err(KeepframeError::Message(
            "That intelligent mask category is unsupported.".into(),
        ));
    }
    if !segmentation_model_installed() {
        return Err(KeepframeError::Message(
            "Intelligent masking model not installed.".into(),
        ));
    }
    let request_id = state.mask_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let root = root_from(&state)?;
    let connection = open_db(&root)?;
    let preview: String = connection.query_row(
        "SELECT thumbnail_path FROM assets WHERE id=?1 AND trashed_at IS NULL",
        [&asset_id],
        |row| row.get(0),
    )?;
    drop(connection);
    let source = image::open(preview)?.thumbnail(640, 640);
    let analysis_width = source.width();
    let analysis_height = source.height();
    let png = encode_srgb_png(&image::DynamicImage::ImageRgb8(source.to_rgb8()))?;
    let (url, token) = analysis_worker_endpoint(&state)?;
    let started = Instant::now();
    let form = multipart::Form::new()
        .text("category", category.clone())
        .part(
            "image",
            multipart::Part::bytes(png)
                .file_name("analysis.png")
                .mime_str("image/png")?,
        );
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/segment"))
        .header("X-Keepframe-Token", token)
        .timeout(Duration::from_secs(300))
        .multipart(form)
        .send()
        .await?;
    if request_id != state.mask_generation.load(Ordering::SeqCst) || root_from(&state)? != root {
        return Err(KeepframeError::Message(
            "Intelligent mask request cancelled or superseded.".into(),
        ));
    }
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        let detail = response
            .json::<Value>()
            .await
            .ok()
            .and_then(|value| {
                value
                    .get("detail")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "No credible segmentation result was found.".into());
        return Err(KeepframeError::Message(detail));
    }
    let result = response
        .error_for_status()?
        .json::<WorkerSegmentationResult>()
        .await?;
    if result.width != analysis_width
        || result.height != analysis_height
        || !result.confidence.is_finite()
        || !(0.0..=1.0).contains(&result.confidence)
        || !result.coverage_fraction.is_finite()
        || !(0.0..=1.0).contains(&result.coverage_fraction)
    {
        return Err(KeepframeError::Message(
            "The segmentation provider returned invalid dimensions or confidence.".into(),
        ));
    }
    let coverage = decode_semantic_payload(&result.coverage_png, &result.checksum)?;
    if coverage.dimensions() != (result.width, result.height)
        || result.model != SEGMENTATION_MODEL_ID
        || result.model_revision != SEGMENTATION_MODEL_REVISION
        || result.model_sha256 != SEGMENTATION_MODEL_SHA256
    {
        return Err(KeepframeError::Message(
            "The segmentation provider returned incompatible model provenance.".into(),
        ));
    }
    let provenance = MaskProvenance {
        provider: result.provider,
        provider_version: result.provider_version,
        model: result.model,
        model_revision: result.model_revision,
        model_sha256: result.model_sha256,
        category: category.clone(),
        execution_provider: result.execution_provider,
    };
    let title = match category.as_str() {
        "subject" => "Subject",
        "people" => "People",
        _ => "Sky",
    };
    let mask = DevelopMask {
        id: Uuid::new_v4().to_string(),
        name: title.into(),
        enabled: true,
        inverted: false,
        opacity: 1.0,
        feather: 0.0,
        geometry: MaskGeometry::Semantic {
            width: result.width,
            height: result.height,
            coverage_png: result.coverage_png,
            checksum: result.checksum,
            provenance: Box::new(provenance),
            refinements: Vec::new(),
        },
        adjustments: LocalAdjustments::default(),
    };
    DevelopRecipe {
        schema_version: 2,
        settings: BasicAdjustments::neutral(),
        masks: vec![mask.clone()],
    }
    .validate()?;
    Ok(IntelligentMaskProposal {
        request_id,
        category,
        confidence: result.confidence,
        coverage_fraction: result.coverage_fraction,
        elapsed_ms: started.elapsed().as_millis(),
        mask,
        timings: result.timings,
    })
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
    let relocated = interoperability::recover_moved_managed_paths(&mut open_db(&root)?, &root)?;
    if relocated > 0 { append_runtime_log(&root, "managed_paths_rebased", &format!("{relocated} verified managed paths were rebased after opening this library."),); }
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
    restart_folder_watcher(&root, &state)?;
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
fn validate_ai_output(output: &Path, source_hash: &str, expected: Option<(u32, u32)>,) -> Result<String> {
    let metadata = fs::metadata(output).map_err(|_| KeepframeError::Message("The AI result file was not found.".into()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(KeepframeError::Message("The AI result is empty or incomplete.".into(),));
    }
    let image = image::open(output).map_err(|_| { KeepframeError::Message("The AI result is not a valid decodable image.".into())
    })?;
    let (width, height) = image.dimensions();
    if width < 64 || height < 64 {
        return Err(KeepframeError::Message("The AI result dimensions are implausibly small.".into(),));
    }
    if let Some((source_width, source_height)) = expected {
        let source_pixels = u64::from(source_width) * u64::from(source_height);
        let output_pixels = u64::from(width) * u64::from(height);
        if output_pixels < source_pixels / 100 || output_pixels > source_pixels.saturating_mul(100) {
            return Err(KeepframeError::Message("The AI result dimensions are implausible for this photograph.".into(),));
        }
    }
    let output_hash = hash_file(output)?;
    if output_hash == source_hash {
        return Err(KeepframeError::Message("The returned image is identical to the protected source; no edit was imported.".into(),));
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
        // Balanced remains lossless and avoids trading an excessive file-size
        // increase for the last few milliseconds of encoder throughput.
        encoder.set_compression(png::Compression::Balanced);
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
    // Local contrast is intentionally evaluated on a bounded working image.
    // It preserves the visual scale of the correction while avoiding a huge
    // Gaussian blur on every interactive full-size preview request.
    let largest = image.width().max(image.height());
    let blurred = if largest > 640 {
        let scale = 640.0 / largest as f32;
        let width = (image.width() as f32 * scale).round().max(1.0) as u32;
        let height = (image.height() as f32 * scale).round().max(1.0) as u32;
        let working = image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle);
        let small_blur = image::imageops::blur(&working, (sigma * scale).max(1.2));
        image::imageops::resize(&small_blur, image.width(), image.height(), image::imageops::FilterType::Triangle,)
    } else {
        image::imageops::blur(image, sigma)
    };
    image.as_mut().par_chunks_mut(3).zip(blurred.as_raw().par_chunks(3)).for_each(|(pixel, blurred_pixel)| {
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
    });
}

fn apply_colour_adjustments_to_image(
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
    output.as_mut().par_chunks_mut(3).for_each(|pixel| {
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
    });
    apply_protected_local_contrast(&mut output, adjustments.texture / 100.0 * 0.28, 220.0);
    apply_protected_local_contrast(&mut output, adjustments.clarity / 100.0 * 0.42, 75.0);
    apply_protected_local_contrast(&mut output, dehaze * 0.18, 48.0);
    output
}

fn apply_adjustments_to_image(image: &image::DynamicImage, adjustments: BasicAdjustments,) -> RgbImage {
    let colour = apply_colour_adjustments_to_image(image, adjustments);
    apply_geometry(&colour, adjustments)
}

fn segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let dx = end.0 - start.0; let dy = end.1 - start.1; let length_squared = dx * dx + dy * dy;
    let t = (if length_squared <= f32::EPSILON { 0.0 } else { ((point.0-start.0)*dx+(point.1-start.1)*dy)/length_squared }).clamp(0.0,1.0);
    ((point.0-(start.0+dx*t)).powi(2)+(point.1-(start.1+dy*t)).powi(2)).sqrt()
}

fn brush_stroke_alpha(stroke: &BrushStroke, x: u32, y: u32, width: u32, height: u32) -> f32 {
    let smallest = width.min(height).max(1) as f32;
    let point = (x as f32, y as f32);
    let radius = stroke.radius * smallest;
    let distance = if stroke.points.len() == 1 {
        let p = stroke.points[0];
        segment_distance(
            point,
            (p.x * width as f32, p.y * height as f32),
            (p.x * width as f32, p.y * height as f32),
        )
    } else {
        stroke
            .points
            .windows(2)
            .map(|pair| {
                segment_distance(
                    point,
                    (pair[0].x * width as f32, pair[0].y * height as f32),
                    (pair[1].x * width as f32, pair[1].y * height as f32),
                )
            })
            .fold(f32::INFINITY, f32::min)
    };
    (1.0 - smoothstep(
        radius * (1.0 - stroke.feather),
        radius.max(0.000_001),
        distance,
    )) * stroke.flow
}

fn prepared_semantic_coverage(
    mask: &DevelopMask,
    width: u32,
    height: u32,
) -> Result<Option<image::GrayImage>> {
    let MaskGeometry::Semantic {
        coverage_png,
        checksum,
        ..
    } = &mask.geometry
    else {
        return Ok(None);
    };
    let source = decode_semantic_payload(coverage_png, checksum)?;
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::CatmullRom,
    );
    let sigma = (mask.feather * 0.005 * width.min(height) as f32).min(40.0);
    Ok(Some(if sigma >= 0.1 {
        image::imageops::blur(&resized, sigma)
    } else {
        resized
    }))
}

fn digest_json(value: &impl Serialize) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).expect("validated renderer values serialise")))
}

fn semantic_cache_key(mask: &DevelopMask, width: u32, height: u32) -> Option<String> {
    let MaskGeometry::Semantic { checksum, .. } = &mask.geometry else { return None; };
    Some(format!("{checksum}:{width}x{height}:{:08x}", mask.feather.to_bits()))
}

fn prepared_semantic_coverage_cached(
    mask: &DevelopMask,
    width: u32,
    height: u32,
    caches: &mut RendererCaches,
) -> Result<Option<Arc<image::GrayImage>>> {
    let Some(key) = semantic_cache_key(mask, width, height) else { return Ok(None); };
    if let Some(coverage) = caches.semantics.get(&key) { return Ok(Some(coverage)); }
    let coverage = Arc::new(prepared_semantic_coverage(mask, width, height)?.expect("semantic key requires semantic geometry"));
    caches.semantics.insert(key, Arc::clone(&coverage), width as usize * height as usize);
    Ok(Some(coverage))
}

fn mask_cache_key(mask: &DevelopMask, width: u32, height: u32) -> String {
    #[derive(Serialize)]
    struct CoverageIdentity<'a> {
        geometry: &'a MaskGeometry,
        inverted: bool,
        opacity: f32,
        feather: f32,
        width: u32,
        height: u32,
    }
    digest_json(&CoverageIdentity { geometry: &mask.geometry, inverted: mask.inverted, opacity: mask.opacity, feather: mask.feather, width, height })
}

fn mask_coverage_prepared(
    mask: &DevelopMask,
    semantic: Option<&image::GrayImage>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,) -> f32 {
    let nx=if width>1{x as f32/(width-1) as f32}else{0.0}; let ny=if height>1{y as f32/(height-1) as f32}else{0.0};
    let mut coverage=match &mask.geometry {
        MaskGeometry::Linear{start,end}=>{let dx=end.x-start.x;let dy=end.y-start.y;let t=((nx-start.x)*dx+(ny-start.y)*dy)/(dx*dx+dy*dy).max(0.000_001);let half=mask.feather.max(0.001)*0.5;smoothstep(0.5-half,0.5+half,t)}
        MaskGeometry::Radial{centre,radius_x,radius_y,rotation,}=>{let(sin,cos)=rotation.to_radians().sin_cos();let dx=nx-centre.x;let dy=ny-centre.y;let rx=(cos*dx+sin*dy)/radius_x.max(0.000_001);let ry=(-sin*dx+cos*dy)/radius_y.max(0.000_001);1.0-smoothstep((1.0-mask.feather).clamp(0.0,0.999),1.0,(rx*rx+ry*ry).sqrt(),)}
        MaskGeometry::Brush{strokes}=>{let mut painted=0.0_f32;for stroke in strokes{let alpha= brush_stroke_alpha(stroke, x, y, width, height);painted=if stroke.erase{painted*(1.0-alpha)}else{painted+(1.0-painted)*alpha};}painted}
        MaskGeometry::Semantic { refinements, .. } => {
            let mut value = semantic
                .map(|coverage| coverage.get_pixel(x, y)[0] as f32 / 255.0)
                .unwrap_or(0.0);
            for stroke in refinements {
                let alpha = brush_stroke_alpha(stroke, x, y, width, height);
                value = if stroke.erase {
                    value * (1.0 - alpha)
                } else {
                    value + (1.0 - value) * alpha
                };
            }
            value
        }
    };
    if mask.inverted { coverage=1.0-coverage; }
    (coverage*mask.opacity).clamp(0.0,1.0)
}

fn prepared_mask_coverage(
    mask: &DevelopMask,
    width: u32,
    height: u32,
    semantic: Option<&image::GrayImage>,
) -> Vec<f32> {
    let count = width as usize * height as usize;
    (0..count).into_par_iter().map(|index| {
        let x = index as u32 % width;
        let y = index as u32 / width;
        mask_coverage_prepared(mask, semantic, x, y, width, height)
    }).collect()
}

fn prepared_mask_coverage_cached(
    mask: &DevelopMask,
    width: u32,
    height: u32,
    caches: &mut RendererCaches,
) -> Result<Arc<Vec<f32>>> {
    let key = mask_cache_key(mask, width, height);
    if let Some(coverage) = caches.masks.get(&key) { return Ok(coverage); }
    let semantic = prepared_semantic_coverage_cached(mask, width, height, caches)?;
    let coverage = Arc::new(prepared_mask_coverage(mask, width, height, semantic.as_deref()));
    caches.masks.insert(key, Arc::clone(&coverage), coverage.len() * std::mem::size_of::<f32>());
    Ok(coverage)
}

#[cfg(test)]
fn mask_coverage(mask: &DevelopMask, x: u32, y: u32, width: u32, height: u32) -> f32 {
    let semantic = prepared_semantic_coverage(mask, width, height)
        .ok()
        .flatten();
    mask_coverage_prepared(mask, semantic.as_ref(), x, y, width, height)
}

fn render_develop_recipe(image: &image::DynamicImage, recipe: &DevelopRecipe) -> RgbImage {
    // Deliberate order: EXIF-oriented source -> global colour/tone -> ordered local masks -> transform/crop.
    let mut output=apply_colour_adjustments_to_image(image,recipe.settings);let(width,height)=output.dimensions();
    for mask in recipe.masks.iter().filter(|mask|mask.enabled&&mask.opacity>0.0){
        let semantic = prepared_semantic_coverage(mask, width, height).ok().flatten();
        let coverage = prepared_mask_coverage(mask,width,height,semantic.as_ref());
        let adjusted=apply_colour_adjustments_to_image(&image::DynamicImage::ImageRgb8(output.clone()),mask.adjustments.as_basic(),);
        output.as_mut().par_chunks_mut(3).zip(adjusted.as_raw().par_chunks(3)).zip(coverage.par_iter()).for_each(|((pixel,target),alpha)| { if *alpha>0.0 { for channel in 0..3 { pixel[channel]=(pixel[channel] as f32+(target[channel] as f32-pixel[channel] as f32)*alpha).round().clamp(0.0,255.0) as u8; } } });
    }
    apply_geometry(&output,recipe.settings)
}

fn colour_identity(mut settings: BasicAdjustments) -> BasicAdjustments {
    settings.crop_left=0.0; settings.crop_top=0.0; settings.crop_width=1.0; settings.crop_height=1.0;
    settings.rotate_quadrants=0; settings.straighten=0.0; settings.horizontal_flip=false; settings.vertical_flip=false;
    settings
}

fn render_develop_recipe_cached(
    image: &image::DynamicImage,
    source_key: &str,
    recipe: &DevelopRecipe,
    caches: &Mutex<RendererCaches>,
    generation: Option<(&AtomicU64,u64)>,
) -> Result<RgbImage> {
    let cancelled=||generation.is_some_and(|(current,requested)|current.load(Ordering::Acquire)!=requested);
    if cancelled(){return Err(KeepframeError::Message("Superseded Develop preview.".into()));}
    let global_key=format!("{source_key}:{}",digest_json(&colour_identity(recipe.settings)));
    let cached_global=caches.lock().expect("renderer cache lock").intermediates.get(&global_key);
    let mut output=if let Some(global)=cached_global{(*global).clone()}else{
        let rendered=apply_colour_adjustments_to_image(image,recipe.settings);
        caches.lock().expect("renderer cache lock").intermediates.insert(global_key,Arc::new(rendered.clone()),rendered.as_raw().len());
        rendered
    };
    if cancelled(){return Err(KeepframeError::Message("Superseded Develop preview.".into()));}
    let(width,height)=output.dimensions();
    for mask in recipe.masks.iter().filter(|mask|mask.enabled&&mask.opacity>0.0){
        let coverage=prepared_mask_coverage_cached(mask,width,height,&mut caches.lock().expect("renderer cache lock"))?;
        let adjusted=apply_colour_adjustments_to_image(&image::DynamicImage::ImageRgb8(output.clone()),mask.adjustments.as_basic());
        output.as_mut().par_chunks_mut(3).zip(adjusted.as_raw().par_chunks(3)).zip(coverage.par_iter()).for_each(|((pixel,target),alpha)|{if *alpha>0.0{for channel in 0..3{pixel[channel]=(pixel[channel]as f32+(target[channel]as f32-pixel[channel]as f32)*alpha).round().clamp(0.0,255.0)as u8;}}});
        if cancelled(){return Err(KeepframeError::Message("Superseded Develop preview.".into()));}
    }
    Ok(apply_geometry(&output,recipe.settings))
}

fn decoded_source_key(path:&Path,width:u32,height:u32)->Result<String>{
    let metadata=fs::metadata(path)?;
    let modified=metadata.modified().ok().and_then(|value|value.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
    Ok(format!("{}:{}:{}:{}:{}x{}",path.to_string_lossy(),metadata.len(),modified.as_secs(),modified.subsec_nanos(),width,height))
}

fn decode_preview_cached(path:&Path,caches:&Mutex<RendererCaches>)->Result<(String,Arc<image::DynamicImage>)>{
    let key=decoded_source_key(path,720,540)?;
    if let Some(image)=caches.lock().expect("renderer cache lock").decoded.get(&key){return Ok((key,image));}
    let image=Arc::new(image::open(path)?.thumbnail(720,540));
    let bytes=image.width()as usize*image.height()as usize*4;
    caches.lock().expect("renderer cache lock").decoded.insert(key.clone(),Arc::clone(&image),bytes);
    Ok((key,image))
}

fn apply_geometry(source: &RgbImage, adjustments: BasicAdjustments) -> RgbImage {
    let mut image = source.clone();
    if adjustments.horizontal_flip {
        image = image::imageops::flip_horizontal(&image);
    }
    if adjustments.vertical_flip {
        image = image::imageops::flip_vertical(&image);
    }
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

fn original_adjustment_input(connection: &Connection, asset_id: &str) -> Result<AdjustmentInput> {
    let (captured, width, height): (String, Option<u32>, Option<u32>) = connection.query_row(
        "SELECT captured_at,width,height FROM assets WHERE id=?1 AND trashed_at IS NULL",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let (path, source_hash): (String, String) = connection.query_row(
        "SELECT path,sha256 FROM representations WHERE asset_id=?1 ORDER BY CASE WHEN lower(extension) IN ('jpg','jpeg','png','tif','tiff') THEN 0 WHEN is_raw=1 THEN 1 ELSE 2 END,path LIMIT 1",
        [asset_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(AdjustmentInput { path: PathBuf::from(path), source_hash, captured_at: captured, expected_dimensions: width.zip(height), })
}

fn image_statistics(image: &image::DynamicImage) -> ImageStatistics {
    let rgb = image.to_rgb8();
    let step = ((rgb.width() as usize * rgb.height() as usize) / 250_000).max(1);
    let mut luminance_bins = vec![0_u64; 64]; let mut red_bins = vec![0_u64; 64]; let mut green_bins = vec![0_u64; 64]; let mut blue_bins = vec![0_u64; 64];
    let mut luminances = Vec::new(); let mut saturation_total = 0.0_f32; let mut channel_total = [0.0_f32; 3]; let mut shadows = 0_u64; let mut highlights = 0_u64;
    for pixel in rgb.pixels().step_by(step) {
        let channels = [pixel[0] as f32 / 255.0, pixel[1] as f32 / 255.0, pixel[2] as f32 / 255.0,];
        let luminance = channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
        luminance_bins[(luminance * 63.0).round().clamp(0.0, 63.0) as usize] += 1;
        red_bins[(channels[0] * 63.0).round() as usize] += 1; green_bins[(channels[1] * 63.0).round() as usize] += 1; blue_bins[(channels[2] * 63.0).round() as usize] += 1;
        let maximum = channels.into_iter().fold(0.0_f32, f32::max); let minimum = channels.into_iter().fold(1.0_f32, f32::min);
        saturation_total += if maximum > 0.0 { (maximum - minimum) / maximum } else { 0.0 };
        channel_total[0] += channels[0]; channel_total[1] += channels[1]; channel_total[2] += channels[2];
        if luminance <= 0.02 { shadows += 1; }
        if luminance >= 0.98 { highlights += 1; }
        luminances.push(luminance);
    }
    luminances.sort_by(|left, right| left.total_cmp(right)); let samples = luminances.len().max(1);
    let percentile = |fraction: f32| { luminances.get(((samples - 1) as f32 * fraction).round() as usize).copied().unwrap_or(0.5)
    };
    ImageStatistics { luminance_bins, red_bins, green_bins, blue_bins, samples: samples as u64, average_luminance: luminances.iter().sum::<f32>() / samples as f32, p01: percentile(0.01), p50: percentile(0.50), p99: percentile(0.99), shadow_clip_fraction: shadows as f32 / samples as f32, highlight_clip_fraction: highlights as f32 / samples as f32, average_saturation: saturation_total / samples as f32, red_green_blue: [channel_total[0] / samples as f32, channel_total[1] / samples as f32, channel_total[2] / samples as f32,], dynamic_range: (percentile(0.99) - percentile(0.01)).max(0.0), }
}

fn auto_proposal(image: &image::DynamicImage, current: BasicAdjustments) -> AutoProposal {
    let stats = image_statistics(image); let mut settings = current; let mut explanation = Vec::new();
    // Auto analyses the source-derived, bounded thumbnail and adds restrained
    // deltas to the current recipe. It deliberately never changes geometry.
    let exposure_delta = ((0.46 / stats.p50.max(0.08)).log2()).clamp(-0.35, 0.65);
    if exposure_delta.abs() >= 0.08 { settings.exposure = (current.exposure + exposure_delta).clamp(-3.0, 3.0); explanation.push(if exposure_delta > 0.0 { "Exposure increased because midtones were under-represented.".into() } else { "Exposure reduced because midtones were already bright.".into() }); }
    if stats.highlight_clip_fraction > 0.004 || stats.p99 > 0.94 { settings.highlights = (current.highlights - 18.0).max(-45.0); explanation.push("Highlights reduced because clipping was detected.".into()); }
    if stats.shadow_clip_fraction > 0.012 || stats.p01 < 0.07 { settings.shadows = (current.shadows + 14.0).min(35.0); explanation.push("Shadows raised moderately because dark detail was limited.".into()); }
    if stats.dynamic_range < 0.55 { settings.contrast = (current.contrast + 7.0).min(20.0); explanation.push("Contrast raised slightly because the tonal range was flat.".into()); }
    if stats.average_saturation < 0.18 { settings.colour_boost = (current.colour_boost + 8.0).min(18.0); explanation.push("Vibrance increased conservatively because average saturation was low.".into()); }
    let neutral_spread = (stats.red_green_blue[0] - stats.red_green_blue[1]).abs().max((stats.red_green_blue[2] - stats.red_green_blue[1]).abs());
    let wb_confidence = (1.0 - neutral_spread * 8.0).clamp(0.0, 1.0);
    if wb_confidence >= 0.58 && stats.average_saturation < 0.42 {
        let temperature = ((stats.red_green_blue[2] - stats.red_green_blue[0]) * 55.0).clamp(-12.0, 12.0);
        if temperature.abs() >= 2.0 { settings.light_balance = (current.light_balance + temperature).clamp(-100.0, 100.0); explanation.push(if temperature > 0.0 { "White balance warmed using a low-saturation grey-world estimate.".into() } else { "White balance cooled using a low-saturation grey-world estimate.".into() }); }
    } else { explanation.push("White balance unchanged because no reliable neutral estimate was found.".into()); }
    if explanation.is_empty() { explanation.push("The image already falls within conservative Auto thresholds; no changes are proposed.".into(),); }
    let mut recommendations = Vec::new();
    if stats.average_saturation > 0.24 && stats.dynamic_range > 0.55 { recommendations.push("Landscape".into()); }
    if stats.average_saturation < 0.22 && stats.p50 > 0.38 && stats.p50 < 0.70 { recommendations.push("Portrait".into()); }
    if stats.dynamic_range < 0.45 { recommendations.push("Soft Contrast".into()); }
    AutoProposal { settings, explanation, confidence: ((wb_confidence + (1.0 - stats.highlight_clip_fraction * 8.0).clamp(0.0, 1.0)) / 2.0).clamp(0.25, 1.0), statistics: stats, recommendations, }
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
        luminances.push((pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722) / 255.0,);
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
        horizontal_flip: false,
        vertical_flip: false,
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
        // A bounded preview keeps slider feedback independent of the size of
        // the cached browse image. Full-resolution export uses a separate path.
        let image = image::open(preview)?.thumbnail(720, 540);
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

fn develop_recipe_in(connection: &Connection, asset_id: &str) -> Result<DevelopRecipe> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assets WHERE id=?1 AND trashed_at IS NULL)",
        [asset_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(KeepframeError::Message("That photograph is no longer available for Develop.".into(),));
    }
    let recipe_json: Option<String> = connection.query_row(
        "SELECT recipe_json FROM develop_recipes WHERE asset_id=?1",
        [asset_id],
        |row| row.get(0),
    ).optional()?;
    match recipe_json {
        Some(json) => serde_json::from_str::<DevelopRecipe>(&json)
            .map_err(KeepframeError::from)?
            .validate(),
        None => Ok(DevelopRecipe::neutral()),
    }
}

#[tauri::command]
fn get_develop_recipe(asset_id: String, state: State<'_, AppState>) -> Result<DevelopRecipe> {
    let connection = open_db(&root_from(&state)?)?;
    develop_recipe_in(&connection, &asset_id)
}

#[tauri::command]
fn save_develop_recipe(asset_id: String, recipe: DevelopRecipe, state: State<'_, AppState>,) -> Result<bool> {
    let recipe = recipe.validate()?;
    let mut connection = open_db(&root_from(&state)?)?;
    let _ = develop_recipe_in(&connection, &asset_id)?;
    let tx = connection.transaction()?;
    if recipe.is_edited() {
        tx.execute(
            "INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES(?1,?2,?3,?4) ON CONFLICT(asset_id) DO UPDATE SET schema_version=excluded.schema_version,recipe_json=excluded.recipe_json,updated_at=excluded.updated_at",
            params![asset_id, recipe.schema_version, serde_json::to_string(&recipe)?, Utc::now().to_rfc3339()],
        )?;
    } else {
        tx.execute("DELETE FROM develop_recipes WHERE asset_id=?1", [&asset_id])?;
    }
    tx.commit()?;
    Ok(recipe.is_edited())
}

#[tauri::command]
async fn preview_develop_recipe(asset_id: String, recipe: DevelopRecipe, state: State<'_, AppState>,) -> Result<String> {
    let recipe = recipe.validate()?;
    let root = root_from(&state)?;
    let generation_counter=Arc::clone(&state.preview_generation);
    let generation=generation_counter.fetch_add(1,Ordering::AcqRel)+1;
    let caches=Arc::clone(&state.renderer_caches);
    tauri::async_runtime::spawn_blocking(move || -> Result<String> {
        let connection = open_db(&root)?;
        let _ = develop_recipe_in(&connection, &asset_id)?;
        let preview: String = connection.query_row(
            "SELECT thumbnail_path FROM assets WHERE id=?1 AND trashed_at IS NULL", [&asset_id], |row| row.get(0),
        )?;
        // Keep interactive work bounded even when the browsing thumbnail cache
        // was built at a larger size. Export always decodes the original.
        let (source_key,image)=decode_preview_cached(Path::new(&preview),&caches)?;
        let adjusted=image::DynamicImage::ImageRgb8(render_develop_recipe_cached(&image,&source_key,&recipe,&caches,Some((&generation_counter,generation)))?);
        if generation_counter.load(Ordering::Acquire)!=generation{return Err(KeepframeError::Message("Superseded Develop preview.".into()));}
        let mut digest = Sha256::new();
        digest.update(b"keepframe-develop-preview-v3");
        digest.update(asset_id.as_bytes());
        digest.update(source_key.as_bytes());
        digest.update(serde_json::to_vec(&recipe)?);
        let key = format!("{:x}", digest.finalize());
        let output = root.join(".keepframe/previews").join(format!("develop-{}-{}.png", asset_id, &key[..16]));
        if !output.exists() {
            let temporary = output.with_extension("png.tmp");
            fs::write(&temporary, encode_srgb_png(&adjusted)?)?;
            fs::rename(&temporary, &output)?;
        }
        Ok(output.to_string_lossy().into())
    }).await.map_err(|error| KeepframeError::Message(format!("Develop preview failed: {error}")))?
}

#[tauri::command]
fn list_develop_presets(state: State<'_, AppState>) -> Result<Vec<DevelopPreset>> {
    let connection = open_db(&root_from(&state)?)?;
    let mut presets = built_in_presets();
    let mut statement = connection.prepare("SELECT preset_json FROM develop_presets ORDER BY name COLLATE NOCASE")?;
    let user_presets = statement.query_map([], |row| row.get::<_, String>(0))?
        .map(|row| { row.map_err(KeepframeError::from).and_then(|json| { serde_json::from_str::<DevelopPreset>(&json).map_err(KeepframeError::from)?.validate(true)
            })
        })
        .collect::<Result<Vec<_>>>()?;
    presets.extend(user_presets); Ok(presets)
}

#[tauri::command]
fn save_develop_preset(preset: DevelopPreset, state: State<'_, AppState>) -> Result<DevelopPreset> {
    let mut preset = preset.validate(true)?; preset.built_in = false;
    let now = Utc::now().to_rfc3339(); let mut connection = open_db(&root_from(&state)?)?; let tx = connection.transaction()?;
    tx.execute("INSERT INTO develop_presets(id,name,schema_version,preset_json,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name,schema_version=excluded.schema_version,preset_json=excluded.preset_json,updated_at=excluded.updated_at", params![preset.id,preset.name,preset.schema_version,serde_json::to_string(&preset)?,now])?;
    tx.commit()?; Ok(preset)
}

#[tauri::command]
fn delete_develop_preset(id: String, state: State<'_, AppState>) -> Result<()> {
    if built_in_presets().iter().any(|preset| preset.id == id) { return Err(KeepframeError::Message("Built-in presets cannot be deleted.".into(),)); }
    let connection = open_db(&root_from(&state)?)?; connection.execute("DELETE FROM develop_presets WHERE id=?1", [id])?; Ok(())
}

#[tauri::command]
fn export_develop_preset(id: String, path: String, state: State<'_, AppState>) -> Result<()> {
    let preset = list_develop_presets(state)?.into_iter().find(|preset| preset.id == id).ok_or_else(|| KeepframeError::Message("The preset no longer exists.".into()))?;
    let path = PathBuf::from(path); if !path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("keepframe-preset")).unwrap_or(false) { return Err(KeepframeError::Message("Preset exports must use the .keepframe-preset extension.".into(),)); }
    let file = PresetFile { schema_version: 1, name: preset.name, categories: preset.categories, settings: preset.settings, };
    fs::write(path, serde_json::to_vec_pretty(&file)?)?; Ok(())
}

#[tauri::command]
fn import_develop_preset(path: String, state: State<'_, AppState>) -> Result<DevelopPreset> {
    let path = PathBuf::from(path); let metadata = fs::metadata(&path)?;
    if metadata.len() > PRESET_FILE_MAX_BYTES { return Err(KeepframeError::Message("Preset file is larger than the 128 KiB safety limit.".into(),)); }
    let file: PresetFile = serde_json::from_slice(&fs::read(path)?)?;
    let preset = DevelopPreset { schema_version: file.schema_version, id: Uuid::new_v4().to_string(), name: file.name, categories: file.categories, settings: file.settings, built_in: false, };
    save_develop_preset(preset, state)
}

#[tauri::command]
async fn propose_develop_auto(asset_id: String, current: BasicAdjustments, state: State<'_, AppState>,) -> Result<AutoProposal> {
    let current = current.validate()?; let root = root_from(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<AutoProposal> {
        let connection = open_db(&root)?; let preview: String = connection.query_row("SELECT thumbnail_path FROM assets WHERE id=?1 AND trashed_at IS NULL", [&asset_id], |row| row.get(0),)?;
        let image = image::open(preview)?.thumbnail(720, 540); Ok(auto_proposal(&image, current))
    }).await.map_err(|error| { KeepframeError::Message(format!("Automatic Develop analysis failed: {error}"))
    })?
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
            "Exposure {:+0.2} EV; temperature {:+0.0}; tint {:+0.0}; contrast {:+0.0}; highlights {:+0.0}; shadows {:+0.0}; whites {:+0.0}; blacks {:+0.0}; texture {:+0.0}; clarity {:+0.0}; dehaze {:+0.0}; vibrance {:+0.0}; saturation {:+0.0}",
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
                        managed_path.as_deref().ok_or_else(|| { KeepframeError::Message(
                            "No verified managed copy was available; the source was retained.".into(),
                        )
                        })?,
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
         COALESCE(a.location_source,CASE WHEN a.latitude IS NULL OR a.longitude IS NULL THEN 'none' ELSE 'embedded' END),
         COALESCE(a.missing_state,'available'),
         EXISTS(SELECT 1 FROM develop_recipes dr WHERE dr.asset_id=a.id)
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
            missing_state: row.get(16)?,
            has_edits: row.get::<_, i64>(17)? != 0,
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
         COALESCE(a.location_source,CASE WHEN a.latitude IS NULL OR a.longitude IS NULL THEN 'none' ELSE 'embedded' END),
         COALESCE(a.missing_state,'available'),
         EXISTS(SELECT 1 FROM develop_recipes dr WHERE dr.asset_id=a.id)
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
            tags: if tag_string.is_empty() { Vec::new() } else { tag_string.split('\u{1f}').map(str::to_string).collect() }, location_source: row.get(15)?, missing_state: row.get(16)?, has_edits: row.get::<_, i64>(17)? != 0,
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

#[tauri::command]
fn export_xmp_sidecars(asset_ids: Vec<String>, replace_existing: bool, state: State<'_, AppState>,) -> Result<interoperability::SidecarExportSummary> {
    let mut connection = open_db(&root_from(&state)?)?;
    interoperability::export_sidecars(&mut connection, &asset_ids, replace_existing)
}

#[tauri::command]
fn import_xmp_sidecar(asset_id: String, state: State<'_, AppState>,) -> Result<interoperability::SidecarImportResult> {
    let mut connection = open_db(&root_from(&state)?)?;
    interoperability::import_sidecar(&mut connection, &asset_id)
}

#[tauri::command]
fn export_portable_catalogue(destination: String, state: State<'_, AppState>) -> Result<String> {
    let root = root_from(&state)?;
    let connection = open_db(&root)?;
    Ok(interoperability::export_portable_catalogue(&connection, &root, Path::new(&destination))?.to_string_lossy().into(),)
}

#[tauri::command]
fn rescan_library(state: State<'_, AppState>) -> Result<interoperability::IntegrityReport> {
    let root = root_from(&state)?;
    let mut connection = open_db(&root)?;
    interoperability::rescan_library(&mut connection, &root)
}

#[tauri::command]
fn find_relink_candidates(asset_id: String, directory: String, state: State<'_, AppState>,) -> Result<Vec<interoperability::RelinkCandidate>> {
    let connection = open_db(&root_from(&state)?)?;
    interoperability::relink_candidates(&connection, &asset_id, Path::new(&directory))
}

#[tauri::command]
fn relink_asset(asset_id: String, path: String, state: State<'_, AppState>) -> Result<()> {
    let mut connection = open_db(&root_from(&state)?)?;
    interoperability::relink_asset(&mut connection, &asset_id, Path::new(&path))
}

fn watch_kind(kind: &EventKind) -> Option<&'static str> {
    match kind {
        EventKind::Create(_) => Some("new_file"),
        EventKind::Modify(_) => Some("file_changed"),
        EventKind::Remove(_) => Some("file_removed"),
        _ => None,
    }
}

fn write_debounced_watch_event(root: &Path, folder: &str, path: &Path, kind: &str) {
    let Ok(connection) = open_db(root) else { return; };
    let _ = connection.execute(
        "INSERT INTO watch_events(folder_path,path,kind,observed_at,state)VALUES(?1,?2,?3,?4,'inbox') ON CONFLICT(folder_path,path,kind,state) DO UPDATE SET observed_at=excluded.observed_at",
        params![folder,path.to_string_lossy(),kind,Utc::now().to_rfc3339()],
    );
}

fn restart_folder_watcher(root: &Path, state: &AppState) -> Result<()> {
    let connection = open_db(root)?;
    let mut statement = connection.prepare("SELECT path FROM watch_folders WHERE enabled=1 ORDER BY path")?;
    let folders = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter().map(PathBuf::from).filter(|path| path.is_dir()).collect::<Vec<_>>();
    drop(statement);
    let mut watcher_slot = state.folder_watcher.lock().map_err(|_| KeepframeError::Message("Folder watch state lock failed".into()))?;
    *watcher_slot = None;
    if folders.is_empty() { return Ok(()); }
    let (sender, receiver) = mpsc::channel::<(String, PathBuf, String)>();
    let watched_roots = folders.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return; };
        let Some(kind) = watch_kind(&event.kind) else { return; };
        for path in event.paths {
            if let Some(folder) = watched_roots.iter().find(|folder| path.starts_with(folder)) {
                let _ = sender.send((folder.to_string_lossy().into(), path, kind.into()));
            }
        }
    }).map_err(|error| KeepframeError::Message(format!("Folder watch could not start: {error}")))?;
    for folder in &folders { watcher.watch(folder, RecursiveMode::Recursive).map_err(|error| { KeepframeError::Message(format!("Could not watch {}: {error}", folder.display()))
            })?; }
    let root_for_thread = root.to_path_buf();
    std::thread::spawn(move || {
        let mut pending: HashMap<(String, PathBuf, String), Instant> = HashMap::new();
        loop {
            match receiver.recv_timeout(Duration::from_millis(150)) {
                Ok((folder, path, kind)) => { pending.insert((folder, path, kind), Instant::now()); }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let due = pending.iter().filter(|(_, at)| at.elapsed() >= Duration::from_millis(600)).map(|(key, _)| key.clone()).collect::<Vec<_>>();
            for (folder, path, kind) in due { pending.remove(&(folder.clone(), path.clone(), kind.clone())); write_debounced_watch_event(&root_for_thread, &folder, &path, &kind); }
        }
    });
    *watcher_slot = Some(FolderWatcher { _watcher: watcher });
    Ok(())
}

#[tauri::command]
fn configure_folder_watch(path: String, enabled: bool, state: State<'_, AppState>) -> Result<()> {
    let root = root_from(&state)?;
    let folder = PathBuf::from(path).canonicalize().map_err(|_| { KeepframeError::Message("Choose an existing source folder to watch.".into())
    })?;
    if !folder.is_dir() || folder.parent().is_none() { return Err(KeepframeError::Message("Keepframe will not watch a drive root or system-wide location.".into(),)); }
    let connection = open_db(&root)?;
    connection.execute("INSERT INTO watch_folders(path,enabled,created_at)VALUES(?1,?2,?3) ON CONFLICT(path) DO UPDATE SET enabled=excluded.enabled", params![folder.to_string_lossy(),enabled as i64,Utc::now().to_rfc3339()])?;
    restart_folder_watcher(&root, &state)
}

#[tauri::command]
fn disable_folder_watches(state: State<'_, AppState>) -> Result<()> {
    let root = root_from(&state)?;
    open_db(&root)?.execute("UPDATE watch_folders SET enabled=0", [])?;
    restart_folder_watcher(&root, &state)
}

#[tauri::command]
fn list_folder_watch_events(state: State<'_, AppState>) -> Result<Vec<WatchEventRecord>> {
    let connection = open_db(&root_from(&state)?)?;
    let mut statement = connection.prepare("SELECT id,folder_path,path,kind,observed_at FROM watch_events WHERE state='inbox' ORDER BY observed_at DESC,id DESC LIMIT 250")?;
    let events = statement.query_map([], |row| { Ok(WatchEventRecord { id: row.get(0)?,folder_path: row.get(1)?,path: row.get(2)?,kind: row.get(3)?,observed_at: row.get(4)?, })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(events)
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

fn update_locations_in(connection: &mut Connection, asset_ids: &[String], latitude: f64, longitude: f64,) -> Result<usize> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(KeepframeError::Message("Coordinates are outside the valid range".into(),));
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
fn update_locations(asset_ids: Vec<String>, latitude: f64, longitude: f64, state: State<'_, AppState>,) -> Result<usize> {
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

#[tauri::command]
async fn start_export_batch(
    request: professional_export::ExportRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<professional_export::ExportBatchReport> {
    let root = root_from(&state)?;
    let caches = Arc::clone(&state.renderer_caches);
    let generation = Arc::clone(&state.export_generation);
    let requested_generation = generation.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        professional_export::run_batch(
            &root,
            request,
            &caches,
            &generation,
            requested_generation,
            Some(&app),
        )
    })
    .await
    .map_err(|error| KeepframeError::Message(format!("Export task failed: {error}")))?
}

#[tauri::command]
fn cancel_export_batch(state: State<'_, AppState>) {
    state.export_generation.fetch_add(1, Ordering::AcqRel);
}

#[tauri::command]
fn list_export_presets(state: State<'_, AppState>) -> Result<Vec<professional_export::ExportPreset>> {
    professional_export::list_presets(&root_from(&state)?)
}

#[tauri::command]
fn save_export_preset(preset: professional_export::ExportPreset, state: State<'_, AppState>) -> Result<professional_export::ExportPreset> {
    professional_export::save_preset(&root_from(&state)?, preset)
}

#[tauri::command]
fn delete_export_preset(id: String, state: State<'_, AppState>) -> Result<()> {
    professional_export::delete_preset(&root_from(&state)?, &id)
}

#[tauri::command]
fn export_export_preset(id: String, path: String, state: State<'_, AppState>) -> Result<()> {
    professional_export::export_preset(&root_from(&state)?, &id, Path::new(&path))
}

#[tauri::command]
fn import_export_preset(path: String, state: State<'_, AppState>) -> Result<professional_export::ExportPreset> {
    professional_export::import_preset(&root_from(&state)?, Path::new(&path))
}

fn export_asset_image_with_cache(root: &Path, asset_id: &str, destination: &Path, renderer_caches: Option<&Mutex<RendererCaches>>) -> Result<PathBuf> {
    if !destination.is_dir() {
        return Err(KeepframeError::Message("Choose an existing export folder.".into(),));
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

    // Develop recipes are authoritative non-destructive edits.  Export always
    // re-renders them from the protected full-resolution representation; it
    // never upscales a preview or writes back to the original.
    let recipe = develop_recipe_in(&connection, asset_id)?;
    if recipe.is_edited() {
        let input = original_adjustment_input(&connection, asset_id)?;
        let rendered=if let Some(caches)=renderer_caches{
            let(expected_width,expected_height)=input.expected_dimensions.unwrap_or((0,0));
            let source_key=format!("full:{}:{}",input.source_hash,decoded_source_key(&input.path,expected_width,expected_height)?);
            let cached=caches.lock().expect("renderer cache lock").decoded.get(&source_key);
            let image=if let Some(image)=cached{image}else{
                let image=Arc::new(prepare_full_resolution_image(&input.path,input.expected_dimensions,&root.join(".keepframe/staging/working"))?);
                let bytes=image.as_bytes().len();caches.lock().expect("renderer cache lock").decoded.insert(source_key.clone(),Arc::clone(&image),bytes);image
            };
            image::DynamicImage::ImageRgb8(render_develop_recipe_cached(&image,&source_key,&recipe,caches,None)?)
        }else{
            let image=prepare_full_resolution_image(&input.path,input.expected_dimensions,&root.join(".keepframe/staging/working"))?;
            image::DynamicImage::ImageRgb8(render_develop_recipe(&image,&recipe))
        };
        let output = unused_export_path(destination, stem, "png");
        fs::write(&output, encode_srgb_png(&rendered)?)?;
        return Ok(output);
    }

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

#[cfg(test)]
fn export_asset_image_in(root:&Path,asset_id:&str,destination:&Path)->Result<PathBuf>{
    export_asset_image_with_cache(root,asset_id,destination,None)
}

#[tauri::command]
async fn export_asset_image(
    asset_id: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<String> {
    let root = root_from(&state)?;
    let caches=Arc::clone(&state.renderer_caches);
    tauri::async_runtime::spawn_blocking(move || {
        export_asset_image_with_cache(&root, &asset_id, Path::new(&destination),Some(&caches))
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
        tx.execute("UPDATE batches SET state='succeeded' WHERE id=?1", [&batch_id],)?;
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
            mask_generation: AtomicU64::new(0),
            preview_generation: Arc::new(AtomicU64::new(0)),
            renderer_caches: Arc::new(Mutex::new(RendererCaches::default())),
            mask_install_cancel: AtomicBool::new(false),
            export_generation: Arc::new(AtomicU64::new(0)),
            folder_watcher: Mutex::new(None),
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
                            let relocated = interoperability::recover_moved_managed_paths(&mut open_db(&root)?, &root)?;
                            if relocated > 0 { append_runtime_log(&root, "managed_paths_rebased", &format!("{relocated} verified managed paths were rebased after opening this library.")); }
                            let recovery_notice = recover_interrupted_operations(&root)?;
                            allow_media_scope(app.handle(), &root)?;
                            *app.state::<AppState>().root.lock().unwrap() = Some(root.clone());
                            *app.state::<AppState>().library_lock.lock().unwrap() = Some(lock);
                            *app.state::<AppState>().recovery_notice.lock().unwrap() = recovery_notice;
                            restart_folder_watcher(&root, &app.state::<AppState>())?;
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
            get_intelligent_mask_health,
            install_intelligent_mask_model,
            cancel_intelligent_mask_install,
            propose_intelligent_mask,
            cancel_intelligent_mask,
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
            export_xmp_sidecars,
            import_xmp_sidecar,
            export_portable_catalogue,
            rescan_library,
            find_relink_candidates,
            relink_asset,
            configure_folder_watch,
            disable_folder_watches,
            list_folder_watch_events,
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
            get_develop_recipe,
            save_develop_recipe,
            preview_develop_recipe,
            list_develop_presets,
            save_develop_preset,
            delete_develop_preset,
            export_develop_preset,
            import_develop_preset,
            propose_develop_auto,
            import_replacement,
            start_export_batch,
            cancel_export_batch,
            list_export_presets,
            save_export_preset,
            delete_export_preset,
            export_export_preset,
            import_export_preset,
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
    fn local(exposure:f32)->LocalAdjustments{LocalAdjustments{exposure,..LocalAdjustments::default()}}
    fn linear_mask(id:&str,exposure:f32)->DevelopMask{DevelopMask{id:id.into(),name:"Linear".into(),enabled:true,inverted:false,opacity:1.0,feather:1.0,geometry:MaskGeometry::Linear{start:MaskPoint{x:0.0,y:0.5},end:MaskPoint{x:1.0,y:0.5},},adjustments:local(exposure),}}
    fn radial_mask(id:&str,exposure:f32)->DevelopMask{DevelopMask{id:id.into(),name:"Radial".into(),enabled:true,inverted:false,opacity:1.0,feather:0.5,geometry:MaskGeometry::Radial{centre:MaskPoint{x:0.5,y:0.5},radius_x:0.35,radius_y:0.25,rotation:25.0,},adjustments:local(exposure),}}
    fn brush_mask(id:&str,erase:bool)->DevelopMask{DevelopMask{id:id.into(),name:"Brush".into(),enabled:true,inverted:false,opacity:1.0,feather:0.5,geometry:MaskGeometry::Brush{strokes:vec![BrushStroke{points:vec![MaskPoint{x:0.2,y:0.5},MaskPoint{x:0.8,y:0.5}],radius:0.2,feather:0.5,flow:1.0,erase:false,},BrushStroke{points:vec![MaskPoint{x:0.5,y:0.5}],radius:0.08,feather:0.2,flow:1.0,erase,},],},adjustments:local(1.0),
        }
    }
    fn semantic_mask(id: &str, exposure: f32) -> DevelopMask {
        let coverage = image::GrayImage::from_fn(32, 24, |x, y| {
            image::Luma([if (8..24).contains(&x) && (5..19).contains(&y) {
                255
            } else {
                0
            }])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(coverage)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        DevelopMask {
            id: id.into(),
            name: "Subject".into(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            feather: 0.0,
            geometry: MaskGeometry::Semantic {
                width: 32,
                height: 24,
                coverage_png: BASE64.encode(&bytes),
                checksum: format!("{:x}", Sha256::digest(&bytes)),
                provenance: Box::new(MaskProvenance {
                    provider: SEGMENTATION_PROVIDER.into(),
                    provider_version: SEGMENTATION_PROVIDER_VERSION.into(),
                    model: SEGMENTATION_MODEL_ID.into(),
                    model_revision: SEGMENTATION_MODEL_REVISION.into(),
                    model_sha256: SEGMENTATION_MODEL_SHA256.into(),
                    category: "subject".into(),
                    execution_provider: "mock CPU".into(),
                }),
                refinements: Vec::new(),
            },
            adjustments: local(exposure),}}
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
        let rotated = apply_geometry(&source, BasicAdjustments { rotate_quadrants: 1, ..BasicAdjustments::default() },);
        assert_eq!(rotated.dimensions(), (2, 3));
        assert_eq!(rotated.get_pixel(1, 0), &image::Rgb([255, 0, 0]));
        let cropped = apply_geometry(&source, BasicAdjustments { crop_left: 1.0 / 3.0, crop_width: 2.0 / 3.0, crop_height: 1.0, ..BasicAdjustments::default() },);
        assert_eq!(cropped.dimensions(), (2, 2));
        assert_eq!(cropped.get_pixel(0, 0), &image::Rgb([0, 255, 0]));
    }

    #[test]
    fn develop_recipe_is_versioned_deterministic_and_supports_flips() {
        let recipe = DevelopRecipe { schema_version: 2, settings: BasicAdjustments { exposure: 0.5, horizontal_flip: true, ..BasicAdjustments::neutral() }, masks: Vec::new(), };
        let round_trip: DevelopRecipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
        assert_eq!(round_trip.validate().unwrap(), recipe);
        assert!(recipe.is_edited());
        let source = image::DynamicImage::ImageRgb8(RgbImage::from_fn(3, 2, |x, y| { image::Rgb([(x * 40) as u8, (y * 70) as u8, 30])
        }));
        assert_eq!(apply_adjustments_to_image(&source, recipe.settings), apply_adjustments_to_image(&source, recipe.settings));
        let flipped = apply_geometry(&source.to_rgb8(), BasicAdjustments { horizontal_flip: true, ..BasicAdjustments::neutral() },);
        assert_eq!(flipped.get_pixel(0, 0), source.to_rgb8().get_pixel(2, 0));
    }

    #[test]
    fn legacy_v1_recipe_migrates_in_memory_without_visual_change() {
        let json=serde_json::json!({"schemaVersion":1,"settings":BasicAdjustments{exposure:0.4,..BasicAdjustments::neutral()}});let migrated=serde_json::from_value::<DevelopRecipe>(json).unwrap().validate().unwrap();assert_eq!(migrated.schema_version,2);assert!(migrated.masks.is_empty());let source=image::DynamicImage::ImageRgb8(RgbImage::from_pixel(8,8,image::Rgb([80,90,100])));assert_eq!(render_develop_recipe(&source,&migrated),apply_adjustments_to_image(&source,migrated.settings));
    }

    #[test]
    fn mask_geometry_round_trips_and_disabled_masks_still_count_as_edits() {
        let mut recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![linear_mask("linear",1.0),radial_mask("radial",-0.5),brush_mask("brush",true),
                semantic_mask("subject", 0.4),],};recipe.masks[0].enabled=false;let json=serde_json::to_string(&recipe).unwrap();let decoded=serde_json::from_str::<DevelopRecipe>(&json).unwrap().validate().unwrap();assert_eq!(decoded,recipe);assert!(decoded.is_edited());assert!(!json.contains("bitmap"));
        assert!(json.contains("coveragePng"));
    }

    #[test]
    fn semantic_mask_payload_is_authoritative_and_rejects_corruption() {
        let mask = semantic_mask("subject", 1.0);
        let recipe = DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments::neutral(),
            masks: vec![mask.clone()],
        };
        let json = serde_json::to_string(&recipe).unwrap();
        assert_eq!(
            serde_json::from_str::<DevelopRecipe>(&json)
                .unwrap()
                .validate()
                .unwrap(),
            recipe
        );
        let source = image::DynamicImage::ImageRgb8(RgbImage::from_pixel(
            320,
            240,
            image::Rgb([64, 64, 64]),
        ));
        let rendered = render_develop_recipe(&source, &recipe);
        assert!(rendered.get_pixel(160, 120)[0] > rendered.get_pixel(10, 10)[0]);
        let mut corrupt = mask;
        if let MaskGeometry::Semantic { coverage_png, .. } = &mut corrupt.geometry {
            coverage_png.push('A');
        }
        assert!(DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments::neutral(),
            masks: vec![corrupt]
        }
        .validate()
        .is_err());
    }

    #[test]
    fn semantic_add_subtract_opacity_and_invert_use_normalised_brushes() {
        let mut base = semantic_mask("base", 1.0);
        let left = mask_coverage(&base, 5, 12, 32, 24);
        let centre = mask_coverage(&base, 16, 12, 32, 24);
        assert!(left < 0.05 && centre > 0.95);
        if let MaskGeometry::Semantic { refinements, .. } = &mut base.geometry {
            refinements.push(BrushStroke {
                points: vec![MaskPoint { x: 0.15, y: 0.5 }],
                radius: 0.15,
                feather: 0.0,
                flow: 1.0,
                erase: false,
            });
            refinements.push(BrushStroke {
                points: vec![MaskPoint { x: 0.5, y: 0.5 }],
                radius: 0.1,
                feather: 0.0,
                flow: 1.0,
                erase: true,
            });
        }
        assert!(mask_coverage(&base, 5, 12, 32, 24) > 0.9);
        assert!(mask_coverage(&base, 16, 12, 32, 24) < 0.1);
        base.opacity = 0.5;
        assert!(mask_coverage(&base, 5, 12, 32, 24) < 0.51);
        base.inverted = true;
        assert!(mask_coverage(&base, 16, 12, 32, 24) > 0.45);
    }

    #[test]
    fn semantic_mask_alignment_matches_preview_full_and_all_geometry_transforms() {
        let mask = semantic_mask("subject", 0.7);
        for (x, y) in [(0.1, 0.1), (0.5, 0.5), (0.8, 0.7)] {
            let preview = mask_coverage(&mask, (x * 319.0) as u32, (y * 239.0) as u32, 320, 240);
            let full = mask_coverage(&mask, (x * 3199.0) as u32, (y * 2399.0) as u32, 3200, 2400);
            assert!(
                (preview - full).abs() < 0.06,
                "{x},{y}: {preview} vs {full}"
            );
        }
        let source = image::DynamicImage::ImageRgb8(RgbImage::from_fn(96, 64, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 3) as u8, 80])
        }));
        let settings = BasicAdjustments {
            crop_left: 0.1,
            crop_top: 0.1,
            crop_width: 0.75,
            crop_height: 0.8,
            rotate_quadrants: 1,
            straighten: 3.0,
            horizontal_flip: true,
            vertical_flip: true,
            ..BasicAdjustments::neutral()
        };
        let before = render_develop_recipe(
            &source,
            &DevelopRecipe {
                schema_version: 2,
                settings: BasicAdjustments::neutral(),
                masks: vec![mask.clone()],
            },
        );
        let after = render_develop_recipe(
            &source,
            &DevelopRecipe {
                schema_version: 2,
                settings,
                masks: vec![mask],
            },
        );
        assert_eq!(after, apply_geometry(&before, settings));
    }

    #[test]
    fn linear_radial_and_inverted_masks_apply_bounded_local_adjustments() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_pixel(41,31,image::Rgb([80,80,80])));let linear=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![linear_mask("linear",1.0)],};let output=render_develop_recipe(&source,&linear);assert!(output.get_pixel(38,15)[0]>output.get_pixel(2,15)[0]);let radial=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![radial_mask("radial",1.0)],};let inside=render_develop_recipe(&source,&radial);assert!(inside.get_pixel(20,15)[0]>inside.get_pixel(0,0)[0]);let mut inverted=radial.clone();inverted.masks[0].inverted=true;inverted.masks[0].opacity=0.5;let outside=render_develop_recipe(&source,&inverted);assert!(outside.get_pixel(0,0)[0]>outside.get_pixel(20,15)[0]);
    }

    #[test]
    fn brush_erase_removes_coverage_and_one_stroke_is_compact() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_pixel(51,51,image::Rgb([64,64,64])));let paint=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![brush_mask("brush",false)],};let erased=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![brush_mask("brush",true)],};assert!(render_develop_recipe(&source,&paint).get_pixel(25,25)[0]>render_develop_recipe(&source,&erased).get_pixel(25,25)[0]);assert_eq!(match &erased.masks[0].geometry{MaskGeometry::Brush{strokes}=>strokes.len(),_=>0,},2);
    }

    #[test]
    fn overlapping_masks_compose_deterministically_after_global_edits() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_pixel(32,24,image::Rgb([90,100,110]),));let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{contrast:10.0,..BasicAdjustments::neutral()},masks:vec![linear_mask("a",0.5),radial_mask("b",0.5)],};let first=render_develop_recipe(&source,&recipe);assert_eq!(first,render_develop_recipe(&source,&recipe));assert_ne!(first,source.to_rgb8());
    }

    #[test]
    fn masks_remain_attached_when_crop_rotation_and_flips_change() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_fn(48,32,|x,y| {image::Rgb([(x*3)as u8,(y*4)as u8,80])
        }));let geometry=BasicAdjustments{crop_left:0.1,crop_top:0.1,crop_width:0.8,crop_height:0.8,rotate_quadrants:1,horizontal_flip:true,vertical_flip:true,..BasicAdjustments::neutral()};let recipe=DevelopRecipe{schema_version:2,settings:geometry,masks:vec![linear_mask("linear",0.7)],};let rendered=render_develop_recipe(&source,&recipe);let before=render_develop_recipe(&source,&DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:recipe.masks.clone(),},);assert_eq!(rendered,apply_geometry(&before,geometry));
    }

    #[test]
    fn normalised_mask_coverage_matches_preview_and_full_resolution() {
        let mask=linear_mask("linear",1.0);for x in [0.1_f32,0.5,0.9]{let preview=mask_coverage(&mask,(x*100.0).round()as u32,50,101,101);let full=mask_coverage(&mask,(x*1000.0).round()as u32,500,1001,1001);assert!((preview-full).abs()<0.001);}
    }

    #[test]
    fn disabling_a_mask_restores_the_global_render() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_pixel(24,18,image::Rgb([72,82,92])));let mut recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{contrast:8.0,..BasicAdjustments::neutral()},masks:vec![linear_mask("linear",1.0)],};let enabled=render_develop_recipe(&source,&recipe);recipe.masks[0].enabled=false;let disabled=render_develop_recipe(&source,&recipe);assert_eq!(disabled,apply_adjustments_to_image(&source,recipe.settings));assert_ne!(enabled,disabled);
    }

    #[test]
    fn mask_geometry_changes_preview_cache_identity() {
        let mut recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![linear_mask("linear",0.5)],};let first=Sha256::digest(serde_json::to_vec(&recipe).unwrap());if let MaskGeometry::Linear{end,..}=&mut recipe.masks[0].geometry{end.x=0.8;}let second=Sha256::digest(serde_json::to_vec(&recipe).unwrap());assert_ne!(first,second);
        let mut semantic = DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments::neutral(),
            masks: vec![semantic_mask("subject", 0.5)],
        };
        let accepted = Sha256::digest(serde_json::to_vec(&semantic).unwrap());
        if let MaskGeometry::Semantic { refinements, .. } = &mut semantic.masks[0].geometry {
            refinements.push(BrushStroke {
                points: vec![MaskPoint { x: 0.5, y: 0.5 }],
                radius: 0.1,
                feather: 0.5,
                flow: 1.0,
                erase: true,
            });
        }
        assert_ne!(
            accepted,
            Sha256::digest(serde_json::to_vec(&semantic).unwrap())
        );
    }

    #[test]
    fn copied_develop_recipe_contains_no_asset_metadata() {
        let recipe = DevelopRecipe { schema_version: 2, settings: BasicAdjustments { exposure: 0.5, saturation: 12.0, ..BasicAdjustments::neutral() }, masks: Vec::new(), };
        let copied: DevelopRecipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
        let stored = serde_json::to_string(&copied).unwrap();
        for forbidden in ["filename", "capturedAt", "decision", "tags", "latitude", "longitude", "path", "assetId",] {
            assert!(!stored.contains(forbidden), "copied Develop settings leaked {forbidden}");
        }
        assert_eq!(copied.settings, recipe.settings);
    }

    #[test]
    fn develop_recipe_persists_without_changing_the_protected_original() {
        let root = std::env::temp_dir().join(format!("keepframe-develop-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/photo.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 48, image::Rgb([70, 90, 110]))).save(&original).unwrap();
        let original_hash = hash_file(&original).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,width,height,thumbnail_path,created_at)VALUES('asset','photo.png','2026-09-12T12:00:00Z',64,48,?1,'2026-09-12T12:00:00Z')", [original.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','photo',?3,0)", params![original.to_string_lossy(), original_hash, fs::metadata(&original).unwrap().len()]).unwrap();
        let mut inverted_radial=radial_mask("radial",-0.3);inverted_radial.inverted=true;
        let recipe = DevelopRecipe { schema_version: 2, settings: BasicAdjustments { exposure: 0.75, crop_left: 0.1, crop_width: 0.8, rotate_quadrants: 1, ..BasicAdjustments::neutral() }, masks: vec![linear_mask("linear",0.5),inverted_radial,brush_mask("brush",true),
                semantic_mask("subject", 0.3),], };
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES('asset',2,?1,?2)", params![serde_json::to_string(&recipe).unwrap(), Utc::now().to_rfc3339()]).unwrap();
        assert_eq!(develop_recipe_in(&connection, "asset").unwrap(), recipe);
        drop(connection);
        let destination = root.join("Exports");
        let exported = export_asset_image_in(&root, "asset", &destination).unwrap();
        assert_eq!(image::open(exported).unwrap().dimensions(), (38, 64));
        assert_eq!(hash_file(&original).unwrap(), original_hash);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn renderer_cache_is_bounded_and_evicts_least_recently_used_entries() {
        let mut cache=BoundedCache::new(8);
        cache.insert("a".into(),Arc::new(vec![1_u8;4]),4);
        cache.insert("b".into(),Arc::new(vec![2_u8;4]),4);
        assert!(cache.get("a").is_some());
        cache.insert("c".into(),Arc::new(vec![3_u8;4]),4);
        assert!(cache.get("b").is_none());
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.used_bytes<=cache.limit_bytes);
        assert_eq!(cache.evictions,1);
    }

    #[test]
    fn preview_decode_cache_reuses_unchanged_files_and_invalidates_source_changes() {
        let root=std::env::temp_dir().join(format!("keepframe-render-cache-{}",Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path=root.join("preview.png");
        image::DynamicImage::new_rgb8(32,24).save(&path).unwrap();
        let original_hash=hash_file(&path).unwrap();
        let caches=Mutex::new(RendererCaches::default());
        let(first_key,first)=decode_preview_cached(&path,&caches).unwrap();
        let(second_key,second)=decode_preview_cached(&path,&caches).unwrap();
        assert_eq!(first_key,second_key);assert!(Arc::ptr_eq(&first,&second));
        assert_eq!(hash_file(&path).unwrap(),original_hash);
        image::DynamicImage::new_rgb8(33,24).save(&path).unwrap();
        let(third_key,third)=decode_preview_cached(&path,&caches).unwrap();
        assert_ne!(first_key,third_key);assert!(!Arc::ptr_eq(&first,&third));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn full_resolution_export_reuses_safe_source_and_intermediate_caches() {
        let root=std::env::temp_dir().join(format!("keepframe-full-render-cache-{}",Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original=root.join("Originals/2026/09/12/photo.png");fs::create_dir_all(original.parent().unwrap()).unwrap();
        image::DynamicImage::ImageRgb8(RgbImage::from_fn(128,96,|x,y|image::Rgb([x as u8,y as u8,(x+y)as u8]))).save(&original).unwrap();
        let original_hash=hash_file(&original).unwrap();let connection=open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,width,height,thumbnail_path,created_at)VALUES('asset','photo.png','2026-09-12T12:00:00Z',128,96,?1,'2026-09-12T12:00:00Z')",[original.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','photo',?3,0)",params![original.to_string_lossy(),original_hash,fs::metadata(&original).unwrap().len()]).unwrap();
        let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.2,..BasicAdjustments::neutral()},masks:Vec::new()};
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES('asset',2,?1,'2026-09-12T12:00:00Z')",[serde_json::to_string(&recipe).unwrap()]).unwrap();drop(connection);
        let caches=Mutex::new(RendererCaches::default());let destination=root.join("Exports");
        let first=export_asset_image_with_cache(&root,"asset",&destination,Some(&caches)).unwrap();
        let second=export_asset_image_with_cache(&root,"asset",&destination,Some(&caches)).unwrap();
        assert_eq!(image::open(first).unwrap().to_rgb8(),image::open(second).unwrap().to_rgb8());
        let cache=caches.lock().unwrap();assert!(cache.decoded.hits>=1);assert!(cache.intermediates.hits>=1);drop(cache);
        assert_eq!(hash_file(&original).unwrap(),original_hash);fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn intermediate_cache_invalidates_only_colour_dependencies() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_fn(64,48,|x,y|image::Rgb([x as u8,y as u8,(x+y)as u8])));
        let caches=Mutex::new(RendererCaches::default());
        let base=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.2,..BasicAdjustments::neutral()},masks:Vec::new()};
        render_develop_recipe_cached(&source,"source",&base,&caches,None).unwrap();
        let mut crop=base.clone();crop.settings.crop_width=0.8;
        render_develop_recipe_cached(&source,"source",&crop,&caches,None).unwrap();
        let mut exposure=crop;exposure.settings.exposure=0.3;
        render_develop_recipe_cached(&source,"source",&exposure,&caches,None).unwrap();
        let cache=caches.lock().unwrap();
        assert_eq!((cache.intermediates.hits,cache.intermediates.misses),(1,2));
    }

    #[test]
    fn mask_and_semantic_caches_reuse_coverage_and_invalidate_geometry() {
        let mut caches=RendererCaches::default();
        let mut mask=semantic_mask("subject",0.3);
        let first=prepared_mask_coverage_cached(&mask,72,48,&mut caches).unwrap();
        let second=prepared_mask_coverage_cached(&mask,72,48,&mut caches).unwrap();
        assert!(Arc::ptr_eq(&first,&second));assert_eq!(caches.masks.hits,1);
        mask.opacity=0.5;
        let third=prepared_mask_coverage_cached(&mask,72,48,&mut caches).unwrap();
        assert!(!Arc::ptr_eq(&first,&third));assert_eq!(caches.masks.misses,2);
        assert_eq!(caches.semantics.misses,1);assert_eq!(caches.semantics.hits,1);
    }

    #[test]
    fn cached_parallel_renderer_matches_uncached_pixels_and_is_deterministic() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_fn(96,64,|x,y|image::Rgb([(x*2)as u8,(y*3)as u8,(x+y)as u8])));
        let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.25,clarity:12.0,crop_left:0.1,crop_width:0.8,..BasicAdjustments::neutral()},masks:vec![linear_mask("one",0.4),radial_mask("two",-0.2)]};
        let expected=render_develop_recipe(&source,&recipe);
        let caches=Mutex::new(RendererCaches::default());
        let first=render_develop_recipe_cached(&source,"source",&recipe,&caches,None).unwrap();
        let second=render_develop_recipe_cached(&source,"source",&recipe,&caches,None).unwrap();
        assert_eq!(first,expected);assert_eq!(second,expected);
        let expected=Arc::new(expected);
        std::thread::scope(|scope|{for _ in 0..4{let source=&source;let recipe=&recipe;let expected=Arc::clone(&expected);scope.spawn(move||assert_eq!(render_develop_recipe(source,recipe),*expected));}});
    }

    #[test]
    fn stale_preview_generation_is_rejected_before_rendering() {
        let source=image::DynamicImage::new_rgb8(32,24);
        let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:Vec::new()};
        let caches=Mutex::new(RendererCaches::default());
        let generation=AtomicU64::new(2);
        let error=render_develop_recipe_cached(&source,"source",&recipe,&caches,Some((&generation,1))).unwrap_err();
        assert_eq!(error.to_string(),"Superseded Develop preview.");
        assert_eq!(caches.lock().unwrap().intermediates.misses,0);
    }

    #[test]
    #[ignore = "opt-in Milestone 10 renderer benchmark; run scripts/benchmark-renderer.ps1"]
    fn milestone_10_renderer_performance_checkpoint() {
        let root=std::env::temp_dir().join(format!("keepframe-m10-benchmark-{}",Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let fixture=root.join("representative.jpg");
        let source=RgbImage::from_fn(720,480,|x,y|image::Rgb([((x*7+y*3)%256)as u8,((x*2+y*5)%256)as u8,((x+y*11)%256)as u8]));
        image::DynamicImage::ImageRgb8(source).save_with_format(&fixture,ImageFormat::Jpeg).unwrap();

        let caches=Mutex::new(RendererCaches::default());
        let started=Instant::now();let(_,cold_decoded)=decode_preview_cached(&fixture,&caches).unwrap();let jpeg_decode_cold=started.elapsed();
        let started=Instant::now();let(source_key,warm_decoded)=decode_preview_cached(&fixture,&caches).unwrap();let jpeg_decode_warm=started.elapsed();
        let started=Instant::now();let working=warm_decoded.to_rgb8();let working_buffer=started.elapsed();
        let started=Instant::now();let oriented=decode_standard_with_orientation(&fixture).unwrap();let orientation_decode=started.elapsed();

        let stage=|settings|{let started=Instant::now();let output=apply_colour_adjustments_to_image(&oriented,settings);(started.elapsed(),output)};
        let(tone,_)=stage(BasicAdjustments{exposure:0.35,contrast:12.0,highlights:-20.0,shadows:18.0,..BasicAdjustments::neutral()});
        let(white_balance,_)=stage(BasicAdjustments{light_balance:10.0,tint:-4.0,..BasicAdjustments::neutral()});
        let(presence,_)=stage(BasicAdjustments{texture:10.0,clarity:8.0,dehaze:5.0,..BasicAdjustments::neutral()});
        let(colour,_)=stage(BasicAdjustments{colour_boost:8.0,saturation:6.0,..BasicAdjustments::neutral()});
        let global_settings=BasicAdjustments{exposure:0.35,highlights:-20.0,shadows:18.0,texture:10.0,clarity:8.0,saturation:6.0,..BasicAdjustments::neutral()};
        let global_recipe=DevelopRecipe{schema_version:2,settings:global_settings,masks:Vec::new()};
        let started=Instant::now();let global_cold=render_develop_recipe_cached(&warm_decoded,&source_key,&global_recipe,&caches,None).unwrap();let overall_cold=started.elapsed();
        let started=Instant::now();let global_warm=render_develop_recipe_cached(&warm_decoded,&source_key,&global_recipe,&caches,None).unwrap();let overall_warm=started.elapsed();
        assert_eq!(global_cold,global_warm);assert!(Arc::ptr_eq(&cold_decoded,&warm_decoded));

        let mask=linear_mask("profile",0.25);let started=Instant::now();let mask_coverage=prepared_mask_coverage(&mask,720,480,None);let mask_raster=started.elapsed();
        let mut mask_times=Vec::new();
        for count in [1,5,10]{let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.2,..BasicAdjustments::neutral()},masks:(0..count).map(|index|linear_mask(&format!("mask-{index}"),0.12)).collect()};let started=Instant::now();let output=render_develop_recipe(&warm_decoded,&recipe);mask_times.push((count,started.elapsed(),output));}
        let cached_mask_recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.2,..BasicAdjustments::neutral()},masks:vec![linear_mask("cached",0.12)]};
        let started=Instant::now();render_develop_recipe_cached(&warm_decoded,&source_key,&cached_mask_recipe,&caches,None).unwrap();let cached_mask_cold=started.elapsed();
        let started=Instant::now();render_develop_recipe_cached(&warm_decoded,&source_key,&cached_mask_recipe,&caches,None).unwrap();let cached_mask_warm=started.elapsed();
        let semantic_recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments::neutral(),masks:vec![semantic_mask("subject",0.2)]};
        let started=Instant::now();render_develop_recipe_cached(&warm_decoded,&source_key,&semantic_recipe,&caches,None).unwrap();let semantic_cold=started.elapsed();
        let started=Instant::now();render_develop_recipe_cached(&warm_decoded,&source_key,&semantic_recipe,&caches,None).unwrap();let semantic_warm=started.elapsed();
        let geometry=BasicAdjustments{crop_left:0.1,crop_top:0.1,crop_width:0.8,crop_height:0.8,rotate_quadrants:1,straighten:1.5,..BasicAdjustments::neutral()};
        let started=Instant::now();let transformed=apply_geometry(&global_cold,geometry);let transform=started.elapsed();
        let started=Instant::now();let resized=image::imageops::resize(&global_cold,360,240,image::imageops::FilterType::Triangle);let resize=started.elapsed();
        let started=Instant::now();let encoded=encode_srgb_png(&image::DynamicImage::ImageRgb8(global_cold.clone())).unwrap();let png_encode=started.elapsed();
        let full=image::DynamicImage::ImageRgb8(image::imageops::resize(&working,2400,1600,image::imageops::FilterType::Triangle));
        let started=Instant::now();let full_render=apply_adjustments_to_image(&full,global_settings);let full_render_elapsed=started.elapsed();
        let started=Instant::now();let full_png=encode_srgb_png(&image::DynamicImage::ImageRgb8(full_render)).unwrap();let full_png_elapsed=started.elapsed();
        let cache=caches.lock().unwrap();
        eprintln!("M10 benchmark 720x480: jpeg-decode cold={jpeg_decode_cold:?} warm={jpeg_decode_warm:?}; orientation+decode={orientation_decode:?}; working-buffer={working_buffer:?}; tone/exposure={tone:?}; white-balance={white_balance:?}; presence={presence:?}; colour={colour:?}; mask-raster={mask_raster:?}; masks 1={:?} 5={:?} 10={:?}; cached-mask cold={cached_mask_cold:?} warm={cached_mask_warm:?}; semantic cold={semantic_cold:?} warm={semantic_warm:?}; transform={transform:?}; resize={resize:?}; PNG={png_encode:?}; overall cold={overall_cold:?} warm={overall_warm:?}; full 2400x1600 render={full_render_elapsed:?} PNG={full_png_elapsed:?}; cache decoded={}B intermediate={}B mask={}B semantic={}B hits={}/{}/{}/{} evictions={}/{}/{}/{}; outputs={}+{}B coverage={} transformed={}x{} resized={}x{}",mask_times[0].1,mask_times[1].1,mask_times[2].1,cache.decoded.used_bytes,cache.intermediates.used_bytes,cache.masks.used_bytes,cache.semantics.used_bytes,cache.decoded.hits,cache.intermediates.hits,cache.masks.hits,cache.semantics.hits,cache.decoded.evictions,cache.intermediates.evictions,cache.masks.evictions,cache.semantics.evictions,encoded.len(),full_png.len(),mask_coverage.iter().sum::<f32>(),transformed.width(),transformed.height(),resized.width(),resized.height());
        assert!(overall_cold<Duration::from_millis(250));assert!(mask_times[0].1<Duration::from_millis(300));assert!(mask_times[1].1<Duration::from_millis(800));assert!(mask_times[2].1<Duration::from_millis(1500));assert!(full_render_elapsed+full_png_elapsed<Duration::from_secs(3));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[ignore = "manual performance checkpoint; run with --ignored --nocapture"]
    fn develop_render_performance_checkpoint() {
        let preview = image::DynamicImage::ImageRgb8(RgbImage::from_fn(720, 480, |x, y| { image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        }));
        let recipe = BasicAdjustments { exposure: 0.35, highlights: -20.0, shadows: 18.0, texture: 10.0, clarity: 8.0, saturation: 6.0, ..BasicAdjustments::neutral() };
        let preview_started = Instant::now();
        let rendered_preview = apply_adjustments_to_image(&preview, recipe);
        let preview_elapsed = preview_started.elapsed();
        let warm_started = Instant::now();
        let cache_key = Sha256::digest(serde_json::to_vec(&recipe).unwrap());
        let warm_elapsed = warm_started.elapsed();
        let full = image::DynamicImage::ImageRgb8(image::imageops::resize(&rendered_preview, 2400, 1600, image::imageops::FilterType::Triangle,));
        let export_started = Instant::now();
        let encoded = encode_srgb_png(&image::DynamicImage::ImageRgb8(apply_adjustments_to_image(&full, recipe,))).unwrap();
        let export_elapsed = export_started.elapsed();
        eprintln!("Develop checkpoint: 720x480 preview={preview_elapsed:?}; warm recipe/cache-key={warm_elapsed:?}; 2400x1600 render+PNG={export_elapsed:?}; output={} bytes", encoded.len());
        assert!(!cache_key.is_empty());
        assert!(preview_elapsed < Duration::from_secs(15));
        assert!(export_elapsed < Duration::from_secs(15));
    }

    #[test]
    #[ignore = "manual Milestone 8 renderer and masking profile; run with --ignored --nocapture"]
    fn mask_render_performance_checkpoint() {
        let source=image::DynamicImage::ImageRgb8(RgbImage::from_fn(720,480,|x,y| {image::Rgb([(x%220)as u8+20,(y%210)as u8+20,((x+y)%200)as u8+30,])
        }));let bytes=encode_srgb_png(&source).unwrap();let started=Instant::now();let decoded=image::load_from_memory(&bytes).unwrap();let decode=started.elapsed();let started=Instant::now();let global=apply_colour_adjustments_to_image(&decoded,BasicAdjustments{exposure:0.2,clarity:5.0,..BasicAdjustments::neutral()},);let global_elapsed=started.elapsed();let mask=linear_mask("profile",0.25);let started=Instant::now();let mut sum=0.0;for y in 0..decoded.height(){for x in 0..decoded.width(){sum+=mask_coverage(&mask,x,y,decoded.width(),decoded.height());}}let raster=started.elapsed();let mut times=Vec::new();for count in [1,5,10]{let masks=(0..count).map(|index|linear_mask(&format!("mask-{index}"),0.12)).collect();let recipe=DevelopRecipe{schema_version:2,settings:BasicAdjustments{exposure:0.2,..BasicAdjustments::neutral()},masks,};let started=Instant::now();let rendered=render_develop_recipe(&decoded,&recipe);times.push((count,started.elapsed(),rendered));}let started=Instant::now();let resized=image::imageops::resize(&global,360,240,image::imageops::FilterType::Triangle);let resize=started.elapsed();let started=Instant::now();let encoded=encode_srgb_png(&image::DynamicImage::ImageRgb8(times[0].2.clone())).unwrap();let encode=started.elapsed();eprintln!("Milestone 8 profile 720x480: decode={decode:?}; global={global_elapsed:?}; one-mask-raster={raster:?}; local-composite 1={:?} 5={:?} 10={:?}; resize={resize:?}; encode={encode:?}; coverage={sum:.1}; bytes={} resized={}x{}",times[0].1,times[1].1,times[2].1,encoded.len(),resized.width(),resized.height());assert!(times[2].1<Duration::from_secs(30));
    }

    #[test]
    #[ignore = "manual Milestone 9 accepted-mask performance checkpoint; run with --ignored --nocapture"]
    fn semantic_mask_performance_checkpoint() {
        let coverage = image::GrayImage::from_fn(640, 427, |x, y| {
            image::Luma([if x > 120 && x < 520 && y < 230 {
                255
            } else {
                0
            }])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(coverage)
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        let encoded = encoded.into_inner();
        let mask = DevelopMask {
            id: "semantic-profile".into(),
            name: "Sky".into(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            feather: 0.0,
            geometry: MaskGeometry::Semantic {
                width: 640,
                height: 427,
                coverage_png: BASE64.encode(&encoded),
                checksum: format!("{:x}", Sha256::digest(&encoded)),
                provenance: Box::new(MaskProvenance {
                    provider: SEGMENTATION_PROVIDER.into(),
                    provider_version: SEGMENTATION_PROVIDER_VERSION.into(),
                    model: SEGMENTATION_MODEL_ID.into(),
                    model_revision: SEGMENTATION_MODEL_REVISION.into(),
                    model_sha256: SEGMENTATION_MODEL_SHA256.into(),
                    category: "sky".into(),
                    execution_provider: "qualification fixture".into(),
                }),
                refinements: Vec::new(),
            },
            adjustments: local(0.25),
        };
        let recipe = DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments::neutral(),
            masks: vec![mask],
        }
        .validate()
        .unwrap();
        let started = Instant::now();
        let prepared = prepared_semantic_coverage(&recipe.masks[0], 720, 480)
            .unwrap()
            .unwrap();
        let overlay = started.elapsed();
        assert_eq!(prepared.dimensions(), (720, 480));
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE develop_recipes(asset_id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,recipe_json TEXT NOT NULL,updated_at TEXT NOT NULL);").unwrap();
        let serialised = serde_json::to_string(&recipe).unwrap();
        let started = Instant::now();
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES('asset',2,?1,'2026-09-12T00:00:00Z')",[&serialised]).unwrap();
        let persist = started.elapsed();
        let source = image::DynamicImage::ImageRgb8(RgbImage::from_fn(720, 480, |x, y| {
            image::Rgb([
                (x % 220) as u8 + 20,
                (y % 210) as u8 + 20,
                ((x + y) % 200) as u8 + 30,
            ])
        }));
        let started = Instant::now();
        let rendered = render_develop_recipe(&source, &recipe);
        let preview = started.elapsed();
        eprintln!("Milestone 9 semantic profile 640x427 -> 720x480: overlay/raster={overlay:?}; accepted-payload persistence={persist:?}; preview render={preview:?}; compressed={} bytes; JSON={} bytes; temporary coverage={} bytes",encoded.len(),serialised.len(),640*427);
        assert_eq!(rendered.dimensions(), (720, 480));
        assert!(preview < Duration::from_secs(10));
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
    fn built_in_develop_presets_are_valid_and_never_include_geometry() {
        let presets = built_in_presets();
        assert_eq!(presets.len(), 13);
        assert!(presets.iter().all(|preset| preset.clone().validate(false).is_ok()));
        assert!(presets.iter().all(|preset| preset.settings.crop_left == 0.0 && preset.settings.crop_width == 1.0 && !preset.settings.horizontal_flip));
        assert!(presets.iter().any(|preset| preset.name == "Black & White" && preset.settings.saturation == -100.0));
    }

    #[test]
    fn untrusted_preset_rejects_unknown_categories_and_out_of_range_values() {
        let invalid_category = DevelopPreset { schema_version: 1, id: "user".into(), name: "Unsafe".into(), categories: vec!["filesystem".into()], settings: BasicAdjustments::neutral(), built_in: false, };
        assert!(invalid_category.validate(true).is_err());
        let invalid_value = DevelopPreset { schema_version: 1, id: "user".into(), name: "Unsafe".into(), categories: vec!["tone".into()], settings: BasicAdjustments { exposure: 99.0, ..BasicAdjustments::neutral() }, built_in: false, };
        assert!(invalid_value.validate(true).is_err());
    }

    #[test]
    fn auto_analysis_is_image_dependent_conservative_and_preserves_geometry() {
        let dark = image::DynamicImage::ImageRgb8(RgbImage::from_pixel(96, 64, image::Rgb([45, 18, 8])));
        let bright = image::DynamicImage::ImageRgb8(RgbImage::from_pixel(96, 64, image::Rgb([245, 245, 245]),));
        let current = BasicAdjustments { crop_left: 0.1, crop_top: 0.1, crop_width: 0.8, crop_height: 0.8, rotate_quadrants: 1, ..BasicAdjustments::neutral() };
        let dark_proposal = auto_proposal(&dark, current); let bright_proposal = auto_proposal(&bright, current);
        assert!(dark_proposal.settings.exposure > current.exposure);
        assert!(bright_proposal.settings.highlights < current.highlights);
        assert_eq!((dark_proposal.settings.crop_left, dark_proposal.settings.crop_width, dark_proposal.settings.rotate_quadrants), (0.1, 0.8, 1));
        assert!(dark_proposal.statistics.p50 < bright_proposal.statistics.p50);
        assert!(dark_proposal.explanation.iter().any(|line| line.contains("White balance unchanged")));
    }

    #[test]
    fn statistics_detect_clipping_and_channel_cast_without_a_model() {
        let mut source = RgbImage::from_pixel(100, 1, image::Rgb([50, 30, 20]));
        for index in 0..10 { source.put_pixel(index, 0, image::Rgb([255, 255, 255])); }
        let statistics = image_statistics(&image::DynamicImage::ImageRgb8(source));
        assert!(statistics.highlight_clip_fraction > 0.05);
        assert!(statistics.red_green_blue[0] > statistics.red_green_blue[2]);
        assert_eq!(statistics.luminance_bins.len(), 64);
    }

    #[test]
    #[ignore = "manual Milestone 7 performance checkpoint; run with --ignored --nocapture"]
    fn intelligent_editing_performance_checkpoint() {
        let image = image::DynamicImage::ImageRgb8(RgbImage::from_fn(720, 480, |x, y| { image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x * 3 + y) % 256) as u8])
        }));
        let histogram_started = Instant::now(); let statistics = image_statistics(&image); let histogram_elapsed = histogram_started.elapsed();
        let auto_started = Instant::now(); let proposal = auto_proposal(&image, BasicAdjustments::neutral()); let auto_elapsed = auto_started.elapsed();
        let preview_started = Instant::now(); let _preview = apply_adjustments_to_image(&image, proposal.settings); let preview_elapsed = preview_started.elapsed();
        let warm_preset = built_in_presets().into_iter().find(|preset| preset.id == "warm").unwrap();
        let preset_started = Instant::now(); let _preset = apply_adjustments_to_image(&image, warm_preset.settings); let preset_elapsed = preset_started.elapsed();
        eprintln!("M7 checkpoint: histogram={histogram_elapsed:?}; auto={auto_elapsed:?}; auto-preview={preview_elapsed:?}; warm-preset-render={preset_elapsed:?}; samples={}", statistics.samples);
        assert_eq!(statistics.luminance_bins.len(), 64);
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
            .query_row("SELECT COUNT(*) FROM schema_migrations WHERE version=5", [], |row| row.get(0),)
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
        let none: String = connection.query_row("SELECT location_source FROM assets WHERE id='none'", [], |row| row.get(0),).unwrap();
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
        let state: String = connection.query_row("SELECT state FROM imports WHERE id='import-1'", [], |row| { row.get(0)
            }).unwrap();
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
        let is_trashed: Option<String> = connection.query_row("SELECT trashed_at FROM assets WHERE id='asset-1'", [], |row| row.get(0),).unwrap();
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
        for (name, format) in [("fixture.jpg", image::ImageFormat::Jpeg), ("fixture.png", image::ImageFormat::Png), ("fixture.tiff", image::ImageFormat::Tiff),] {
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
        for (name, format) in [("fixture.jpg", image::ImageFormat::Jpeg), ("fixture.png", image::ImageFormat::Png), ("fixture.tiff", image::ImageFormat::Tiff),] {
            let source = root.join(name);
            source_image.save_with_format(&source, format).unwrap();
            let before_hash = hash_file(&source).unwrap();
            let preview = apply_adjustments_to_image(&image::open(&source).unwrap(), adjustments);
            let full_resolution = prepare_full_resolution_image(&source, Some((30, 20)), &root.join("working")).unwrap();
            let candidate = apply_adjustments_to_image(&full_resolution, adjustments);
            assert_eq!(candidate.dimensions(), (10, 23), "{name}");
            assert_eq!(candidate, preview, "{name}");
            let output = root.join(format!("{name}.candidate.png"));
            fs::write(&output, encode_srgb_png(&image::DynamicImage::ImageRgb8(candidate)).unwrap(),).unwrap();
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
        AssetFilter { decision: "all".into(), search: String::new(), year: None, tag: None, date_from: None, date_to: None, camera: None, tagged: None, located: None, trashed: Some(false), }
    }

    #[test]
    fn map_query_reads_the_full_filtered_catalogue_and_excludes_bad_coordinates() {
        let root = std::env::temp_dir().join(format!("keepframe-map-query-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let connection = open_db(&root).unwrap();
        for (id, decision, latitude, longitude) in [("keep", "keep", 56.12, -3.1), ("discard", "discard", 56.15, -3.2), ("bad", "keep", 95.0, -3.3), ("none", "keep", 0.0, 0.0),] {
            connection.execute("INSERT INTO assets(id,filename,decision,captured_at,latitude,longitude,location_source,thumbnail_path,created_at)VALUES(?1,?1,?2,'2026-09-12T12:00:00Z',?3,?4,'embedded','thumb.jpg','2026-09-12T12:00:00Z')", params![id, decision, latitude, longitude]).unwrap();
        }
        connection.execute("UPDATE assets SET latitude=NULL,longitude=NULL,location_source='none' WHERE id='none'", []).unwrap();
        let all = query_map_assets_in(&connection, &MapQuery { filter: map_filter(), bounds: None, },).unwrap();
        assert_eq!(all.len(), 2);
        let mut keep = map_filter(); keep.decision = "keep".into();
        let bounded = query_map_assets_in(&connection, &MapQuery { filter: keep, bounds: Some(MapBounds { south: 56.0, west: -3.15, north: 56.13, east: -3.0, }), },).unwrap();
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
            let markers = query_map_assets_in(&connection, &MapQuery { filter: map_filter(), bounds: None, },).unwrap();
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
        let waiting: String = connection.query_row("SELECT state FROM jobs WHERE asset_id='asset'", [], |row| { row.get(0)
            }).unwrap();
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
        let job = import_returned_edit_in(&root, "asset", &returned, "chatgpt", "Improve the lighting naturally.", Some(test_recipe("asset")),).unwrap();
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
        let preferred: String = connection.query_row("SELECT preferred_version_id FROM assets WHERE id='asset'", [], |row| row.get(0),).unwrap();
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

    #[test]
    fn folder_watch_inbox_deduplicates_debounced_events() {
        let root = std::env::temp_dir().join(format!("keepframe-watch-test-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let path = root.join("external/new-photo.jpg");
        write_debounced_watch_event(&root, r"D:\Camera", &path, "new_file");
        write_debounced_watch_event(&root, r"D:\Camera", &path, "new_file");
        let connection = open_db(&root).unwrap();
        let count: i64 = connection.query_row("SELECT count(*) FROM watch_events WHERE folder_path=?1 AND path=?2 AND kind='new_file'", params![r"D:\Camera",path.to_string_lossy()], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        assert_eq!(watch_kind(&EventKind::Create(notify::event::CreateKind::File)), Some("new_file"));
        assert_eq!(watch_kind(&EventKind::Remove(notify::event::RemoveKind::File)), Some("file_removed"));
        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }
}

//! Local AI enhancement orchestration and authoritative derived-pixel lineage.
//!
//! Inference output is never a recipe or disposable cache entry. A completed
//! result is staged, decoded, hashed and atomically promoted before one database
//! transaction makes a new physical source and its primary catalogue item visible.

use super::{encode_srgb_png, hash_file, thumbnail_from, DevelopRecipe, KeepframeError, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use image::{imageops::FilterType, DynamicImage, RgbImage};
use reqwest::{multipart, Client, StatusCode};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    f32::consts::FRAC_PI_2,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub(crate) const PROVIDER: &str = "Keepframe local restoration provider";
pub(crate) const PROVIDER_VERSION: &str = "1.0.0";
pub(crate) const PROVENANCE_VERSION: i64 = 1;
pub(crate) const MAX_OUTPUT_PIXELS: u64 = 120_000_000;

#[derive(Clone, Copy)]
pub(crate) struct ModelSpec {
    pub operation: &'static str,
    pub model: &'static str,
    pub revision: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub licence: &'static str,
    pub source: &'static str,
    pub filename: &'static str,
    pub native_scale: u32,
}

pub(crate) const DENOISE_MODEL: ModelSpec = ModelSpec {
    operation: "denoise",
    model: "SCUNet colour real PSNR",
    revision: "SCUNet@52e440a80a655b01e0b41e9dd9bfe599bc11625e+KAIR-v1.0",
    sha256: "fa78899ba2caec9d235a900e91d96c689da71c42029230c2028b00f09f809c2e",
    bytes: 71_982_841,
    licence: "Apache-2.0",
    source: "https://github.com/cszn/KAIR/releases/download/v1.0/scunet_color_real_psnr.pth",
    filename: "scunet_color_real_psnr.pth",
    native_scale: 1,
};

pub(crate) const SUPER_RESOLUTION_MODEL: ModelSpec = ModelSpec {
    operation: "super_resolution",
    model: "Real-ESRGAN x4plus",
    revision: "Real-ESRGAN@a4abfb2979a7bbff3f69f58f58ae324608821e27+v0.1.0",
    sha256: "4fa0d38905f75ac06eb49a7951b426670021be3018265fd191d2125df9d682f1",
    bytes: 67_040_989,
    licence: "BSD-3-Clause",
    source: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth",
    filename: "RealESRGAN_x4plus.pth",
    native_scale: 4,
};

pub(crate) fn model_spec(operation: &str) -> Result<ModelSpec> {
    match operation {
        "denoise" => Ok(DENOISE_MODEL),
        "super_resolution" => Ok(SUPER_RESOLUTION_MODEL),
        _ => Err(KeepframeError::Message(
            "Choose AI Denoise or Super Resolution.".into(),
        )),
    }
}

pub(crate) fn model_root() -> PathBuf {
    PathBuf::from(r"D:\AI Models\Keepframe\enhancement")
}

pub(crate) fn model_path(spec: ModelSpec) -> PathBuf {
    model_root().join(spec.filename)
}

pub(crate) fn model_installed(spec: ModelSpec) -> bool {
    let path = model_path(spec);
    fs::metadata(&path).is_ok_and(|metadata| metadata.len() == spec.bytes)
        && hash_file(&path).is_ok_and(|digest| digest.eq_ignore_ascii_case(spec.sha256))
}

pub(crate) fn recommended_tile(operation: &str, health: &EnhancementHealth) -> u32 {
    if !health.cuda_available {
        return if operation == "denoise" { 192 } else { 96 };
    }
    let gib = health.cuda_free_bytes / (1024 * 1024 * 1024);
    match (operation, gib) {
        ("denoise", 20..) => 512,
        ("denoise", 10..) => 384,
        ("denoise", _) => 256,
        (_, 20..) => 256,
        (_, 10..) => 192,
        _ => 128,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementModelStatus {
    pub operation: String,
    pub installed: bool,
    pub loaded: bool,
    pub provider: String,
    pub provider_version: String,
    pub model: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub licence: String,
    pub source: String,
    pub approximate_bytes: u64,
    pub storage_path: String,
    pub execution_provider: String,
    pub loaded_ms: u64,
    pub supported_scales: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementHealth {
    pub runtime_available: bool,
    pub busy: bool,
    pub cuda_available: bool,
    pub cuda_free_bytes: u64,
    pub cuda_total_bytes: u64,
    pub models: Vec<EnhancementModelStatus>,
}

pub(crate) fn offline_health(runtime_available: bool) -> EnhancementHealth {
    EnhancementHealth {
        runtime_available,
        busy: false,
        cuda_available: false,
        cuda_free_bytes: 0,
        cuda_total_bytes: 0,
        models: [DENOISE_MODEL, SUPER_RESOLUTION_MODEL]
            .into_iter()
            .map(|spec| EnhancementModelStatus {
                operation: spec.operation.into(),
                installed: model_installed(spec),
                loaded: false,
                provider: PROVIDER.into(),
                provider_version: PROVIDER_VERSION.into(),
                model: spec.model.into(),
                model_revision: spec.revision.into(),
                model_sha256: spec.sha256.into(),
                licence: spec.licence.into(),
                source: spec.source.into(),
                approximate_bytes: spec.bytes,
                storage_path: model_path(spec).to_string_lossy().into(),
                execution_provider: "not loaded".into(),
                loaded_ms: 0,
                supported_scales: if spec.native_scale == 1 {
                    vec![1]
                } else {
                    vec![2, 4]
                },
            })
            .collect(),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewRegion {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PreviewRegion {
    pub fn validate(self) -> Result<Self> {
        let values = [self.x, self.y, self.width, self.height];
        if values.iter().any(|value| !value.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width <= 0.0
            || self.height <= 0.0
            || self.x + self.width > 1.000_01
            || self.y + self.height > 1.000_01
        {
            return Err(KeepframeError::Message(
                "The enhancement preview region is invalid.".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementRequest {
    pub asset_id: String,
    pub operation: String,
    pub target_scale: u32,
    pub preview_region: Option<PreviewRegion>,
    #[serde(default)]
    pub allow_slow_cpu_fallback: bool,
}

impl EnhancementRequest {
    pub fn validate(&self) -> Result<ModelSpec> {
        let spec = model_spec(&self.operation)?;
        if (spec.native_scale == 1 && self.target_scale != 1)
            || (spec.native_scale == 4 && !matches!(self.target_scale, 2 | 4))
        {
            return Err(KeepframeError::Message(
                "That output scale is not supported by the selected model.".into(),
            ));
        }
        if self.asset_id.trim().is_empty() || self.asset_id.len() > 128 {
            return Err(KeepframeError::Message(
                "The enhancement source item is invalid.".into(),
            ));
        }
        if let Some(region) = self.preview_region {
            region.validate()?;
        }
        Ok(spec)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementTimings {
    pub load_ms: u64,
    pub preprocess_ms: u64,
    pub inference_ms: u64,
    pub postprocess_ms: u64,
    pub write_validate_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementProvenance {
    pub operation: String,
    pub provider: String,
    pub provider_version: String,
    pub model: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub execution_provider: String,
    pub scale: u32,
    pub tile_size: u32,
    pub overlap: u32,
    pub tile_peak_bytes: u64,
    pub timings: EnhancementTimings,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementPreview {
    pub preview_id: String,
    pub before_path: String,
    pub after_path: String,
    pub width: u32,
    pub height: u32,
    pub provenance: EnhancementProvenance,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedDerivative {
    pub derivative_id: String,
    pub asset_id: String,
    pub parent_source_id: String,
    pub root_source_id: String,
    pub source_asset_id: String,
    pub parent_asset_id: String,
    pub display_name: String,
    pub managed_relative_path: String,
    pub output_sha256: String,
    pub width: u32,
    pub height: u32,
    pub provenance: EnhancementProvenance,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnhancementJob {
    pub id: String,
    pub asset_id: String,
    pub operation: String,
    pub state: String,
    pub stage: String,
    pub tiles_complete: usize,
    pub tiles_total: usize,
    pub error: Option<String>,
    pub result_asset_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

fn axis_origins(length: u32, tile: u32, overlap: u32) -> Vec<u32> {
    if length <= tile {
        return vec![0];
    }
    let stride = tile.saturating_sub(overlap.saturating_mul(2)).max(1);
    let mut result = vec![0_u32];
    loop {
        let current = *result.last().unwrap_or(&0);
        let next = current.saturating_add(stride);
        let final_origin = length - tile;
        if next >= final_origin {
            if current != final_origin {
                result.push(final_origin);
            }
            break;
        }
        result.push(next);
    }
    result
}

pub(crate) fn plan_tiles(width: u32, height: u32, tile: u32, overlap: u32) -> Result<Vec<Tile>> {
    if width == 0 || height == 0 || tile < 64 || overlap < 8 || overlap * 2 >= tile {
        return Err(KeepframeError::Message(
            "The enhancement tile plan is unsafe.".into(),
        ));
    }
    let tile_width = tile.min(width);
    let tile_height = tile.min(height);
    let mut result = Vec::new();
    for y in axis_origins(height, tile_height, overlap.min((tile_height - 1) / 2)) {
        for x in axis_origins(width, tile_width, overlap.min((tile_width - 1) / 2)) {
            result.push(Tile {
                x,
                y,
                width: tile_width,
                height: tile_height,
            });
        }
    }
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerTimings {
    load_ms: u64,
    preprocess_ms: u64,
    inference_ms: u64,
    postprocess_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerTileResponse {
    png_base64: String,
    width: u32,
    height: u32,
    scale: u32,
    execution_provider: String,
    tile_peak_bytes: u64,
    timings: WorkerTimings,
    model: String,
    model_revision: String,
    model_sha256: String,
    provider: String,
    provider_version: String,
}

enum WorkerFailure {
    OutOfMemory,
    Other(KeepframeError),
}

#[derive(Debug, PartialEq, Eq)]
enum OomRecovery {
    SmallerTile(u32),
    CpuFallback,
    Fail,
}

fn oom_recovery(tile_size: u32, attempt: u32, allow_cpu: bool, force_cpu: bool) -> OomRecovery {
    if tile_size > 96 && attempt <= 3 {
        OomRecovery::SmallerTile((tile_size / 2).max(64))
    } else if allow_cpu && !force_cpu {
        OomRecovery::CpuFallback
    } else {
        OomRecovery::Fail
    }
}

async fn run_worker_tile(
    client: &Client,
    url: &str,
    token: &str,
    operation: &str,
    force_cpu: bool,
    tile: &RgbImage,
) -> std::result::Result<(RgbImage, WorkerTileResponse), WorkerFailure> {
    let bytes =
        encode_srgb_png(&DynamicImage::ImageRgb8(tile.clone())).map_err(WorkerFailure::Other)?;
    let form = multipart::Form::new()
        .part(
            "image",
            multipart::Part::bytes(bytes)
                .file_name("enhancement-tile.png")
                .mime_str("image/png")
                .map_err(|error| {
                    WorkerFailure::Other(KeepframeError::Message(error.to_string()))
                })?,
        )
        .text("operation", operation.to_string())
        .text("force_cpu", force_cpu.to_string());
    let response = client
        .post(format!("{url}/v1/enhancement/tile"))
        .header("X-Keepframe-Token", token)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|error| WorkerFailure::Other(KeepframeError::Network(error)))?;
    if response.status() == StatusCode::INSUFFICIENT_STORAGE {
        return Err(WorkerFailure::OutOfMemory);
    }
    let response = response.error_for_status().map_err(|error| {
        WorkerFailure::Other(KeepframeError::Message(format!(
            "The local enhancement provider rejected a tile: {error}"
        )))
    })?;
    let payload = response
        .json::<WorkerTileResponse>()
        .await
        .map_err(|error| {
            WorkerFailure::Other(KeepframeError::Message(format!(
                "The local enhancement provider returned invalid output: {error}"
            )))
        })?;
    let decoded = BASE64.decode(&payload.png_base64).map_err(|_| {
        WorkerFailure::Other(KeepframeError::Message(
            "The enhancement tile payload was not valid base64.".into(),
        ))
    })?;
    let image = image::load_from_memory_with_format(&decoded, image::ImageFormat::Png)
        .map_err(|error| WorkerFailure::Other(KeepframeError::Image(error)))?
        .to_rgb8();
    if image.dimensions() != (payload.width, payload.height)
        || payload.scale == 0
        || image.width() != tile.width() * payload.scale
        || image.height() != tile.height() * payload.scale
        || payload.model_sha256.len() != 64
    {
        return Err(WorkerFailure::Other(KeepframeError::Message(
            "The enhancement provider returned inconsistent dimensions or provenance.".into(),
        )));
    }
    Ok((image, payload))
}

fn edge_weight(
    position: u32,
    length: u32,
    overlap: u32,
    outer_before: bool,
    outer_after: bool,
) -> f32 {
    if overlap == 0 {
        return 1.0;
    }
    if !outer_before && position < overlap {
        let phase = (position as f32 + 0.5) / overlap as f32;
        return (phase * FRAC_PI_2).sin().powi(2);
    }
    if !outer_after && position + overlap >= length {
        let remaining = length.saturating_sub(position) as f32 - 0.5;
        return (remaining.max(0.0) / overlap as f32 * FRAC_PI_2)
            .sin()
            .powi(2);
    }
    1.0
}

fn composite_tiles(
    width: u32,
    height: u32,
    scale: u32,
    overlap: u32,
    tiles: &[(Tile, RgbImage)],
) -> Result<RgbImage> {
    let output_width = width
        .checked_mul(scale)
        .ok_or_else(|| KeepframeError::Message("Enhancement dimensions overflowed.".into()))?;
    let output_height = height
        .checked_mul(scale)
        .ok_or_else(|| KeepframeError::Message("Enhancement dimensions overflowed.".into()))?;
    let pixels = u64::from(output_width) * u64::from(output_height);
    if pixels > MAX_OUTPUT_PIXELS {
        return Err(KeepframeError::Message(format!(
            "The requested enhancement would create {pixels} pixels; the safe limit is {MAX_OUTPUT_PIXELS}."
        )));
    }
    let count = usize::try_from(pixels).map_err(|_| {
        KeepframeError::Message("Enhancement dimensions exceed addressable memory.".into())
    })?;
    let mut accumulation = vec![[0.0_f32; 3]; count];
    let mut weights = vec![0.0_f32; count];
    let scaled_overlap = overlap * scale;
    for (tile, image) in tiles {
        let expected = (tile.width * scale, tile.height * scale);
        if image.dimensions() != expected {
            return Err(KeepframeError::Message(
                "A processed tile had unexpected dimensions.".into(),
            ));
        }
        for (local_x, local_y, pixel) in image.enumerate_pixels() {
            let global_x = tile.x * scale + local_x;
            let global_y = tile.y * scale + local_y;
            let horizontal = edge_weight(
                local_x,
                image.width(),
                scaled_overlap,
                tile.x == 0,
                tile.x + tile.width == width,
            );
            let vertical = edge_weight(
                local_y,
                image.height(),
                scaled_overlap,
                tile.y == 0,
                tile.y + tile.height == height,
            );
            let weight = horizontal * vertical;
            let index = (global_y * output_width + global_x) as usize;
            weights[index] += weight;
            for channel in 0..3 {
                accumulation[index][channel] += pixel[channel] as f32 * weight;
            }
        }
    }
    let mut output = RgbImage::new(output_width, output_height);
    for (index, pixel) in output.pixels_mut().enumerate() {
        let weight = weights[index].max(f32::EPSILON);
        for channel in 0..3 {
            pixel[channel] = (accumulation[index][channel] / weight)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    Ok(output)
}

#[derive(Debug)]
pub(crate) struct ProcessedImage {
    pub image: RgbImage,
    pub provenance: EnhancementProvenance,
}

pub(crate) async fn process_tiled<F, C>(
    source: &DynamicImage,
    request: &EnhancementRequest,
    initial_tile_size: u32,
    url: &str,
    token: &str,
    mut progress: F,
    cancelled: C,
) -> Result<ProcessedImage>
where
    F: FnMut(usize, usize, &str),
    C: Fn() -> bool,
{
    let spec = request.validate()?;
    let source = source.to_rgb8();
    let output_pixels = u64::from(source.width())
        * u64::from(source.height())
        * u64::from(request.target_scale).pow(2);
    if output_pixels > MAX_OUTPUT_PIXELS {
        return Err(KeepframeError::Message(format!(
            "This enhancement would create {output_pixels} pixels and exceed the safe {MAX_OUTPUT_PIXELS}-pixel limit. Choose a smaller source or scale."
        )));
    }
    let started = std::time::Instant::now();
    let client = Client::new();
    let overlap = if request.operation == "denoise" {
        32
    } else {
        16
    };
    let mut tile_size = initial_tile_size.clamp(64, 1024);
    let mut force_cpu = false;
    let mut attempts = 0;
    loop {
        let tiles = plan_tiles(source.width(), source.height(), tile_size, overlap)?;
        let mut processed = Vec::with_capacity(tiles.len());
        let mut aggregate = EnhancementTimings {
            load_ms: 0,
            preprocess_ms: 0,
            inference_ms: 0,
            postprocess_ms: 0,
            write_validate_ms: 0,
            total_ms: 0,
        };
        let mut peak = 0;
        let mut identity: Option<WorkerTileResponse> = None;
        let mut oom = false;
        for (index, tile) in tiles.iter().enumerate() {
            if cancelled() {
                return Err(KeepframeError::Message(
                    "AI enhancement was cancelled.".into(),
                ));
            }
            progress(
                index,
                tiles.len(),
                if index == 0 {
                    "Loading model"
                } else {
                    "Processing tiles"
                },
            );
            let crop = image::imageops::crop_imm(&source, tile.x, tile.y, tile.width, tile.height)
                .to_image();
            match run_worker_tile(&client, url, token, &request.operation, force_cpu, &crop).await {
                Ok((mut output, response)) => {
                    if response.scale != spec.native_scale
                        || response.model_sha256 != spec.sha256
                        || response.provider != PROVIDER
                        || response.provider_version != PROVIDER_VERSION
                    {
                        return Err(KeepframeError::Message(
                            "The enhancement provider identity did not match the pinned model."
                                .into(),
                        ));
                    }
                    if request.target_scale != response.scale {
                        output = image::imageops::resize(
                            &output,
                            tile.width * request.target_scale,
                            tile.height * request.target_scale,
                            FilterType::Lanczos3,
                        );
                    }
                    aggregate.load_ms += response.timings.load_ms;
                    aggregate.preprocess_ms += response.timings.preprocess_ms;
                    aggregate.inference_ms += response.timings.inference_ms;
                    aggregate.postprocess_ms += response.timings.postprocess_ms;
                    peak = peak.max(response.tile_peak_bytes);
                    identity.get_or_insert(response);
                    processed.push((*tile, output));
                    progress(index + 1, tiles.len(), "Processing tiles");
                }
                Err(WorkerFailure::OutOfMemory) => {
                    oom = true;
                    break;
                }
                Err(WorkerFailure::Other(error)) => return Err(error),
            }
        }
        if oom {
            attempts += 1;
            match oom_recovery(
                tile_size,
                attempts,
                request.allow_slow_cpu_fallback,
                force_cpu,
            ) {
                OomRecovery::SmallerTile(next) => {
                    tile_size = next;
                    progress(
                        0,
                        tiles.len(),
                        "Retrying with smaller tiles after CUDA memory pressure",
                    );
                    continue;
                }
                OomRecovery::CpuFallback => {
                    force_cpu = true;
                    tile_size = 64;
                    attempts = 0;
                    progress(0, tiles.len(), "Using slow CPU fallback");
                    continue;
                }
                OomRecovery::Fail => {}
            }
            return Err(KeepframeError::Message(
                "CUDA ran out of memory after bounded smaller-tile retries. No derivative was created. Enable the explicitly slow CPU fallback or reduce the input size.".into(),
            ));
        }
        if cancelled() {
            return Err(KeepframeError::Message(
                "AI enhancement was cancelled.".into(),
            ));
        }
        progress(tiles.len(), tiles.len(), "Blending tiles");
        let image = composite_tiles(
            source.width(),
            source.height(),
            request.target_scale,
            overlap,
            &processed,
        )?;
        let identity = identity
            .ok_or_else(|| KeepframeError::Message("The enhancement produced no tiles.".into()))?;
        aggregate.total_ms = started.elapsed().as_millis() as u64;
        return Ok(ProcessedImage {
            image,
            provenance: EnhancementProvenance {
                operation: request.operation.clone(),
                provider: identity.provider,
                provider_version: identity.provider_version,
                model: identity.model,
                model_revision: identity.model_revision,
                model_sha256: identity.model_sha256,
                execution_provider: identity.execution_provider,
                scale: request.target_scale,
                tile_size,
                overlap,
                tile_peak_bytes: peak,
                timings: aggregate,
            },
        });
    }
}

pub(crate) fn crop_preview(source: &DynamicImage, region: PreviewRegion) -> Result<DynamicImage> {
    let region = region.validate()?;
    let width = source.width();
    let height = source.height();
    let x = (region.x * width as f32).floor() as u32;
    let y = (region.y * height as f32).floor() as u32;
    let requested_width = ((region.width * width as f32).round() as u32).min(1024);
    let requested_height = ((region.height * height as f32).round() as u32).min(1024);
    let crop_width = requested_width.clamp(32, width.saturating_sub(x));
    let crop_height = requested_height.clamp(32, height.saturating_sub(y));
    Ok(source.crop_imm(x, y, crop_width, crop_height))
}

pub(crate) fn create_job(connection: &Connection, request: &EnhancementRequest) -> Result<String> {
    request.validate()?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO ai_enhancement_jobs(id,asset_id,operation,state,stage,request_json,created_at,updated_at)VALUES(?1,?2,?3,'preparing','Preparing',?4,?5,?5)",
        params![id,request.asset_id,request.operation,serde_json::to_string(request)?,now],
    )?;
    Ok(id)
}

pub(crate) fn update_job(
    connection: &Connection,
    id: &str,
    state: &str,
    stage: &str,
    current: usize,
    total: usize,
    error: Option<&str>,
) -> Result<()> {
    connection.execute(
        "UPDATE ai_enhancement_jobs SET state=?2,stage=?3,tiles_complete=?4,tiles_total=?5,error=?6,updated_at=?7 WHERE id=?1",
        params![id,state,stage,current as i64,total as i64,error,Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn job_cancelled(connection: &Connection, id: &str) -> bool {
    connection
        .query_row(
            "SELECT cancel_requested!=0 FROM ai_enhancement_jobs WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap_or(true)
}

pub(crate) fn cancel_job(connection: &Connection, id: &str) -> Result<()> {
    let changed = connection.execute(
        "UPDATE ai_enhancement_jobs SET cancel_requested=1,updated_at=?2 WHERE id=?1 AND state NOT IN('complete','failed','cancelled')",
        params![id,Utc::now().to_rfc3339()],
    )?;
    if changed == 0 {
        return Err(KeepframeError::Message(
            "That enhancement job is no longer cancellable.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn list_jobs(
    connection: &Connection,
    asset_id: Option<&str>,
) -> Result<Vec<EnhancementJob>> {
    let mut statement = connection.prepare(
        "SELECT id,asset_id,operation,state,stage,tiles_complete,tiles_total,error,result_asset_id,created_at,updated_at FROM ai_enhancement_jobs WHERE (?1 IS NULL OR asset_id=?1) ORDER BY created_at DESC LIMIT 100",
    )?;
    let jobs = statement
        .query_map([asset_id], |row| {
            Ok(EnhancementJob {
                id: row.get(0)?,
                asset_id: row.get(1)?,
                operation: row.get(2)?,
                state: row.get(3)?,
                stage: row.get(4)?,
                tiles_complete: row.get::<_, i64>(5)? as usize,
                tiles_total: row.get::<_, i64>(6)? as usize,
                error: row.get(7)?,
                result_asset_id: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(jobs)
}

#[derive(Debug)]
pub(crate) struct SourceIdentity {
    pub source_id: String,
    pub root_source_id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub expected_dimensions: Option<(u32, u32)>,
}

pub(crate) fn source_identity(connection: &Connection, asset_id: &str) -> Result<SourceIdentity> {
    connection
        .query_row(
            "SELECT s.id,COALESCE((SELECT d.root_source_id FROM ai_derivatives d WHERE d.derived_source_id=s.id),s.id),r.path,r.sha256,s.width,s.height
             FROM assets a JOIN sources s ON s.id=a.source_id JOIN representations r ON r.id=(SELECT r2.id FROM representations r2 WHERE r2.source_id=s.id ORDER BY r2.is_raw DESC,r2.path LIMIT 1)
             WHERE a.id=?1 AND a.trashed_at IS NULL",
            [asset_id],
            |row| {
                Ok(SourceIdentity {
                    source_id: row.get(0)?,
                    root_source_id: row.get(1)?,
                    path: PathBuf::from(row.get::<_, String>(2)?),
                    sha256: row.get(3)?,
                    expected_dimensions: row
                        .get::<_, Option<u32>>(4)?
                        .zip(row.get::<_, Option<u32>>(5)?),
                })
            },
        )
        .map_err(KeepframeError::from)
}

fn display_name(
    connection: &Connection,
    asset_id: &str,
    provenance: &EnhancementProvenance,
) -> Result<(String, String)> {
    let filename: String = connection.query_row(
        "SELECT s.filename FROM assets a JOIN sources s ON s.id=a.source_id WHERE a.id=?1",
        [asset_id],
        |row| row.get(0),
    )?;
    let stem = Path::new(&filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Photograph");
    let label = if provenance.operation == "denoise" {
        "Denoised".to_string()
    } else {
        format!("{}× Super Resolution", provenance.scale)
    };
    Ok((format!("{stem} — {label}.png"), label))
}

pub(crate) fn persist_accepted(
    root: &Path,
    connection: &mut Connection,
    job_id: &str,
    source_asset_id: &str,
    image: &RgbImage,
    mut provenance: EnhancementProvenance,
) -> Result<AcceptedDerivative> {
    if job_cancelled(connection, job_id) {
        return Err(KeepframeError::Message(
            "AI enhancement was cancelled before acceptance; no derivative was promoted.".into(),
        ));
    }
    let identity = source_identity(connection, source_asset_id)?;
    if !identity.path.is_file() || hash_file(&identity.path)? != identity.sha256 {
        return Err(KeepframeError::Message(
            "The source changed before enhancement acceptance; no derivative was created.".into(),
        ));
    }
    let derivative_id = Uuid::new_v4().to_string();
    let derived_source_id = Uuid::new_v4().to_string();
    let asset_id = Uuid::new_v4().to_string();
    let representation_id = Uuid::new_v4().to_string();
    let (filename, label) = display_name(connection, source_asset_id, &provenance)?;
    let relative = PathBuf::from(".keepframe")
        .join("derivatives")
        .join("ai")
        .join(&identity.source_id)
        .join(format!("{derived_source_id}.png"));
    let output = root.join(&relative);
    let stage_directory = root.join(".keepframe/staging/enhancement");
    fs::create_dir_all(&stage_directory)?;
    fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| KeepframeError::Message("The derivative path has no parent.".into()))?,
    )?;
    let temporary = stage_directory.join(format!("{job_id}.partial.png"));
    let write_started = std::time::Instant::now();
    let thumbnail = root
        .join(".keepframe/thumbnails")
        .join(format!("{asset_id}.jpg"));
    let staged: Result<(String, i64)> = (|| {
        fs::write(
            &temporary,
            encode_srgb_png(&DynamicImage::ImageRgb8(image.clone()))?,
        )?;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        let reopened = image::open(&temporary)?.to_rgb8();
        if reopened.dimensions() != image.dimensions() {
            return Err(KeepframeError::Message(
                "The staged enhancement did not reopen at the expected dimensions.".into(),
            ));
        }
        let output_sha256 = hash_file(&temporary)?;
        thumbnail_from(&temporary, &thumbnail)?;
        if job_cancelled(connection, job_id) {
            return Err(KeepframeError::Message(
                "AI enhancement was cancelled before promotion; no derivative was accepted.".into(),
            ));
        }
        fs::rename(&temporary, &output)?;
        Ok((output_sha256, fs::metadata(&output)?.len() as i64))
    })();
    let (output_sha256, output_bytes) = match staged {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&output);
            let _ = fs::remove_file(&thumbnail);
            return Err(error);
        }
    };
    provenance.timings.write_validate_ms = write_started.elapsed().as_millis() as u64;
    provenance.timings.total_ms += provenance.timings.write_validate_ms;
    if job_cancelled(connection, job_id) {
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&thumbnail);
        return Err(KeepframeError::Message(
            "AI enhancement was cancelled before catalogue acceptance; the staged output was discarded."
                .into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    let parameters = serde_json::to_string(&serde_json::json!({
        "targetScale": provenance.scale,
        "tileSize": provenance.tile_size,
        "overlap": provenance.overlap,
        "precision": if provenance.execution_provider.contains("FP16") { "FP16" } else { "FP32" },
        "faceEnhancement": false,
        "detailModel": false,
        "totalMs": provenance.timings.total_ms,
    }))?;
    let transaction = (|| -> Result<()> {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let cancelled = tx.query_row(
            "SELECT cancel_requested!=0 FROM ai_enhancement_jobs WHERE id=?1",
            [job_id],
            |row| row.get::<_, bool>(0),
        )?;
        if cancelled {
            return Err(KeepframeError::Message(
                "AI enhancement was cancelled at the acceptance boundary; no derivative was registered."
                    .into(),
            ));
        }
        tx.execute(
            "INSERT INTO sources(id,filename,captured_at,date_fallback,camera,lens,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,created_at,trashed_at,versions_collapsed,focal_length,aperture,iso)
             SELECT ?1,?2,s.captured_at,s.date_fallback,s.camera,s.lens,?3,?4,s.latitude,s.longitude,s.embedded_latitude,s.embedded_longitude,s.manual_latitude,s.manual_longitude,s.location_source,'available',?5,?6,?5,NULL,0,s.focal_length,s.aperture,s.iso
             FROM assets a JOIN sources s ON s.id=a.source_id WHERE a.id=?7",
            params![derived_source_id,filename,image.width(),image.height(),now,thumbnail.to_string_lossy(),source_asset_id],
        )?;
        tx.execute(
            "INSERT INTO assets(id,filename,decision,rating,title,caption,copyright,creator,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,created_at,source_id,is_primary,version_name,version_index)
             SELECT ?1,?2,'undecided',0,NULL,NULL,a.copyright,a.creator,s.captured_at,s.date_fallback,s.camera,?3,?4,s.latitude,s.longitude,s.embedded_latitude,s.embedded_longitude,s.manual_latitude,s.manual_longitude,s.location_source,'available',?5,?6,?5,?7,1,?8,1
             FROM assets a JOIN sources s ON s.id=a.source_id WHERE a.id=?9",
            params![asset_id,filename,image.width(),image.height(),now,thumbnail.to_string_lossy(),derived_source_id,label,source_asset_id],
        )?;
        tx.execute(
            "INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw,source_id)VALUES(?1,?2,?3,?4,'png',?5,?6,0,?7)",
            params![representation_id,asset_id,output.to_string_lossy(),output_sha256,Path::new(&filename).file_stem().and_then(|value| value.to_str()).unwrap_or("enhanced"),output_bytes,derived_source_id],
        )?;
        if let Some(recipe_json) = tx
            .query_row(
                "SELECT recipe_json FROM develop_recipes WHERE asset_id=?1",
                [source_asset_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let recipe = serde_json::from_str::<DevelopRecipe>(&recipe_json)?.validate()?;
            tx.execute(
                "INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES(?1,?2,?3,?4)",
                params![asset_id,recipe.schema_version,serde_json::to_string(&recipe)?,now],
            )?;
        }
        tx.execute(
            "INSERT INTO ai_derivatives(id,parent_source_id,root_source_id,derived_source_id,source_asset_id,operation,provider,provider_version,model_id,model_revision,model_sha256,execution_provider,parameters_json,scale,tile_size,overlap,source_sha256,output_sha256,output_width,output_height,pixel_format,bit_depth,managed_relative_path,provenance_version,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,'sRGB PNG',8,?21,?22,?23)",
            params![derivative_id,identity.source_id,identity.root_source_id,derived_source_id,source_asset_id,provenance.operation,provenance.provider,provenance.provider_version,provenance.model,provenance.model_revision,provenance.model_sha256,provenance.execution_provider,parameters,provenance.scale,provenance.tile_size,provenance.overlap,identity.sha256,output_sha256,image.width(),image.height(),relative.to_string_lossy(),PROVENANCE_VERSION,now],
        )?;
        // Derivatives intentionally reuse the root source semantic identity by
        // default; do not spend storage indexing materially equivalent pixels.
        tx.execute(
            "DELETE FROM semantic_index_queue WHERE source_id=?1",
            [&derived_source_id],
        )?;
        tx.execute(
            "UPDATE ai_enhancement_jobs SET state='complete',stage='Complete',result_asset_id=?2,temporary_path=NULL,updated_at=?3 WHERE id=?1",
            params![job_id,asset_id,now],
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = transaction {
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&thumbnail);
        return Err(error);
    }
    if hash_file(&identity.path)? != identity.sha256 {
        let _ = connection.execute("DELETE FROM sources WHERE id=?1", [&derived_source_id]);
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&thumbnail);
        return Err(KeepframeError::Message(
            "The protected source changed during acceptance; the new derivative was rolled back."
                .into(),
        ));
    }
    let parent_asset_id = connection.query_row(
        "SELECT id FROM assets WHERE source_id=?1 AND is_primary=1 ORDER BY id LIMIT 1",
        [&identity.source_id],
        |row| row.get(0),
    )?;
    Ok(AcceptedDerivative {
        derivative_id,
        asset_id,
        parent_source_id: identity.source_id,
        root_source_id: identity.root_source_id,
        source_asset_id: source_asset_id.into(),
        parent_asset_id,
        display_name: label,
        managed_relative_path: relative.to_string_lossy().into(),
        output_sha256,
        width: image.width(),
        height: image.height(),
        provenance,
    })
}

pub(crate) fn derivative_for_asset(
    connection: &Connection,
    asset_id: &str,
) -> Result<Option<AcceptedDerivative>> {
    connection.query_row(
        "SELECT d.id,a.id,d.parent_source_id,d.root_source_id,d.source_asset_id,(SELECT pa.id FROM assets pa WHERE pa.source_id=d.parent_source_id AND pa.is_primary=1 ORDER BY pa.id LIMIT 1),COALESCE(a.version_name,s.filename),d.managed_relative_path,d.output_sha256,d.output_width,d.output_height,d.operation,d.provider,d.provider_version,d.model_id,d.model_revision,d.model_sha256,d.execution_provider,d.scale,d.tile_size,d.overlap,d.parameters_json
         FROM assets a JOIN sources s ON s.id=a.source_id JOIN ai_derivatives d ON d.derived_source_id=s.id WHERE a.id=?1",
        [asset_id],
        |row| {
            let operation: String = row.get(11)?;
            let parameters: serde_json::Value = serde_json::from_str(&row.get::<_, String>(21)?).unwrap_or_default();
            Ok(AcceptedDerivative {
                derivative_id: row.get(0)?,asset_id: row.get(1)?,parent_source_id: row.get(2)?,root_source_id: row.get(3)?,source_asset_id:row.get(4)?,parent_asset_id:row.get(5)?,display_name: row.get(6)?,managed_relative_path: row.get(7)?,output_sha256: row.get(8)?,width: row.get(9)?,height: row.get(10)?,
                provenance: EnhancementProvenance { operation,provider:row.get(12)?,provider_version:row.get(13)?,model:row.get(14)?,model_revision:row.get(15)?,model_sha256:row.get(16)?,execution_provider:row.get(17)?,scale:row.get(18)?,tile_size:row.get(19)?,overlap:row.get(20)?,tile_peak_bytes:0,timings:EnhancementTimings{load_ms:0,preprocess_ms:0,inference_ms:0,postprocess_ms:0,write_validate_ms:0,total_ms:parameters.get("totalMs").and_then(|value|value.as_u64()).unwrap_or(0)}}
            })
        },
    ).optional().map_err(KeepframeError::from)
}

pub(crate) fn delete_derivative(
    connection: &mut Connection,
    root: &Path,
    asset_id: &str,
    confirmed: bool,
) -> Result<String> {
    if !confirmed {
        return Err(KeepframeError::Message(
            "Deleting an accepted derivative requires explicit confirmation.".into(),
        ));
    }
    let (derived_source_id,parent_asset_id,stored): (String,String,String) = connection.query_row(
        "SELECT d.derived_source_id,(SELECT id FROM assets WHERE source_id=d.parent_source_id AND is_primary=1),r.path FROM ai_derivatives d JOIN assets a ON a.source_id=d.derived_source_id JOIN representations r ON r.source_id=d.derived_source_id WHERE a.id=?1 AND a.is_primary=1",
        [asset_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    let stored = PathBuf::from(stored);
    let canonical_root = root.canonicalize()?;
    let canonical_stored = stored.canonicalize()?;
    if !canonical_stored.starts_with(canonical_root.join(".keepframe/derivatives/ai")) {
        return Err(KeepframeError::Message(
            "Refusing to remove a derivative outside managed AI storage.".into(),
        ));
    }
    let trash_dir = root.join(".keepframe/Trash/ai");
    fs::create_dir_all(&trash_dir)?;
    let trashed = trash_dir.join(format!(
        "{}-{}",
        Utc::now().format("%Y%m%d%H%M%S"),
        stored
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("derivative.png")
    ));
    fs::rename(&stored, &trashed)?;
    let result = (|| -> Result<()> {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM sources WHERE id=?1", [&derived_source_id])?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::rename(&trashed, &stored);
        return Err(error);
    }
    Ok(parent_asset_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::sync::{atomic::AtomicU64, Mutex};

    #[test]
    fn tile_plans_cover_edges_without_duplicates() {
        let tiles = plan_tiles(997, 613, 256, 24).unwrap();
        assert_eq!(tiles.first().unwrap().x, 0);
        assert!(tiles.iter().any(|tile| tile.x + tile.width == 997));
        assert!(tiles.iter().any(|tile| tile.y + tile.height == 613));
        let unique = tiles
            .iter()
            .map(|tile| (tile.x, tile.y))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), tiles.len());
    }

    #[test]
    fn weighted_overlap_is_seam_free_for_identity_gradient_checker_and_diagonal() {
        for source in [
            RgbImage::from_fn(301, 211, |x, y| {
                Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
            }),
            RgbImage::from_fn(301, 211, |x, y| {
                let value = if (x / 7 + y / 7) % 2 == 0 { 240 } else { 12 };
                Rgb([value, value, value])
            }),
            RgbImage::from_fn(301, 211, |x, y| {
                let value = if x.abs_diff(y) < 3 { 255 } else { 20 };
                Rgb([value, 80, 180])
            }),
            RgbImage::from_fn(301, 211, |x, y| {
                let value = ((x * 37 + y * 71 + x * y) % 251) as u8;
                Rgb([value, value.wrapping_mul(3), value.wrapping_mul(7)])
            }),
        ] {
            let plan = plan_tiles(source.width(), source.height(), 128, 16).unwrap();
            let tiles = plan
                .iter()
                .map(|tile| {
                    (
                        *tile,
                        image::imageops::crop_imm(&source, tile.x, tile.y, tile.width, tile.height)
                            .to_image(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                composite_tiles(source.width(), source.height(), 1, 16, &tiles).unwrap(),
                source
            );
        }
    }

    #[test]
    fn two_and_four_times_dimensions_are_exact() {
        let source = RgbImage::from_pixel(65, 47, Rgb([30, 80, 150]));
        for scale in [2, 4] {
            let tile = Tile {
                x: 0,
                y: 0,
                width: 65,
                height: 47,
            };
            let enlarged =
                image::imageops::resize(&source, 65 * scale, 47 * scale, FilterType::Nearest);
            let result = composite_tiles(65, 47, scale, 8, &[(tile, enlarged)]).unwrap();
            assert_eq!(result.dimensions(), (65 * scale, 47 * scale));
        }
    }

    #[test]
    fn request_validation_rejects_fake_scales_and_detail_mode() {
        let base = EnhancementRequest {
            asset_id: "asset".into(),
            operation: "super_resolution".into(),
            target_scale: 3,
            preview_region: None,
            allow_slow_cpu_fallback: false,
        };
        assert!(base.validate().is_err());
        assert!(EnhancementRequest {
            operation: "detail".into(),
            target_scale: 1,
            ..base
        }
        .validate()
        .is_err());
    }

    #[test]
    fn oom_policy_reduces_tiles_caps_retries_and_requires_explicit_cpu_fallback() {
        assert_eq!(
            oom_recovery(512, 1, false, false),
            OomRecovery::SmallerTile(256)
        );
        assert_eq!(
            oom_recovery(256, 2, false, false),
            OomRecovery::SmallerTile(128)
        );
        assert_eq!(
            oom_recovery(128, 3, false, false),
            OomRecovery::SmallerTile(64)
        );
        assert_eq!(oom_recovery(64, 4, false, false), OomRecovery::Fail);
        assert_eq!(oom_recovery(64, 4, true, false), OomRecovery::CpuFallback);
        assert_eq!(oom_recovery(64, 1, true, true), OomRecovery::Fail);
    }

    #[test]
    fn preview_crop_is_positioned_and_capped_for_responsive_inference() {
        let source = DynamicImage::new_rgb8(6000, 4000);
        let crop = crop_preview(
            &source,
            PreviewRegion {
                x: 0.5,
                y: 0.5,
                width: 0.25,
                height: 0.25,
            },
        )
        .unwrap();
        assert_eq!((crop.width(), crop.height()), (1024, 1000));
    }

    #[test]
    fn accepted_result_is_a_separate_physical_source_with_recipe_lineage_and_safe_delete() {
        let root = std::env::temp_dir().join(format!("keepframe-ai-derivative-{}", Uuid::new_v4()));
        super::super::initialise_layout(&root).unwrap();
        let original = root.join("Originals/2026/01/01/source.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        let original_image =
            RgbImage::from_fn(24, 16, |x, y| Rgb([(x * 7) as u8, (y * 11) as u8, 90]));
        fs::write(
            &original,
            encode_srgb_png(&DynamicImage::ImageRgb8(original_image.clone())).unwrap(),
        )
        .unwrap();
        let original_hash = hash_file(&original).unwrap();
        let original_bytes = fs::metadata(&original).unwrap().len() as i64;
        let thumbnail = root.join(".keepframe/thumbnails/source.jpg");
        thumbnail_from(&original, &thumbnail).unwrap();
        let mut connection = super::super::open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,decision,rating,captured_at,date_fallback,width,height,location_source,missing_state,thumbnail_path,created_at)VALUES('source-asset','source.png','keep',5,'2026-01-01T00:00:00Z',0,24,16,'none','available',?1,'2026-01-01T00:00:00Z')",[thumbnail.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('source-rep','source-asset',?1,?2,'png','source',?3,0)",params![original.to_string_lossy(),original_hash,original_bytes]).unwrap();
        let recipe = DevelopRecipe {
            schema_version: 3,
            settings: super::super::BasicAdjustments::neutral(),
            advanced: super::super::advanced_develop::AdvancedDevelopSettings::default(),
            masks: vec![super::super::DevelopMask {
                id: "manual-mask".into(),
                name: "Protected edge".into(),
                enabled: true,
                inverted: false,
                opacity: 1.0,
                feather: 0.5,
                geometry: super::super::MaskGeometry::Linear {
                    start: super::super::MaskPoint { x: 0.0, y: 0.5 },
                    end: super::super::MaskPoint { x: 1.0, y: 0.5 },
                },
                adjustments: super::super::LocalAdjustments {
                    exposure: 0.2,
                    ..Default::default()
                },
            }],
        };
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES('source-asset',3,?1,'2026-01-01T00:00:00Z')",[serde_json::to_string(&recipe).unwrap()]).unwrap();
        let request = EnhancementRequest {
            asset_id: "source-asset".into(),
            operation: "denoise".into(),
            target_scale: 1,
            preview_region: None,
            allow_slow_cpu_fallback: false,
        };
        let provenance = EnhancementProvenance {
            operation: "denoise".into(),
            provider: PROVIDER.into(),
            provider_version: PROVIDER_VERSION.into(),
            model: DENOISE_MODEL.model.into(),
            model_revision: DENOISE_MODEL.revision.into(),
            model_sha256: DENOISE_MODEL.sha256.into(),
            execution_provider: "CPU FP32 test".into(),
            scale: 1,
            tile_size: 192,
            overlap: 32,
            tile_peak_bytes: 0,
            timings: EnhancementTimings {
                load_ms: 1,
                preprocess_ms: 1,
                inference_ms: 1,
                postprocess_ms: 1,
                write_validate_ms: 0,
                total_ms: 4,
            },
        };
        let cancelled_job = create_job(&connection, &request).unwrap();
        cancel_job(&connection, &cancelled_job).unwrap();
        assert!(persist_accepted(
            &root,
            &mut connection,
            &cancelled_job,
            "source-asset",
            &original_image,
            provenance.clone(),
        )
        .is_err());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM ai_derivatives", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        let job = create_job(&connection, &request).unwrap();
        let result = persist_accepted(
            &root,
            &mut connection,
            &job,
            "source-asset",
            &original_image,
            provenance,
        )
        .unwrap();
        assert_ne!(result.asset_id, "source-asset");
        assert_eq!(result.root_source_id, result.parent_source_id);
        assert!(root.join(&result.managed_relative_path).is_file());
        assert_eq!(hash_file(&original).unwrap(), original_hash);
        let copied: String = connection
            .query_row(
                "SELECT recipe_json FROM develop_recipes WHERE asset_id=?1",
                [&result.asset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<DevelopRecipe>(&copied).unwrap(),
            recipe
        );
        assert_eq!(connection.query_row("SELECT count(*) FROM semantic_index_queue WHERE source_id=(SELECT source_id FROM assets WHERE id=?1)",[&result.asset_id],|row|row.get::<_,i64>(0)).unwrap(),0);
        let portable_dir = root.join("portable");
        fs::create_dir_all(&portable_dir).unwrap();
        let portable = super::super::interoperability::export_portable_catalogue(
            &connection,
            &root,
            &portable_dir,
        )
        .unwrap();
        let portable_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&portable).unwrap()).unwrap();
        assert!(portable_json["assets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|asset| asset["aiDerivative"]["outputSha256"] == result.output_sha256));
        connection
            .execute(
                "DELETE FROM ai_derivatives WHERE id=?1",
                [&result.derivative_id],
            )
            .unwrap();
        assert!(derivative_for_asset(&connection, &result.asset_id)
            .unwrap()
            .is_none());
        super::super::interoperability::import_portable_catalogue(&mut connection, &portable)
            .unwrap();
        assert!(derivative_for_asset(&connection, &result.asset_id)
            .unwrap()
            .is_some());
        let virtual_item = super::super::organisation::create_version_in(
            &mut connection,
            super::super::organisation::CreateVersionRequest {
                item_id: result.asset_id.clone(),
                mode: "current".into(),
                name: Some("Enhanced variant".into()),
            },
        )
        .unwrap();
        assert_ne!(virtual_item, result.asset_id);
        assert_eq!(
            connection
                .query_row(
                    "SELECT source_id FROM assets WHERE id=?1",
                    [&virtual_item],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            connection
                .query_row(
                    "SELECT source_id FROM assets WHERE id=?1",
                    [&result.asset_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        );
        let export_dir = root.join("exported");
        fs::create_dir_all(&export_dir).unwrap();
        for format in ["jpeg", "png", "tiff"] {
            let config = super::super::professional_export::ExportConfig {
                format: format.into(),
                sharpening: "none".into(),
                filename_template: format!("accepted-{format}"),
                ..Default::default()
            };
            let report = super::super::professional_export::run_batch(
                &root,
                super::super::professional_export::ExportRequest {
                    asset_ids: vec![result.asset_id.clone()],
                    destination: export_dir.to_string_lossy().into(),
                    config,
                },
                &Mutex::new(super::super::RendererCaches::default()),
                &AtomicU64::new(1),
                1,
                None,
            )
            .unwrap();
            assert_eq!((report.complete, report.failed), (1, 0));
        }
        let managed = root.join(&result.managed_relative_path);
        let accepted_bytes = fs::read(&managed).unwrap();
        let mut modified_bytes = accepted_bytes.clone();
        let last = modified_bytes.len() - 1;
        modified_bytes[last] ^= 1;
        fs::write(&managed, &modified_bytes).unwrap();
        let integrity =
            super::super::interoperability::rescan_library(&mut connection, &root).unwrap();
        assert_eq!(integrity.modified_ai_derivatives, 1);
        assert_eq!(integrity.modified_originals, 0);
        fs::write(&managed, &accepted_bytes).unwrap();
        let missing = managed.with_extension("missing");
        fs::rename(&managed, &missing).unwrap();
        let integrity =
            super::super::interoperability::rescan_library(&mut connection, &root).unwrap();
        assert_eq!(integrity.missing_ai_derivatives, 1);
        assert_eq!(integrity.missing_originals, 0);
        fs::rename(&missing, &managed).unwrap();
        let parent = delete_derivative(&mut connection, &root, &result.asset_id, true).unwrap();
        assert_eq!(parent, "source-asset");
        assert!(original.is_file());
        assert!(derivative_for_asset(&connection, &result.asset_id)
            .unwrap()
            .is_none());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

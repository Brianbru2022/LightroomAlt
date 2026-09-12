use super::*;
use image::{
    codecs::{
        jpeg::{JpegEncoder, PixelDensity},
        png::{CompressionType, FilterType, PngEncoder},
        tiff::TiffEncoder,
    },
    imageops::FilterType as ResizeFilter,
    ExtendedColorType, ImageEncoder,
};

const EXPORT_PRESET_FILE_MAX_BYTES: u64 = 128 * 1024;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportConfig {
    pub(crate) schema_version: i64,
    pub(crate) format: String,
    pub(crate) jpeg_quality: u8,
    pub(crate) png_compression: String,
    pub(crate) resize_mode: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) percentage: u32,
    pub(crate) no_enlarge: bool,
    pub(crate) ppi: u16,
    pub(crate) sharpening: String,
    pub(crate) metadata: String,
    pub(crate) include_location: bool,
    pub(crate) include_keywords: bool,
    pub(crate) include_rating: bool,
    pub(crate) filename_template: String,
    pub(crate) custom_text: String,
    pub(crate) sequence_start: u32,
    pub(crate) sequence_padding: u8,
    pub(crate) collision: String,
    pub(crate) colour_space: String,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            format: "jpeg".into(),
            jpeg_quality: 90,
            png_compression: "balanced".into(),
            resize_mode: "original".into(),
            width: 2400,
            height: 1600,
            percentage: 100,
            no_enlarge: true,
            ppi: 300,
            sharpening: "standard".into(),
            metadata: "all".into(),
            include_location: false,
            include_keywords: true,
            include_rating: true,
            filename_template: "{stem}-{sequence}".into(),
            custom_text: String::new(),
            sequence_start: 1,
            sequence_padding: 3,
            collision: "unique".into(),
            colour_space: "srgb".into(),
        }
    }
}

impl ExportConfig {
    pub(crate) fn validate(mut self) -> Result<Self> {
        self.format = self.format.to_ascii_lowercase();
        self.resize_mode = self.resize_mode.to_ascii_lowercase();
        self.sharpening = self.sharpening.to_ascii_lowercase();
        self.metadata = self.metadata.to_ascii_lowercase();
        self.collision = self.collision.to_ascii_lowercase();
        self.colour_space = self.colour_space.to_ascii_lowercase();
        if self.schema_version != 1 {
            return Err(KeepframeError::Message(
                "This export-settings version is not supported.".into(),
            ));
        }
        if !matches!(self.format.as_str(), "jpeg" | "png" | "tiff") {
            return Err(KeepframeError::Message("Choose JPEG, PNG, or TIFF.".into()));
        }
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(KeepframeError::Message(
                "JPEG quality must be between 1 and 100.".into(),
            ));
        }
        if !matches!(self.png_compression.as_str(), "fast" | "balanced" | "best") {
            return Err(KeepframeError::Message(
                "PNG compression must be Fast, Balanced, or Best.".into(),
            ));
        }
        if !matches!(
            self.resize_mode.as_str(),
            "original" | "bounds" | "longedge" | "shortedge" | "percentage"
        ) {
            return Err(KeepframeError::Message(
                "Choose a supported resize mode.".into(),
            ));
        }
        if self.width == 0 || self.height == 0 || !(1..=800).contains(&self.percentage) {
            return Err(KeepframeError::Message(
                "Resize dimensions and percentage must be greater than zero.".into(),
            ));
        }
        if !(1..=2400).contains(&self.ppi) {
            return Err(KeepframeError::Message(
                "Resolution metadata must be between 1 and 2400 PPI.".into(),
            ));
        }
        if !matches!(
            self.sharpening.as_str(),
            "none" | "low" | "standard" | "high"
        ) {
            return Err(KeepframeError::Message(
                "Choose a supported output-sharpening amount.".into(),
            ));
        }
        if !matches!(
            self.metadata.as_str(),
            "all" | "copyrightcontact" | "copyright" | "none"
        ) {
            return Err(KeepframeError::Message(
                "Choose a supported metadata policy.".into(),
            ));
        }
        if !matches!(
            self.collision.as_str(),
            "ask" | "skip" | "replace" | "unique"
        ) {
            return Err(KeepframeError::Message(
                "Choose Ask, Skip, Replace, or Unique for filename conflicts.".into(),
            ));
        }
        if self.colour_space != "srgb" {
            return Err(KeepframeError::Message(
                "This build exports only genuine sRGB output.".into(),
            ));
        }
        if self.filename_template.trim().is_empty()
            || self.filename_template.chars().count() > 180
            || self.filename_template.contains(['/', '\\'])
        {
            return Err(KeepframeError::Message(
                "The filename template must be 1 to 180 characters and cannot contain a path."
                    .into(),
            ));
        }
        for token in template_tokens(&self.filename_template) {
            if !matches!(
                token.as_str(),
                "filename"
                    | "stem"
                    | "sequence"
                    | "capturedate"
                    | "exportdate"
                    | "rating"
                    | "custom"
            ) {
                return Err(KeepframeError::Message(format!(
                    "Unknown filename token {{{token}}}."
                )));
            }
        }
        self.custom_text = sanitise_component(&self.custom_text);
        self.sequence_padding = self.sequence_padding.clamp(1, 8);
        Ok(self)
    }
    fn extension(&self) -> &'static str {
        match self.format.as_str() {
            "jpeg" => "jpg",
            "png" => "png",
            _ => "tif",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportPreset {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) built_in: bool,
    pub(crate) config: ExportConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportRequest {
    pub(crate) asset_ids: Vec<String>,
    pub(crate) destination: String,
    pub(crate) config: ExportConfig,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportItemResult {
    pub(crate) asset_id: String,
    pub(crate) filename: String,
    pub(crate) state: String,
    pub(crate) path: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) bytes: Option<u64>,
    pub(crate) elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportBatchReport {
    pub(crate) batch_id: String,
    pub(crate) destination: String,
    pub(crate) requested: usize,
    pub(crate) complete: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    pub(crate) skipped: usize,
    pub(crate) elapsed_ms: u128,
    pub(crate) peak_working_bytes: u64,
    pub(crate) concurrency: usize,
    pub(crate) items: Vec<ExportItemResult>,
}

struct SourceDetails {
    filename: String,
    captured_at: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    rating: Option<u8>,
    tags: Vec<String>,
    input: AdjustmentInput,
    recipe: DevelopRecipe,
}

fn template_tokens(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            result.push(after[..end].to_ascii_lowercase());
            rest = &after[end + 1..];
        } else {
            result.push("".into());
            break;
        }
    }
    result
}

pub(crate) fn sanitise_component(value: &str) -> String {
    let mut clean = value
        .chars()
        .map(|value| {
            if value.is_control() || "<>:\"/\\|?*".contains(value) {
                '_'
            } else {
                value
            }
        })
        .collect::<String>();
    clean = clean.trim().trim_end_matches([' ', '.']).to_string();
    while clean.contains("__") {
        clean = clean.replace("__", "_");
    }
    let device = clean
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        device.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        clean.insert(0, '_');
    }
    if clean.is_empty() {
        "photograph".into()
    } else {
        clean.chars().take(180).collect()
    }
}

fn expanded_filename(details: &SourceDetails, config: &ExportConfig, index: usize) -> String {
    let path = Path::new(&details.filename);
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("photograph");
    let original = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("photograph");
    let capture = details.captured_at.get(0..10).unwrap_or("unknown-date");
    let sequence = config.sequence_start.saturating_add(index as u32);
    let rating = details.rating.unwrap_or(0).to_string();
    let values = [
        ("{filename}", original),
        ("{stem}", stem),
        ("{capturedate}", capture),
        ("{rating}", rating.as_str()),
        ("{custom}", config.custom_text.as_str()),
    ];
    let mut output = config.filename_template.clone();
    for (token, value) in values {
        output = output.replace(token, value);
    }
    output = output
        .replace("{exportdate}", &Local::now().format("%Y-%m-%d").to_string())
        .replace(
            "{sequence}",
            &format!(
                "{:0width$}",
                sequence,
                width = config.sequence_padding as usize
            ),
        );
    sanitise_component(&output)
}

fn final_dimensions(width: u32, height: u32, config: &ExportConfig) -> (u32, u32) {
    let scale = match config.resize_mode.as_str() {
        "bounds" => (config.width as f64 / width as f64).min(config.height as f64 / height as f64),
        "longedge" => config.width as f64 / width.max(height) as f64,
        "shortedge" => config.width as f64 / width.min(height) as f64,
        "percentage" => config.percentage as f64 / 100.0,
        _ => 1.0,
    };
    let scale = if config.no_enlarge {
        scale.min(1.0)
    } else {
        scale
    }
    .max(1.0 / width.max(height) as f64);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn srgb_profile() -> Result<Vec<u8>> {
    let system = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let candidates = [
        system.join(r"System32\spool\drivers\color\sRGB Color Space Profile.icm"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/color/sRGB.icc"),
    ];
    for path in candidates {
        if let Ok(bytes) = fs::read(&path) {
            if bytes.len() >= 128
                && bytes.get(36..40) == Some(b"acsp")
                && bytes.get(16..20) == Some(b"RGB ")
            {
                return Ok(bytes);
            }
        }
    }
    Err(KeepframeError::Message(
        "A valid system sRGB ICC profile is required for colour-managed export.".into(),
    ))
}

fn resize_and_sharpen(mut image: RgbImage, config: &ExportConfig) -> RgbImage {
    let (width, height) = final_dimensions(image.width(), image.height(), config);
    if image.dimensions() != (width, height) {
        image = image::imageops::resize(&image, width, height, ResizeFilter::Lanczos3);
    }
    match config.sharpening.as_str() {
        "low" => image::imageops::unsharpen(&image, 0.7, 1),
        "standard" => image::imageops::unsharpen(&image, 1.0, 1),
        "high" => image::imageops::unsharpen(&image, 1.4, 1),
        _ => image,
    }
}

fn encode(image: &RgbImage, config: &ExportConfig, icc: Vec<u8>, path: &Path) -> Result<()> {
    let mut file = fs::File::create(path)?;
    match config.format.as_str() {
        "jpeg" => {
            let mut encoder = JpegEncoder::new_with_quality(&mut file, config.jpeg_quality);
            encoder.set_pixel_density(PixelDensity::dpi(config.ppi));
            encoder
                .set_icc_profile(icc)
                .map_err(|error| KeepframeError::Message(error.to_string()))?;
            encoder.encode(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgb8,
            )?;
        }
        "png" => {
            let compression = match config.png_compression.as_str() {
                "fast" => CompressionType::Fast,
                "best" => CompressionType::Best,
                _ => CompressionType::Default,
            };
            let mut encoder =
                PngEncoder::new_with_quality(&mut file, compression, FilterType::Adaptive);
            encoder
                .set_icc_profile(icc)
                .map_err(|error| KeepframeError::Message(error.to_string()))?;
            encoder.write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgb8,
            )?;
        }
        _ => {
            let mut encoder = TiffEncoder::new(std::io::BufWriter::new(&mut file));
            encoder
                .set_icc_profile(icc)
                .map_err(|error| KeepframeError::Message(error.to_string()))?;
            encoder.write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgb8,
            )?;
        }
    }
    file.sync_all()?;
    Ok(())
}

fn apply_metadata(output: &Path, source: &SourceDetails, config: &ExportConfig) -> Result<()> {
    if config.metadata == "none" && config.ppi == 72 {
        return Ok(());
    }
    let mut command = hidden_command(exiftool_path());
    command.arg("-overwrite_original");
    match config.metadata.as_str() {
        "all" => {
            command
                .args(["-TagsFromFile"])
                .arg(&source.input.path)
                .args(["-all:all", "--ICC_Profile:all"]);
        }
        "copyrightcontact" => {
            command
                .args(["-TagsFromFile"])
                .arg(&source.input.path)
                .args([
                    "-Copyright",
                    "-Artist",
                    "-Creator",
                    "-CreatorContactInfo",
                    "-Credit",
                ]);
        }
        "copyright" => {
            command
                .args(["-TagsFromFile"])
                .arg(&source.input.path)
                .arg("-Copyright");
        }
        _ => {}
    }
    command.args([
        "-Orientation#=1",
        &format!("-XResolution={}", config.ppi),
        &format!("-YResolution={}", config.ppi),
        "-ResolutionUnit=inches",
        "-ColorSpace=sRGB",
    ]);
    if config.metadata == "all" {
        let captured = DateTime::parse_from_rfc3339(&source.captured_at)
            .map(|value| value.format("%Y:%m:%d %H:%M:%S").to_string())
            .unwrap_or_else(|_| source.captured_at.clone());
        command.arg(format!("-DateTimeOriginal={captured}"));
        if config.include_keywords {
            for tag in &source.tags {
                command.arg(format!("-Keywords+={tag}"));
                command.arg(format!("-Subject+={tag}"));
            }
        } else {
            command.args(["-Keywords=", "-Subject="]);
        }
        if config.include_rating {
            if let Some(rating) = source.rating {
                command.arg(format!("-Rating={rating}"));
            }
        } else {
            command.arg("-Rating=");
        }
        if config.include_location {
            if let (Some(latitude), Some(longitude)) = (source.latitude, source.longitude) {
                command
                    .arg(format!("-GPSLatitude={latitude}"))
                    .arg(format!("-GPSLongitude={longitude}"));
            } else {
                command.args(["-GPS:all="]);
            }
        } else {
            command.args(["-GPS:all="]);
        }
    } else {
        command.args(["-GPS:all=", "-Keywords=", "-Subject=", "-Rating="]);
    }
    let result = command.arg(output).output()?;
    if !result.status.success() {
        return Err(KeepframeError::Message(format!(
            "Metadata writing failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

fn validate_output(
    path: &Path,
    expected: (u32, u32),
    format: &str,
    _expected_icc: &[u8],
) -> Result<()> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let guessed = reader.format();
    let mut decoder = reader.into_decoder()?;
    if decoder.dimensions() != expected {
        return Err(KeepframeError::Message(
            "The encoded export dimensions did not match the requested dimensions.".into(),
        ));
    }
    let matches_format = matches!(
        (format, guessed),
        ("jpeg", Some(ImageFormat::Jpeg))
            | ("png", Some(ImageFormat::Png))
            | ("tiff", Some(ImageFormat::Tiff))
    );
    if !matches_format {
        return Err(KeepframeError::Message(
            "The encoded export format did not match its extension.".into(),
        ));
    }
    let profile = decoder
        .icc_profile()?
        .or_else(|| {
            if format == "tiff" {
                hidden_command(exiftool_path())
                    .args(["-b", "-ICC_Profile"])
                    .arg(path)
                    .output()
                    .ok()
                    .filter(|value| value.status.success() && !value.stdout.is_empty())
                    .map(|value| value.stdout)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            KeepframeError::Message(format!(
                "The encoded {format} export is missing its sRGB ICC profile."
            ))
        })?;
    if profile.len() < 128
        || profile.get(36..40) != Some(b"acsp")
        || profile.get(16..20) != Some(b"RGB ")
    {
        return Err(KeepframeError::Message(
            "The encoded export contains an invalid RGB ICC profile.".into(),
        ));
    }
    Ok(())
}

fn load_source(root: &Path, asset_id: &str) -> Result<SourceDetails> {
    let connection = open_db(root)?;
    let (filename, captured_at, latitude, longitude): (String, String, Option<f64>, Option<f64>) = connection.query_row("SELECT filename,captured_at,latitude,longitude FROM assets WHERE id=?1 AND trashed_at IS NULL", [asset_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    let tags = {
        let mut statement=connection.prepare("SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=?1 ORDER BY t.name COLLATE NOCASE")?;
        let values = statement
            .query_map([asset_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        values
    };
    let input = original_adjustment_input(&connection, asset_id)?;
    let rating = hidden_command(exiftool_path())
        .args(["-s3", "-n", "-Rating"])
        .arg(&input.path)
        .output()
        .ok()
        .filter(|value| value.status.success())
        .and_then(|value| {
            String::from_utf8_lossy(&value.stdout)
                .trim()
                .parse::<u8>()
                .ok()
        })
        .filter(|value| *value <= 5);
    Ok(SourceDetails {
        filename,
        captured_at,
        latitude,
        longitude,
        rating,
        tags,
        input,
        recipe: develop_recipe_in(&connection, asset_id)?,
    })
}

fn render_full(
    root: &Path,
    source: &SourceDetails,
    caches: &Mutex<RendererCaches>,
    generation: (&AtomicU64, u64),
) -> Result<RgbImage> {
    let (expected_width, expected_height) = source.input.expected_dimensions.unwrap_or((0, 0));
    let source_key = format!(
        "full:{}:{}",
        source.input.source_hash,
        decoded_source_key(&source.input.path, expected_width, expected_height)?
    );
    let cached = caches
        .lock()
        .expect("renderer cache lock")
        .decoded
        .get(&source_key);
    let image = if let Some(image) = cached {
        image
    } else {
        let image = Arc::new(prepare_full_resolution_image(
            &source.input.path,
            source.input.expected_dimensions,
            &root.join(".keepframe/staging/working"),
        )?);
        let bytes = image.as_bytes().len();
        caches.lock().expect("renderer cache lock").decoded.insert(
            source_key.clone(),
            Arc::clone(&image),
            bytes,
        );
        image
    };
    render_develop_recipe_cached(
        &image,
        &source_key,
        &source.recipe,
        caches,
        Some(generation),
    )
}

fn collision_path(
    destination: &Path,
    filename: &str,
    extension: &str,
    policy: &str,
) -> Result<Option<PathBuf>> {
    // Keep the full path below the traditional Windows limit used by many
    // external metadata and image-codec tools, reserving room for a unique suffix.
    let directory_chars = destination.to_string_lossy().chars().count();
    let filename_budget = 240usize
        .saturating_sub(directory_chars)
        .saturating_sub(extension.chars().count())
        .saturating_sub(8);
    if filename_budget < 8 {
        return Err(KeepframeError::Message(
            "The export destination path is too long for safe filenames.".into(),
        ));
    }
    let filename: String = filename.chars().take(filename_budget).collect();
    let first = destination.join(format!("{filename}.{extension}"));
    if !first.exists() || policy == "replace" {
        return Ok(Some(first));
    }
    if policy == "skip" || policy == "ask" {
        return Ok(None);
    }
    for number in 2..10_000 {
        let candidate = destination.join(format!("{filename}-{number}.{extension}"));
        if !candidate.exists() {
            return Ok(Some(candidate));
        }
    }
    Err(KeepframeError::Message(
        "Could not find a safe unique export filename.".into(),
    ))
}

pub(crate) fn run_batch(
    root: &Path,
    request: ExportRequest,
    caches: &Mutex<RendererCaches>,
    generation: &AtomicU64,
    requested_generation: u64,
    app: Option<&AppHandle>,
) -> Result<ExportBatchReport> {
    let started = Instant::now();
    let config = request.config.validate()?;
    let destination = PathBuf::from(&request.destination);
    if !destination.is_dir() {
        return Err(KeepframeError::Message(
            "Choose an existing export folder.".into(),
        ));
    }
    if request.asset_ids.is_empty() {
        return Err(KeepframeError::Message(
            "Add at least one photograph to the export queue.".into(),
        ));
    }
    if request.asset_ids.len() > 10_000 {
        return Err(KeepframeError::Message(
            "One export queue can contain at most 10,000 photographs.".into(),
        ));
    }
    let canonical_destination = destination.canonicalize()?;
    let icc = srgb_profile()?;
    let mut items = Vec::new();
    let batch_id = Uuid::new_v4().to_string();
    let mut peak = 0u64;
    let connection = open_db(root)?;
    let required_bytes = request
        .asset_ids
        .iter()
        .try_fold(0u64, |total, id| -> Result<u64> {
            let dimensions: Option<(Option<u32>, Option<u32>)> = connection
                .query_row(
                    "SELECT width,height FROM assets WHERE id=?1 AND trashed_at IS NULL",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let estimated = if let Some((width, height)) =
                dimensions.and_then(|(width, height)| width.zip(height))
            {
                let (width, height) = final_dimensions(width, height, &config);
                u64::from(width)
                    .saturating_mul(u64::from(height))
                    .saturating_mul(4)
                    .saturating_add(65_536)
            } else {
                // Unknown dimensions are deliberately expensive so a missing
                // catalogue probe cannot turn into a dangerously small estimate.
                512 * 1024 * 1024
            };
            total
                .checked_add(estimated)
                .ok_or_else(|| KeepframeError::Message("Export size estimation overflowed.".into()))
        })?;
    safety::ensure_capacity(
        required_bytes,
        fs2::available_space(&canonical_destination)?,
    )?;
    for (index, asset_id) in request.asset_ids.iter().enumerate() {
        if generation.load(Ordering::Acquire) != requested_generation {
            for remaining in &request.asset_ids[index..] {
                items.push(ExportItemResult {
                    asset_id: remaining.clone(),
                    filename: String::new(),
                    state: "cancelled".into(),
                    path: None,
                    error: None,
                    width: None,
                    height: None,
                    bytes: None,
                    elapsed_ms: 0,
                });
            }
            break;
        }
        let item_start = Instant::now();
        let source = match load_source(root, asset_id) {
            Ok(value) => value,
            Err(error) => {
                items.push(ExportItemResult {
                    asset_id: asset_id.clone(),
                    filename: String::new(),
                    state: "failed".into(),
                    path: None,
                    error: Some(error.to_string()),
                    width: None,
                    height: None,
                    bytes: None,
                    elapsed_ms: item_start.elapsed().as_millis(),
                });
                continue;
            }
        };
        let name = expanded_filename(&source, &config, index);
        let _=app.map(|app|app.emit("export-progress",json!({"batchId":batch_id,"current":index+1,"completed":items.iter().filter(|item|item.state=="complete").count(),"total":request.asset_ids.len(),"phase":"Rendering","file":source.filename})));
        let result = (|| -> Result<Option<(PathBuf, u32, u32, u64)>> {
            let Some(output) = collision_path(
                &canonical_destination,
                &name,
                config.extension(),
                &config.collision,
            )?
            else {
                return Ok(None);
            };
            if output
                .parent()
                .is_none_or(|parent| parent != canonical_destination)
            {
                return Err(KeepframeError::Message(
                    "The filename would escape the selected export folder.".into(),
                ));
            }
            let mut image = render_full(root, &source, caches, (generation, requested_generation))?;
            peak = peak.max(image.as_raw().len() as u64);
            let _=app.map(|app|app.emit("export-progress",json!({"batchId":batch_id,"current":index+1,"completed":items.iter().filter(|item|item.state=="complete").count(),"total":request.asset_ids.len(),"phase":"Resizing and sharpening","file":source.filename})));
            image = resize_and_sharpen(image, &config);
            peak = peak.max(image.as_raw().len() as u64);
            if generation.load(Ordering::Acquire) != requested_generation {
                return Err(KeepframeError::Message(
                    "Export cancelled at a safe boundary.".into(),
                ));
            }
            let temporary =
                canonical_destination.join(format!(".keepframe-export-{}.tmp", Uuid::new_v4()));
            let work = (|| -> Result<(u32, u32, u64)> {
                let _=app.map(|app|app.emit("export-progress",json!({"batchId":batch_id,"current":index+1,"completed":items.iter().filter(|item|item.state=="complete").count(),"total":request.asset_ids.len(),"phase":"Encoding","file":source.filename})));
                encode(&image, &config, icc.clone(), &temporary)?;
                apply_metadata(&temporary, &source, &config)?;
                validate_output(&temporary, image.dimensions(), &config.format, &icc)?;
                let bytes = fs::metadata(&temporary)?.len();
                let _=app.map(|app|app.emit("export-progress",json!({"batchId":batch_id,"current":index+1,"completed":items.iter().filter(|item|item.state=="complete").count(),"total":request.asset_ids.len(),"phase":"Writing","file":source.filename})));
                fs::rename(&temporary, &output)?;
                Ok((image.width(), image.height(), bytes))
            })();
            if work.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            let (width, height, bytes) = work?;
            Ok(Some((output, width, height, bytes)))
        })();
        match result {
            Ok(Some((path, width, height, bytes))) => items.push(ExportItemResult {
                asset_id: asset_id.clone(),
                filename: source.filename,
                state: "complete".into(),
                path: Some(path.to_string_lossy().into()),
                error: None,
                width: Some(width),
                height: Some(height),
                bytes: Some(bytes),
                elapsed_ms: item_start.elapsed().as_millis(),
            }),
            Ok(None) => {
                items.push(ExportItemResult {
                    asset_id: asset_id.clone(),
                    filename: source.filename,
                    state: "skipped".into(),
                    path: None,
                    error: Some(if config.collision == "ask" {
                        "A file already exists; confirm Replace, Skip, or Unique and run again."
                            .into()
                    } else {
                        "A file already exists.".into()
                    }),
                    width: None,
                    height: None,
                    bytes: None,
                    elapsed_ms: item_start.elapsed().as_millis(),
                });
                if config.collision == "ask" {
                    for remaining in &request.asset_ids[index + 1..] {
                        items.push(ExportItemResult {
                            asset_id: remaining.clone(),
                            filename: String::new(),
                            state: "cancelled".into(),
                            path: None,
                            error: Some("The batch stopped for collision resolution.".into()),
                            width: None,
                            height: None,
                            bytes: None,
                            elapsed_ms: 0,
                        });
                    }
                    break;
                }
            }
            Err(error) => {
                let cancelled = error.to_string().contains("cancelled")
                    || error.to_string().contains("Superseded Develop preview");
                items.push(ExportItemResult {
                    asset_id: asset_id.clone(),
                    filename: source.filename,
                    state: if cancelled {
                        "cancelled".into()
                    } else {
                        "failed".into()
                    },
                    path: None,
                    error: Some(if cancelled {
                        "Export cancelled at a safe boundary.".into()
                    } else {
                        error.to_string()
                    }),
                    width: None,
                    height: None,
                    bytes: None,
                    elapsed_ms: item_start.elapsed().as_millis(),
                })
            }
        }
    }
    let complete = items.iter().filter(|item| item.state == "complete").count();
    let failed = items.iter().filter(|item| item.state == "failed").count();
    let cancelled = items
        .iter()
        .filter(|item| item.state == "cancelled")
        .count();
    let skipped = items.iter().filter(|item| item.state == "skipped").count();
    let _=app.map(|app|app.emit("export-progress",json!({"batchId":batch_id,"current":items.len(),"completed":complete,"total":request.asset_ids.len(),"phase":"Complete","failures":failed,"cancelled":cancelled,"skipped":skipped})));
    Ok(ExportBatchReport {
        batch_id,
        destination: canonical_destination.to_string_lossy().into(),
        requested: request.asset_ids.len(),
        complete,
        failed,
        cancelled,
        skipped,
        elapsed_ms: started.elapsed().as_millis(),
        peak_working_bytes: peak,
        concurrency: 1,
        items,
    })
}

fn built_ins() -> Vec<ExportPreset> {
    vec![
        ExportPreset {
            id: "web-jpeg".into(),
            name: "Web JPEG".into(),
            built_in: true,
            config: ExportConfig {
                resize_mode: "longedge".into(),
                width: 2400,
                ppi: 96,
                sharpening: "standard".into(),
                ..ExportConfig::default()
            },
        },
        ExportPreset {
            id: "full-jpeg".into(),
            name: "Full-size JPEG".into(),
            built_in: true,
            config: ExportConfig::default(),
        },
        ExportPreset {
            id: "archive-tiff".into(),
            name: "Archive TIFF".into(),
            built_in: true,
            config: ExportConfig {
                format: "tiff".into(),
                sharpening: "none".into(),
                metadata: "all".into(),
                ..ExportConfig::default()
            },
        },
    ]
}

pub(crate) fn list_presets(root: &Path) -> Result<Vec<ExportPreset>> {
    let connection = open_db(root)?;
    let mut presets = built_ins();
    let mut statement = connection
        .prepare("SELECT preset_json FROM export_presets ORDER BY name COLLATE NOCASE")?;
    for row in statement.query_map([], |row| row.get::<_, String>(0))? {
        let mut preset: ExportPreset = serde_json::from_str(&row?)?;
        preset.config = preset.config.validate()?;
        presets.push(preset);
    }
    Ok(presets)
}
pub(crate) fn save_preset(root: &Path, mut preset: ExportPreset) -> Result<ExportPreset> {
    preset.name = preset.name.trim().to_string();
    if preset.built_in
        || preset.id.trim().is_empty()
        || preset.name.is_empty()
        || preset.name.chars().count() > 80
    {
        return Err(KeepframeError::Message(
            "User export presets need an id and a 1 to 80 character name.".into(),
        ));
    }
    preset.config = preset.config.validate()?;
    let connection = open_db(root)?;
    let now = Utc::now().to_rfc3339();
    connection.execute("INSERT INTO export_presets(id,name,schema_version,preset_json,created_at,updated_at)VALUES(?1,?2,1,?3,?4,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,preset_json=excluded.preset_json,updated_at=excluded.updated_at",params![preset.id,preset.name,serde_json::to_string(&preset)?,now])?;
    Ok(preset)
}
pub(crate) fn delete_preset(root: &Path, id: &str) -> Result<()> {
    if built_ins().iter().any(|preset| preset.id == id) {
        return Err(KeepframeError::Message(
            "Built-in export presets cannot be deleted.".into(),
        ));
    }
    open_db(root)?.execute("DELETE FROM export_presets WHERE id=?1", [id])?;
    Ok(())
}
pub(crate) fn export_preset(root: &Path, id: &str, path: &Path) -> Result<()> {
    if !path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("keepframe-export-preset"))
    {
        return Err(KeepframeError::Message(
            "Export preset files must use .keepframe-export-preset.".into(),
        ));
    }
    let preset = list_presets(root)?
        .into_iter()
        .find(|preset| preset.id == id)
        .ok_or_else(|| KeepframeError::Message("The export preset no longer exists.".into()))?;
    let portable = json!({"schemaVersion":1,"name":preset.name,"config":preset.config});
    fs::write(path, serde_json::to_vec_pretty(&portable)?)?;
    Ok(())
}
pub(crate) fn import_preset(root: &Path, path: &Path) -> Result<ExportPreset> {
    if fs::metadata(path)?.len() > EXPORT_PRESET_FILE_MAX_BYTES {
        return Err(KeepframeError::Message(
            "Export preset is larger than the 128 KiB safety limit.".into(),
        ));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Portable {
        schema_version: i64,
        name: String,
        config: ExportConfig,
    }
    let file: Portable = serde_json::from_slice(&fs::read(path)?)?;
    if file.schema_version != 1 {
        return Err(KeepframeError::Message(
            "This export preset version is not supported.".into(),
        ));
    }
    let existing = list_presets(root)?;
    let mut name = file.name;
    let base = name.clone();
    let mut suffix = 2;
    while existing
        .iter()
        .any(|preset| preset.name.eq_ignore_ascii_case(&name))
    {
        name = format!("{base} {suffix}");
        suffix += 1;
    }
    save_preset(
        root,
        ExportPreset {
            id: Uuid::new_v4().to_string(),
            name,
            built_in: false,
            config: file.config,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_library() -> (PathBuf, PathBuf, String) {
        let root =
            std::env::temp_dir().join(format!("keepframe-professional-export-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/2026/09/12/Fête portrait.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        image::DynamicImage::ImageRgb8(RgbImage::from_fn(96, 64, |x, y| {
            image::Rgb([
                ((x * 2) % 255) as u8,
                ((y * 3) % 255) as u8,
                ((x + y) % 255) as u8,
            ])
        }))
        .save(&original)
        .unwrap();
        let hash = hash_file(&original).unwrap();
        let connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,captured_at,width,height,latitude,longitude,location_source,thumbnail_path,created_at)VALUES('asset','Fête portrait.png','2026-09-12T12:30:00Z',96,64,56.1,-3.1,'manual',?1,'2026-09-12T12:30:00Z')",[original.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','asset',?1,?2,'png','Fête portrait',?3,0)",params![original.to_string_lossy(),hash,fs::metadata(&original).unwrap().len()]).unwrap();
        connection.execute_batch("INSERT INTO tags(id,name)VALUES('tag','family');INSERT INTO asset_tags(asset_id,tag_id)VALUES('asset','tag');").unwrap();
        let recipe = DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments {
                exposure: 0.2,
                crop_width: 0.75,
                crop_height: 0.75,
                rotate_quadrants: 1,
                ..BasicAdjustments::neutral()
            },
            masks: vec![DevelopMask {
                id: "mask".into(),
                name: "Local".into(),
                enabled: true,
                inverted: false,
                opacity: 0.8,
                feather: 0.4,
                geometry: MaskGeometry::Linear {
                    start: MaskPoint { x: 0.0, y: 0.5 },
                    end: MaskPoint { x: 1.0, y: 0.5 },
                },
                adjustments: LocalAdjustments {
                    exposure: 0.3,
                    ..LocalAdjustments::default()
                },
            }],
        };
        connection.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at)VALUES('asset',2,?1,'2026-09-12T12:30:00Z')",[serde_json::to_string(&recipe).unwrap()]).unwrap();
        drop(connection);
        (root, original, hash)
    }
    fn request(destination: &Path, config: ExportConfig, asset_ids: Vec<&str>) -> ExportRequest {
        ExportRequest {
            asset_ids: asset_ids.into_iter().map(str::to_string).collect(),
            destination: destination.to_string_lossy().into(),
            config,
        }
    }
    #[test]
    fn defaults_are_safe_and_srgb() {
        let value = ExportConfig::default().validate().unwrap();
        assert_eq!(value.format, "jpeg");
        assert!(value.no_enlarge);
        assert!(!value.include_location);
        assert_eq!(value.colour_space, "srgb");
    }
    #[test]
    fn unsupported_format_colour_and_bad_ranges_fail_closed() {
        for value in [
            ExportConfig {
                format: "webp".into(),
                ..Default::default()
            },
            ExportConfig {
                colour_space: "display-p3".into(),
                ..Default::default()
            },
            ExportConfig {
                jpeg_quality: 0,
                ..Default::default()
            },
        ] {
            assert!(value.validate().is_err());
        }
    }
    #[test]
    fn resize_modes_preserve_aspect_and_no_enlarge() {
        let mut value = ExportConfig {
            resize_mode: "bounds".into(),
            width: 1000,
            height: 1000,
            ..Default::default()
        };
        assert_eq!(final_dimensions(4000, 2000, &value), (1000, 500));
        value.resize_mode = "longedge".into();
        value.width = 8000;
        assert_eq!(final_dimensions(4000, 2000, &value), (4000, 2000));
        value.no_enlarge = false;
        assert_eq!(final_dimensions(4000, 2000, &value), (8000, 4000));
        value.resize_mode = "shortedge".into();
        value.width = 1000;
        assert_eq!(final_dimensions(4000, 2000, &value), (2000, 1000));
        value.resize_mode = "percentage".into();
        value.percentage = 25;
        assert_eq!(final_dimensions(4000, 2000, &value), (1000, 500));
    }
    #[test]
    fn windows_names_and_unicode_are_sanitised_without_paths() {
        assert_eq!(sanitise_component("CON"), "_CON");
        assert_eq!(sanitise_component("  Fête:*?.  "), "Fête_");
        let bad = ExportConfig {
            filename_template: "../{stem}".into(),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
    #[test]
    fn templates_reject_unknown_or_unclosed_tokens() {
        assert!(ExportConfig {
            filename_template: "{mystery}".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ExportConfig {
            filename_template: "{stem".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }
    #[test]
    fn collision_policies_are_deterministic() {
        let dir =
            std::env::temp_dir().join(format!("keepframe-export-collision-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("photo.jpg"), b"old").unwrap();
        assert!(collision_path(&dir, "photo", "jpg", "skip")
            .unwrap()
            .is_none());
        assert_eq!(
            collision_path(&dir, "photo", "jpg", "replace")
                .unwrap()
                .unwrap(),
            dir.join("photo.jpg")
        );
        assert_eq!(
            collision_path(&dir, "photo", "jpg", "unique")
                .unwrap()
                .unwrap(),
            dir.join("photo-2.jpg")
        );
        let bounded = collision_path(&dir, &"x".repeat(400), "jpg", "unique")
            .unwrap()
            .unwrap();
        assert!(bounded.to_string_lossy().chars().count() <= 240);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ask_collision_stops_the_batch_without_touching_existing_output() {
        let (root, _, _) = fixture_library();
        let destination = root.join("Exports");
        let existing = destination.join("same.png");
        fs::write(&existing, b"existing").unwrap();
        let generation = AtomicU64::new(1);
        let report = run_batch(
            &root,
            request(
                &destination,
                ExportConfig {
                    format: "png".into(),
                    metadata: "none".into(),
                    filename_template: "same".into(),
                    collision: "ask".into(),
                    ..Default::default()
                },
                vec!["asset", "asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        assert_eq!(
            report
                .items
                .iter()
                .map(|item| item.state.as_str())
                .collect::<Vec<_>>(),
            vec!["skipped", "cancelled"]
        );
        assert_eq!(fs::read(existing).unwrap(), b"existing");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn jpeg_png_and_tiff_are_decodable_dimensionally_correct_and_icc_tagged() {
        let Some(_) = std::env::var_os("WINDIR") else {
            return;
        };
        let image = RgbImage::from_fn(96, 64, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 3) as u8, 90])
        });
        let dir = std::env::temp_dir().join(format!("keepframe-export-codecs-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let icc = srgb_profile().unwrap();
        for format in ["jpeg", "png", "tiff"] {
            let config = ExportConfig {
                format: format.into(),
                png_compression: "best".into(),
                ..Default::default()
            }
            .validate()
            .unwrap();
            let path = dir.join(format!("test.{}", config.extension()));
            encode(&image, &config, icc.clone(), &path).unwrap();
            validate_output(&path, (96, 64), format, &icc).unwrap();
            let decoded = image::open(&path).unwrap().to_rgb8();
            if format == "jpeg" {
                let error = image
                    .as_raw()
                    .iter()
                    .zip(decoded.as_raw())
                    .map(|(left, right)| {
                        (i16::from(*left) - i16::from(*right)).unsigned_abs() as u64
                    })
                    .sum::<u64>() as f64
                    / image.as_raw().len() as f64;
                assert!(error < 8.0, "JPEG mean absolute error was {error}");
            } else {
                assert_eq!(decoded, image, "{format} must preserve RGB pixels exactly");
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn jpeg_quality_is_real_and_changes_the_encoded_payload() {
        let image = RgbImage::from_fn(320, 240, |x, y| {
            image::Rgb([
                ((x * 13 + y * 7) % 256) as u8,
                ((x * 5 + y * 11) % 256) as u8,
                ((x + y * 3) % 256) as u8,
            ])
        });
        let dir = std::env::temp_dir().join(format!("keepframe-jpeg-quality-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let icc = srgb_profile().unwrap();
        let low = dir.join("low.jpg");
        let high = dir.join("high.jpg");
        encode(
            &image,
            &ExportConfig {
                jpeg_quality: 20,
                ..Default::default()
            },
            icc.clone(),
            &low,
        )
        .unwrap();
        encode(
            &image,
            &ExportConfig {
                jpeg_quality: 95,
                ..Default::default()
            },
            icc,
            &high,
        )
        .unwrap();
        assert!(fs::metadata(&high).unwrap().len() > fs::metadata(&low).unwrap().len());
        assert_ne!(fs::read(low).unwrap(), fs::read(high).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sharpening_and_resize_do_not_change_requested_geometry() {
        let image = RgbImage::from_fn(100, 50, |x, y| image::Rgb([x as u8, y as u8, 0]));
        for sharpening in ["none", "low", "standard", "high"] {
            let value = ExportConfig {
                resize_mode: "longedge".into(),
                width: 40,
                sharpening: sharpening.into(),
                ..Default::default()
            };
            assert_eq!(
                resize_and_sharpen(image.clone(), &value).dimensions(),
                (40, 20)
            );
        }
    }
    #[test]
    fn batch_renders_each_accepted_recipe_from_full_resolution_without_mutating_original() {
        let (root, original, hash) = fixture_library();
        let destination = root.join("Exports");
        let generation = AtomicU64::new(1);
        let config = ExportConfig {
            format: "png".into(),
            png_compression: "fast".into(),
            sharpening: "none".into(),
            metadata: "none".into(),
            filename_template: "{stem}".into(),
            ..Default::default()
        };
        let report = run_batch(
            &root,
            request(&destination, config, vec!["asset"]),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        assert_eq!(
            (report.complete, report.failed, report.concurrency),
            (1, 0, 1)
        );
        let output = image::open(report.items[0].path.as_ref().unwrap())
            .unwrap()
            .to_rgb8();
        let source = load_source(&root, "asset").unwrap();
        let expected = render_develop_recipe(
            &prepare_full_resolution_image(
                &source.input.path,
                source.input.expected_dimensions,
                &root.join("working"),
            )
            .unwrap(),
            &source.recipe,
        );
        assert_eq!(output, expected);
        assert_eq!(hash_file(&original).unwrap(), hash);
        assert!(!destination.read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("keepframe-export")));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn batch_reports_partial_failure_and_cancelled_remaining_items() {
        let (root, _, _) = fixture_library();
        let destination = root.join("Exports");
        let generation = AtomicU64::new(1);
        let report = run_batch(
            &root,
            request(
                &destination,
                ExportConfig {
                    metadata: "none".into(),
                    ..Default::default()
                },
                vec!["missing", "asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        assert_eq!((report.complete, report.failed), (1, 1));
        let cancelled = run_batch(
            &root,
            request(
                &destination,
                ExportConfig::default(),
                vec!["asset", "asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &AtomicU64::new(2),
            1,
            None,
        )
        .unwrap();
        assert_eq!(cancelled.cancelled, 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn gps_keywords_and_internal_fields_follow_explicit_metadata_policy() {
        let (root, _, _) = fixture_library();
        let destination = root.join("Exports");
        let generation = AtomicU64::new(1);
        let with_gps = run_batch(
            &root,
            request(
                &destination,
                ExportConfig {
                    format: "jpeg".into(),
                    include_location: true,
                    filename_template: "with-gps".into(),
                    ..Default::default()
                },
                vec!["asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        let path = Path::new(with_gps.items[0].path.as_ref().unwrap());
        let output = hidden_command(exiftool_path())
            .args([
                "-json",
                "-n",
                "-GPSLatitude",
                "-GPSLongitude",
                "-Keywords",
                "-Orientation",
            ])
            .arg(path)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("56.1") && text.contains("family"));
        assert!(!text.contains("assetId") && !text.contains("recipeJson"));
        let no_gps = run_batch(
            &root,
            request(
                &destination,
                ExportConfig {
                    format: "png".into(),
                    metadata: "none".into(),
                    include_location: false,
                    filename_template: "no-gps".into(),
                    ..Default::default()
                },
                vec!["asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        let output = hidden_command(exiftool_path())
            .args(["-json", "-GPSLatitude", "-GPSLongitude", "-Keywords"])
            .arg(no_gps.items[0].path.as_ref().unwrap())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("GPSLatitude") && !text.contains("Keywords"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn user_presets_persist_round_trip_without_destination() {
        let (root, _, _) = fixture_library();
        let preset = ExportPreset {
            id: "user".into(),
            name: "Client JPEG".into(),
            built_in: false,
            config: ExportConfig {
                jpeg_quality: 82,
                ..Default::default()
            },
        };
        save_preset(&root, preset).unwrap();
        let loaded = list_presets(&root)
            .unwrap()
            .into_iter()
            .find(|value| value.id == "user")
            .unwrap();
        assert_eq!(loaded.config.jpeg_quality, 82);
        let path = root.join("client.keepframe-export-preset");
        export_preset(&root, "user", &path).unwrap();
        let json = fs::read_to_string(&path).unwrap();
        assert!(!json.to_ascii_lowercase().contains("destination"));
        let imported = import_preset(&root, &path).unwrap();
        assert_eq!(imported.name, "Client JPEG 2");
        let malformed = root.join("malformed.keepframe-export-preset");
        fs::write(&malformed, br#"{"schemaVersion":2,"name":"Future"}"#).unwrap();
        assert!(import_preset(&root, &malformed).is_err());
        delete_preset(&root, "user").unwrap();
        assert!(delete_preset(&root, "web-jpeg").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replace_promotes_a_complete_temp_output_and_unique_keeps_both() {
        let (root, _, _) = fixture_library();
        let destination = root.join("Exports");
        let generation = AtomicU64::new(1);
        let base = ExportConfig {
            format: "png".into(),
            metadata: "none".into(),
            filename_template: "same".into(),
            collision: "replace".into(),
            ..Default::default()
        };
        let first = run_batch(
            &root,
            request(&destination, base.clone(), vec!["asset"]),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        fs::write(first.items[0].path.as_ref().unwrap(), b"old").unwrap();
        let replaced = run_batch(
            &root,
            request(&destination, base, vec!["asset"]),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        assert!(image::open(replaced.items[0].path.as_ref().unwrap()).is_ok());
        let unique = run_batch(
            &root,
            request(
                &destination,
                ExportConfig {
                    format: "png".into(),
                    metadata: "none".into(),
                    filename_template: "same".into(),
                    collision: "unique".into(),
                    ..Default::default()
                },
                vec!["asset"],
            ),
            &Mutex::new(RendererCaches::default()),
            &generation,
            1,
            None,
        )
        .unwrap();
        assert!(unique.items[0]
            .path
            .as_ref()
            .unwrap()
            .ends_with("same-2.png"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "manual Milestone 11 export benchmark; run scripts/benchmark-export.ps1"]
    fn milestone_11_export_performance_checkpoint() {
        let (root, _, _) = fixture_library();
        let source_details = load_source(&root, "asset").unwrap();
        let source = image::DynamicImage::ImageRgb8(RgbImage::from_fn(3600, 2400, |x, y| {
            image::Rgb([
                ((x * 7 + y * 3) % 256) as u8,
                ((x * 2 + y * 5) % 256) as u8,
                ((x + y * 11) % 256) as u8,
            ])
        }));
        let recipe = DevelopRecipe {
            schema_version: 2,
            settings: BasicAdjustments {
                exposure: 0.15,
                contrast: 8.0,
                clarity: 5.0,
                ..BasicAdjustments::neutral()
            },
            masks: vec![DevelopMask {
                id: "linear".into(),
                name: "Linear".into(),
                enabled: true,
                inverted: false,
                opacity: 0.8,
                feather: 0.5,
                geometry: MaskGeometry::Linear {
                    start: MaskPoint { x: 0.0, y: 0.2 },
                    end: MaskPoint { x: 1.0, y: 0.8 },
                },
                adjustments: LocalAdjustments {
                    exposure: 0.2,
                    ..LocalAdjustments::default()
                },
            }],
        };
        let start = Instant::now();
        let rendered = render_develop_recipe(&source, &recipe);
        let render_ms = start.elapsed().as_millis();
        let mut formats = Vec::new();
        for (format, variant) in [
            ("jpeg", "full"),
            ("jpeg", "resized"),
            ("png", "full"),
            ("png", "resized"),
            ("tiff", "full"),
        ] {
            let config = ExportConfig {
                format: format.into(),
                metadata: "none".into(),
                filename_template: format!("{format}-{variant}"),
                ..Default::default()
            };
            let start = Instant::now();
            let resized = (variant == "resized")
                .then(|| image::imageops::resize(&rendered, 2400, 1600, ResizeFilter::Lanczos3));
            let resize_ms = start.elapsed().as_millis();
            let input = resized.as_ref().unwrap_or(&rendered);
            let start = Instant::now();
            let sharpened = image::imageops::unsharpen(input, 1.0, 1);
            let sharpen_ms = start.elapsed().as_millis();
            let staged = root
                .join("Exports")
                .join(format!(".profile-{format}-{variant}.tmp"));
            let start = Instant::now();
            encode(&sharpened, &config, srgb_profile().unwrap(), &staged).unwrap();
            let encode_stage_ms = start.elapsed().as_millis();
            let start = Instant::now();
            apply_metadata(&staged, &source_details, &config).unwrap();
            let metadata_ms = start.elapsed().as_millis();
            let start = Instant::now();
            let promoted = root
                .join("Exports")
                .join(format!("profile-{format}-{variant}.{}", config.extension()));
            fs::rename(&staged, &promoted).unwrap();
            let write_ms = start.elapsed().as_millis();
            formats.push(json!({"format":format,"variant":variant,"width":sharpened.width(),"height":sharpened.height(),"renderMs":render_ms,"resizeMs":resize_ms,"sharpenMs":sharpen_ms,"encodeAndStageWriteMs":encode_stage_ms,"metadataMs":metadata_ms,"finalPromotionMs":write_ms,"totalMs":render_ms+resize_ms+sharpen_ms+encode_stage_ms+metadata_ms+write_ms,"bytes":fs::metadata(promoted).unwrap().len()}));
        }
        let start = Instant::now();
        let mut peak = 0u64;
        for index in 0..10 {
            let mask = if index % 3 == 0 {
                recipe.masks.clone()
            } else {
                Vec::new()
            };
            let individual = DevelopRecipe {
                schema_version: 2,
                settings: BasicAdjustments {
                    exposure: (index as f32 - 5.0) / 20.0,
                    contrast: index as f32,
                    ..BasicAdjustments::neutral()
                },
                masks: mask,
            };
            let developed = render_develop_recipe(&source, &individual);
            peak = peak.max((source.as_bytes().len() + developed.as_raw().len()) as u64);
            let output = root.join("Exports").join(format!("batch-{index}.jpg"));
            encode(
                &developed,
                &ExportConfig {
                    format: "jpeg".into(),
                    metadata: "none".into(),
                    ..Default::default()
                },
                srgb_profile().unwrap(),
                &output,
            )
            .unwrap();
        }
        let batch_ms = start.elapsed().as_millis();
        println!(
            "M11_EXPORT_BENCHMARK {}",
            json!({"fixture":"3600x2400 deterministic developed RGB with accepted masks; resized variants are 2400x1600","formats":formats,"batch10":{"totalMs":batch_ms,"averageMs":batch_ms/10,"peakTrackedRgbBytes":peak,"memoryScope":"retained source plus one developed RGB item; excludes allocator, encoder and process overhead","concurrency":1,"complete":10,"mixedDevelopStates":true,"acceptedMasks":true}})
        );
        fs::remove_dir_all(root).unwrap();
    }
}

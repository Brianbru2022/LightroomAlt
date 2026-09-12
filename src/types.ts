export type ViewName = "library" | "develop" | "triage" | "map" | "workshop" | "trash" | "settings";
export type Decision = "undecided" | "keep" | "discard";
export type EditIntent =
  | "restoration"
  | "scratch_repair"
  | "denoise"
  | "sharpen"
  | "upscale"
  | "lighting_correction"
  | "object_removal"
  | "sky_replacement"
  | "colourisation"
  | "custom";
export type AiAction =
  | "improve_photo"
  | "improve_lighting"
  | "enhance_colour"
  | "restore_old_photo"
  | "remove_distraction"
  | "custom_instruction";
export type PreserveConstraint =
  | "identity_faces"
  | "composition"
  | "text"
  | "period_detail"
  | "skin_texture"
  | "grain"
  | "monochrome_tonality";
export type JobState =
  | "draft"
  | "analysing"
  | "review_required"
  | "queued"
  | "running"
  | "waiting_external"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "accepted"
  | "rejected";

export type Asset = {
  id: string;
  filename: string;
  sourcePath: string;
  previewUrl: string;
  thumbnailUrl: string;
  decision: Decision;
  capturedAt: string;
  dateFallback: boolean;
  camera?: string;
  width?: number;
  height?: number;
  latitude?: number;
  longitude?: number;
  locationSource?: "embedded" | "manual" | "none" | string;
  missingState?: "available" | "original_missing" | "derived_missing" | "modified" | string;
  tags: string[];
  representationCount: number;
  preferredVersionUrl?: string;
  hasEdits?: boolean;
};

export type MapBounds = { south: number; west: number; north: number; east: number };
export type MapAsset = {
  id: string;
  filename: string;
  latitude: number;
  longitude: number;
  capturedAt: string;
  thumbnailUrl: string;
  decision: Decision;
  locationSource: "embedded" | "manual" | "none" | string;
};

export type LibraryStatus = {
  configured: boolean;
  libraryRoot?: string;
  libraryIssue?: string;
  recoveryNotice?: string;
  counts: Record<Decision, number> & { total: number };
};

export type ServiceHealth = {
  localAiAvailable: boolean;
  serviceReachable: boolean;
  localAiBusy: boolean;
  localAiModel?: string;
  localAiDetail: string;
  localAiState: "available" | "not_configured" | "unavailable" | "service_not_running" | "model_missing" | "incompatible" | "health_check_failed" | string;
  localAiUrl: string;
  analysisModelInstalled: boolean;
  analysisAvailable: boolean;
  analysisDetail: string;
};

export type AssetVersion = {
  id: string;
  kind: "original" | "edited" | "replacement" | string;
  provider?: string;
  createdAt: string;
  state: string;
  imageUrl: string;
  prompt?: string;
  isPreferred: boolean;
  sourceHash?: string;
  outputHash?: string;
};

export type AssetFilter = {
  decision: Decision | "all";
  search: string;
  year?: number;
  tag?: string;
  dateFrom?: string;
  dateTo?: string;
  camera?: string;
  tagged?: boolean;
  located?: boolean;
  trashed?: boolean;
};

export type AssetPage = {
  items: Asset[];
  total: number;
  offset: number;
  limit: number;
  hasMore: boolean;
};

export type EditRecipe = {
  schemaVersion: 1;
  assetId: string;
  commonBrief?: string;
  action?: AiAction;
  observations: string[];
  intents: EditIntent[];
  preserve: PreserveConstraint[];
  negativeConstraints: string[];
  strength: "subtle" | "balanced" | "strong";
  output: { format: "png"; preserveDimensions: boolean; colourSpace: "sRGB" };
  analysisModel: string;
  analysisCreatedAt: string;
};

export type PromptSet = { local: string; chatgpt: string; gemini: string; negative: string };

export type BasicAdjustments = {
  exposure: number;
  lightBalance: number;
  tint: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  dynamicRange: number;
  texture: number;
  clarity: number;
  dehaze: number;
  colourBoost: number;
  saturation: number;
  curveHighlights: number;
  curveLights: number;
  curveDarks: number;
  curveShadows: number;
  cropLeft: number;
  cropTop: number;
  cropWidth: number;
  cropHeight: number;
  rotateQuadrants: number;
  straighten: number;
  horizontalFlip: boolean;
  verticalFlip: boolean;
};

export const neutralAdjustments: BasicAdjustments = {
  exposure: 0, lightBalance: 0, tint: 0, contrast: 0, highlights: 0, shadows: 0,
  whites: 0, blacks: 0, dynamicRange: 0, texture: 0, clarity: 0, dehaze: 0,
  colourBoost: 0, saturation: 0, curveHighlights: 0, curveLights: 0,
  curveDarks: 0, curveShadows: 0,
  cropLeft: 0, cropTop: 0, cropWidth: 1, cropHeight: 1,
  rotateQuadrants: 0, straighten: 0, horizontalFlip: false, verticalFlip: false,
};

export type MaskPoint = { x: number; y: number };
export type LocalAdjustments = {
  exposure: number; contrast: number; highlights: number; shadows: number; whites: number; blacks: number;
  lightBalance: number; tint: number; saturation: number; clarity: number; dehaze: number; texture: number;
};
export const neutralLocalAdjustments: LocalAdjustments = {
  exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  lightBalance: 0, tint: 0, saturation: 0, clarity: 0, dehaze: 0, texture: 0,
};
export type BrushStroke = { points: MaskPoint[]; radius: number; feather: number; flow: number; erase: boolean };
export type IntelligentMaskCategory = "subject" | "people" | "sky";
export type MaskProvenance = {
  provider: string; providerVersion: string; model: string; modelRevision: string;
  modelSha256: string; category: IntelligentMaskCategory; executionProvider: string;
};
export type MaskGeometry =
  | { kind: "linear"; start: MaskPoint; end: MaskPoint }
  | { kind: "radial"; centre: MaskPoint; radiusX: number; radiusY: number; rotation: number }
  | { kind: "brush"; strokes: BrushStroke[] }
  | { kind: "semantic"; width: number; height: number; coveragePng: string; checksum: string; provenance: MaskProvenance; refinements: BrushStroke[] };
export type DevelopMask = {
  id: string; name: string; enabled: boolean; inverted: boolean; opacity: number; feather: number;
  geometry: MaskGeometry; adjustments: LocalAdjustments;
};
export type DevelopRecipe = { schemaVersion: 2; settings: BasicAdjustments; masks: DevelopMask[] };
export type IntelligentMaskHealth = {
  available: boolean; installed: boolean; runtimeAvailable: boolean; loaded: boolean; busy: boolean;
  provider: string; providerVersion: string; model: string; modelRevision: string; licence: string;
  source: string; approximateBytes: number; storagePath: string; executionProvider: string; detail: string;
};
export type IntelligentMaskProposal = {
  requestId: number; category: IntelligentMaskCategory; confidence: number; coverageFraction: number;
  elapsedMs: number; timings: { loadMs: number; preprocessMs: number; inferenceMs: number; postprocessMs: number }; mask: DevelopMask;
};
export type PresetCategory = "whiteBalance" | "tone" | "presence" | "colour";
export type DevelopPreset = { schemaVersion: 1; id: string; name: string; categories: PresetCategory[]; settings: BasicAdjustments; builtIn: boolean };
export type ImageStatistics = { luminanceBins: number[]; redBins: number[]; greenBins: number[]; blueBins: number[]; samples: number; averageLuminance: number; p01: number; p50: number; p99: number; shadowClipFraction: number; highlightClipFraction: number; averageSaturation: number; redGreenBlue: [number, number, number]; dynamicRange: number };
export type AutoProposal = { settings: BasicAdjustments; explanation: string[]; confidence: number; statistics: ImageStatistics; recommendations: string[] };

export type JobAttempt = {
  attemptNumber: number;
  state: "running" | "succeeded" | "failed" | "cancelled";
  startedAt: string;
  finishedAt?: string;
  outputUrl?: string;
  error?: string;
};

export type BatchJob = {
  id: string;
  batchId: string;
  assetId: string;
  assetName: string;
  state: JobState;
  prompt: string;
  recipe?: EditRecipe;
  attempts: JobAttempt[];
  error?: string;
  outputUrl?: string;
};

export type ImportSummary = {
  importId: string;
  state: "running" | "cancelled" | "completed" | "needs_attention";
  discovered: number;
  imported: number;
  copied: number;
  moved: number;
  sourceRetained: number;
  duplicates: number;
  unsupported: number;
  failed: number;
};

export type ImportOptions = {
  mode: "copy" | "move";
  duplicateSourcePolicy: "retain" | "remove_after_verified_match";
};

export type TrashSummary = {
  affected: number;
  failed: number;
};

export type SidecarExportSummary = { requested: number; written: number; preservedExisting: number; failed: string[] };
export type SidecarImportResult = { tagsImported: boolean; locationImported: boolean; triageImported: boolean; conflicts: string[] };

export type ExportConfig = {
  schemaVersion: 1;
  format: "jpeg" | "png" | "tiff";
  jpegQuality: number;
  pngCompression: "fast" | "balanced" | "best";
  resizeMode: "original" | "bounds" | "longedge" | "shortedge" | "percentage";
  width: number;
  height: number;
  percentage: number;
  noEnlarge: boolean;
  ppi: number;
  sharpening: "none" | "low" | "standard" | "high";
  metadata: "all" | "copyrightcontact" | "copyright" | "none";
  includeLocation: boolean;
  includeKeywords: boolean;
  includeRating: boolean;
  filenameTemplate: string;
  customText: string;
  sequenceStart: number;
  sequencePadding: number;
  collision: "ask" | "skip" | "replace" | "unique";
  colourSpace: "srgb";
};
export type ExportPreset = { id: string; name: string; builtIn: boolean; config: ExportConfig };
export type ExportProgress = { batchId: string; current: number; completed: number; total: number; phase: "Waiting" | "Rendering" | "Resizing and sharpening" | "Encoding" | "Writing" | "Complete" | string; file?: string; failures?: number; cancelled?: number; skipped?: number };
export type ExportItemResult = { assetId: string; filename: string; state: "waiting" | "rendering" | "encoding" | "writing" | "complete" | "failed" | "cancelled" | "skipped" | string; path?: string; error?: string; width?: number; height?: number; bytes?: number; elapsedMs: number };
export type ExportBatchReport = { batchId: string; destination: string; requested: number; complete: number; failed: number; cancelled: number; skipped: number; elapsedMs: number; peakWorkingBytes: number; concurrency: number; items: ExportItemResult[] };
export type IntegrityFinding = { assetId?: string; filename?: string; kind: string; detail: string };
export type IntegrityReport = { scannedAssets: number; missingOriginals: number; missingDerivedVersions: number; modifiedOriginals: number; untrackedManagedFiles: number; sidecarConflicts: number; findings: IntegrityFinding[] };
export type RelinkCandidate = { path: string; sha256: string };
export type FolderWatchEvent = { id: number; folderPath: string; path: string; kind: "new_file" | "file_changed" | "file_removed" | string; observedAt: string };

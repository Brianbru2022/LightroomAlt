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
  rating: number;
  title?: string;
  caption?: string;
  copyright?: string;
  creator?: string;
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
  sourceId?: string;
  isPrimary?: boolean;
  versionName?: string;
  versionIndex?: number;
  sourceVersionCount?: number;
  versionGroupCollapsed?: boolean;
  stackId?: string;
  stackCount?: number;
  stackCollapsed?: boolean;
  isStackTop?: boolean;
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
  rating?: number;
  edited?: boolean;
  fileType?: string;
  sort?: "captureTime" | "importTime" | "filename" | "rating" | "editedTime";
  descending?: boolean;
  collectionId?: string;
  versionMode?: "all" | "primary" | "virtual";
  stackMode?: "all" | "stacked" | "unstacked";
};

export type CatalogueVersion={id:string;sourceId:string;name:string;isPrimary:boolean;versionIndex:number;hasEdits:boolean;rating:number;decision:Decision};
export type SmartRule={field:"rating"|"flag"|"edited"|"fileType"|"keyword"|"captureDate"|"importDate"|"camera"|"lens"|"versionStatus"|"hasMultipleVersions"|"stackStatus";operator:string;value:unknown;secondValue?:unknown};
export type CatalogueCollection={id:string;name:string;kind:"manual"|"smart";setId?:string;matchMode:"all"|"any";rules:SmartRule[];position:number;count:number};
export type CollectionSet={id:string;name:string;position:number};
export type StackSummary={id:string;name?:string;collapsed:boolean;topItemId:string;memberIds:string[]};

export type SemanticIndexStatus={available:boolean;installed:boolean;runtimeAvailable:boolean;loaded:boolean;busy:boolean;paused:boolean;provider:string;providerVersion:string;model:string;modelRevision:string;licence:string;source:string;approximateBytes:number;storagePath:string;executionProvider:string;inputResolution:number;embeddingDimensions:number;totalSources:number;indexedSources:number;queuedSources:number;failedSources:number;staleSources:number;storageBytes:number;detail:string};
export type SemanticFilter={ratingMin?:number;decision?:Decision;fileType?:string;edited?:boolean;dateFrom?:string;dateTo?:string;collectionId?:string};
export type SemanticSearchRequest={query:string;limit:number;semanticWeight?:number;filter:SemanticFilter};
export type SemanticSearchResult={assetId:string;sourceId:string;score:number;semanticScore:number;metadataScore:number;strength:string;explanation:string[];missing:boolean};
export type SemanticSearchResponse={requestId:number;results:SemanticSearchResult[];executionProvider:string;timings:Record<string,number>};
export type DiscoveryGroup={id:string;kind:"exact_duplicate"|"near_duplicate"|"burst"|"similar_series";sourceIds:string[];assetIds:string[];score:number;explanation:string;dismissed:boolean};
export type SmartCollectionProposal={name:string;matchMode:"all"|"any";rules:SmartRule[];explanation:string;requiresExplicitSave:true};

export type LibraryMetadataPatch = {
  title?: string | null;
  caption?: string | null;
  copyright?: string | null;
  creator?: string | null;
  rating?: number;
  decision?: Decision;
  addKeywords?: string[];
  removeKeywords?: string[];
  replaceKeywords?: string[];
};
export type SyncCategory = "tone" | "whiteBalance" | "presence" | "colour" | "transform" | "manualMasks" | "intelligentMasks" | "curve" | "colourMixer" | "colourGrading" | "detail" | "lensCorrections" | "lensAuto" | "lensExact";
export type BatchSummary = { requested: number; changed: number; failed: number; cancelled: number };
export type BatchAutoSummary = BatchSummary & { analyzable: number; lowConfidenceWhiteBalance: number; skipped: number };

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

export type CurvePoint = { x: number; y: number };
export type ToneCurves = { master: CurvePoint[]; red: CurvePoint[]; green: CurvePoint[]; blue: CurvePoint[] };
export type HslBand = { hue: number; saturation: number; luminance: number };
export type ColourMixer = { bands: HslBand[] };
export type GradeWheel = { hue: number; saturation: number };
export type ColourGrading = { shadows: GradeWheel; midtones: GradeWheel; highlights: GradeWheel; balance: number; blending: number };
export type DetailSettings = { sharpenAmount: number; sharpenRadius: number; sharpenDetail: number; sharpenMasking: number; luminanceNr: number; luminanceDetail: number; luminanceContrast: number; colourNr: number; colourDetail: number; colourSmoothness: number };
export type LensCorrections = { enabled: boolean; profileMode: "off" | "auto" | "manual"; profileId?: string; profileRevision?: string; profileAmount: number; manualDistortion: number; caRed: number; caBlue: number; vignetteAmount: number; vignetteMidpoint: number; constrainCrop: boolean };
export type AdvancedDevelopSettings = { curves: ToneCurves; colourMixer: ColourMixer; colourGrading: ColourGrading; detail: DetailSettings; lens: LensCorrections };
const identityCurve = (): CurvePoint[] => [{ x: 0, y: 0 }, { x: 1, y: 1 }];
export const neutralAdvancedDevelop = (): AdvancedDevelopSettings => ({
  curves: { master: identityCurve(), red: identityCurve(), green: identityCurve(), blue: identityCurve() },
  colourMixer: { bands: Array.from({ length: 8 }, () => ({ hue: 0, saturation: 0, luminance: 0 })) },
  colourGrading: { shadows: { hue: 0, saturation: 0 }, midtones: { hue: 0, saturation: 0 }, highlights: { hue: 0, saturation: 0 }, balance: 0, blending: 50 },
  detail: { sharpenAmount: 0, sharpenRadius: 1, sharpenDetail: 25, sharpenMasking: 0, luminanceNr: 0, luminanceDetail: 50, luminanceContrast: 0, colourNr: 0, colourDetail: 50, colourSmoothness: 50 },
  lens: { enabled: false, profileMode: "off", profileAmount: 100, manualDistortion: 0, caRed: 0, caBlue: 0, vignetteAmount: 0, vignetteMidpoint: 50, constrainCrop: true },
});

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
export type DevelopRecipe = { schemaVersion: 3; settings: BasicAdjustments; advanced: AdvancedDevelopSettings; masks: DevelopMask[] };
export type IntelligentMaskHealth = {
  available: boolean; installed: boolean; runtimeAvailable: boolean; loaded: boolean; busy: boolean;
  provider: string; providerVersion: string; model: string; modelRevision: string; licence: string;
  source: string; approximateBytes: number; storagePath: string; executionProvider: string; detail: string;
};
export type IntelligentMaskProposal = {
  requestId: number; category: IntelligentMaskCategory; confidence: number; coverageFraction: number;
  elapsedMs: number; timings: { loadMs: number; preprocessMs: number; inferenceMs: number; postprocessMs: number }; mask: DevelopMask;
};
export type PresetCategory = "whiteBalance" | "tone" | "presence" | "colour" | "curve" | "colourMixer" | "colourGrading" | "detail" | "lensCorrections";
export type DevelopPreset = { schemaVersion: 2; id: string; name: string; categories: PresetCategory[]; settings: BasicAdjustments; advanced: AdvancedDevelopSettings; builtIn: boolean };
export type LensProfile = { id: string; cameraContains: string; lens: string; focalLength: number; aperture: number; distortion: [number, number, number]; caScale: [number, number]; vignette: [number, number, number]; sourceFile: string };
export type LensProfileStatus = { camera?: string; lens?: string; focalLength?: number; aperture?: number; iso?: number; matchedProfile?: LensProfile; profiles: LensProfile[]; dataRevision: string; dataLicence: string; detail: string };
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
export type IntegrityReport = { scannedAssets: number; missingOriginals: number; missingDerivedVersions: number; modifiedOriginals: number; missingAiDerivatives: number; modifiedAiDerivatives: number; orphanAiDerivatives: number; unsupportedAiProvenance: number; untrackedManagedFiles: number; sidecarConflicts: number; findings: IntegrityFinding[] };
export type RelinkCandidate = { path: string; sha256: string };
export type FolderWatchEvent = { id: number; folderPath: string; path: string; kind: "new_file" | "file_changed" | "file_removed" | string; observedAt: string };

export type AiEnhancementOperation = "denoise" | "super_resolution";
export type AiEnhancementModelStatus = { operation: AiEnhancementOperation; installed: boolean; loaded: boolean; provider: string; providerVersion: string; model: string; modelRevision: string; modelSha256: string; licence: string; source: string; approximateBytes: number; storagePath: string; executionProvider: string; loadedMs: number; supportedScales: number[] };
export type AiEnhancementHealth = { runtimeAvailable: boolean; busy: boolean; cudaAvailable: boolean; cudaFreeBytes: number; cudaTotalBytes: number; models: AiEnhancementModelStatus[] };
export type AiEnhancementRequest = { assetId: string; operation: AiEnhancementOperation; targetScale: 1 | 2 | 4; previewRegion?: { x: number; y: number; width: number; height: number }; allowSlowCpuFallback: boolean };
export type AiEnhancementTimings = { loadMs: number; preprocessMs: number; inferenceMs: number; postprocessMs: number; writeValidateMs: number; totalMs: number };
export type AiEnhancementProvenance = { operation: AiEnhancementOperation; provider: string; providerVersion: string; model: string; modelRevision: string; modelSha256: string; executionProvider: string; scale: number; tileSize: number; overlap: number; tilePeakBytes: number; timings: AiEnhancementTimings };
export type AiEnhancementPreview = { previewId: string; beforePath: string; afterPath: string; beforeUrl: string; afterUrl: string; width: number; height: number; provenance: AiEnhancementProvenance };
export type AiDerivative = { derivativeId: string; assetId: string; parentSourceId: string; rootSourceId: string; sourceAssetId: string; parentAssetId: string; displayName: string; managedRelativePath: string; outputSha256: string; width: number; height: number; provenance: AiEnhancementProvenance };
export type AiEnhancementJob = { id: string; assetId: string; operation: AiEnhancementOperation; state: string; stage: string; tilesComplete: number; tilesTotal: number; error?: string; resultAssetId?: string; createdAt: string; updatedAt: string };
export type AiEnhancementProgress = { jobId?: string; assetId: string; current: number; total: number; stage: string; resultAssetId?: string };

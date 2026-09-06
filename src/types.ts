export type ViewName = "library" | "triage" | "map" | "workshop" | "trash" | "settings";
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
  tags: string[];
  representationCount: number;
  preferredVersionUrl?: string;
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
};

export const neutralAdjustments: BasicAdjustments = {
  exposure: 0, lightBalance: 0, tint: 0, contrast: 0, highlights: 0, shadows: 0,
  whites: 0, blacks: 0, dynamicRange: 0, texture: 0, clarity: 0, dehaze: 0,
  colourBoost: 0, saturation: 0, curveHighlights: 0, curveLights: 0,
  curveDarks: 0, curveShadows: 0,
};

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

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { neutralAdjustments, type AiAction, type Asset, type AssetFilter, type AssetPage, type AssetVersion, type AutoProposal, type BasicAdjustments, type BatchJob, type Decision, type DevelopPreset, type DevelopRecipe, type EditIntent, type EditRecipe, type FolderWatchEvent, type ImportOptions, type ImportSummary, type IntegrityReport, type LibraryStatus, type MapAsset, type MapBounds, type PromptSet, type RelinkCandidate, type ServiceHealth, type SidecarExportSummary, type SidecarImportResult, type TrashSummary } from "../types";
import { demoAssets, demoJobs, demoStatus, makeRecipe, renderPrompts } from "./demo";

const tauri = () => "__TAURI_INTERNALS__" in window;
let browserAssets = structuredClone(demoAssets);
const browserDevelopRecipes = new Map<string, DevelopRecipe>();
let browserDevelopPresets: DevelopPreset[] = [];
const browserBuiltInDevelopPresets: DevelopPreset[] = [
  ["natural", "Natural", {}], ["clean", "Clean", { contrast: 4, clarity: 3, colourBoost: 3 }], ["warm", "Warm", { lightBalance: 14, tint: 2, colourBoost: 6 }], ["cool", "Cool", { lightBalance: -12, tint: -2, highlights: -8 }], ["high-contrast", "High Contrast", { contrast: 20, whites: 8, blacks: -10 }], ["soft-contrast", "Soft Contrast", { contrast: -14, highlights: -12, shadows: 12 }], ["vivid", "Vivid", { contrast: 8, colourBoost: 20, saturation: 5 }], ["muted", "Muted", { contrast: -4, colourBoost: -8, saturation: -22 }], ["portrait", "Portrait", { highlights: -10, shadows: 8, texture: -12, clarity: -5, colourBoost: 5 }], ["landscape", "Landscape", { contrast: 10, dehaze: 7, clarity: 8, colourBoost: 15 }], ["black-and-white", "Black & White", { contrast: 10, clarity: 4, saturation: -100 }], ["high-key-bw", "High-Key B&W", { exposure: .45, contrast: -8, shadows: 18, blacks: 10, saturation: -100 }], ["low-key-bw", "Low-Key B&W", { exposure: -.45, contrast: 18, highlights: -15, blacks: -18, saturation: -100 }],
].map(([id, name, changes]) => ({ schemaVersion: 1, id: id as string, name: name as string, categories: ["whiteBalance", "tone", "presence", "colour"], settings: { ...neutralAdjustments, ...(changes as Partial<BasicAdjustments>) }, builtIn: true }));
let browserJobs = structuredClone(demoJobs);
const browserTrash = new Set<string>();
type BrowserHistory =
  | { kind: "decision"; id: string; decision: Decision }
  | { kind: "tags"; id: string; tags: string[] }
  | { kind: "location"; id: string; latitude?: number; longitude?: number };
const history: BrowserHistory[] = [];

const withAssetUrls = (asset: Asset): Asset => {
  const normalised = { ...asset, latitude: asset.latitude ?? undefined, longitude: asset.longitude ?? undefined, locationSource: asset.locationSource ?? (asset.latitude != null && asset.longitude != null ? "embedded" : "none"), missingState: asset.missingState ?? "available" };
  if (!tauri()) return normalised;
  const toUrl = (value: string) => value.startsWith("data:") || value.startsWith("http") ? value : convertFileSrc(value);
  return {
    ...normalised,
    previewUrl: toUrl(normalised.previewUrl),
    thumbnailUrl: toUrl(normalised.thumbnailUrl),
    preferredVersionUrl: normalised.preferredVersionUrl ? toUrl(normalised.preferredVersionUrl) : undefined,
  };
};
const withMapAssetUrl = (asset: MapAsset): MapAsset => tauri()
  ? { ...asset, thumbnailUrl: convertFileSrc(asset.thumbnailUrl) }
  : asset;
const withJobUrls = (job: BatchJob): BatchJob => {
  if (!tauri()) return job;
  return {
    ...job,
    outputUrl: job.outputUrl ? convertFileSrc(job.outputUrl) : undefined,
    attempts: job.attempts.map((attempt) => ({ ...attempt, outputUrl: attempt.outputUrl ? convertFileSrc(attempt.outputUrl) : undefined })),
  };
};
const withVersionUrl = (version: AssetVersion): AssetVersion => tauri()
  ? { ...version, imageUrl: convertFileSrc(version.imageUrl) }
  : version;

export const api = {
  isNative: tauri,
  async status(): Promise<LibraryStatus> {
    return tauri() ? invoke("get_library_status") : demoStatus(browserAssets);
  },
  async serviceHealth(force = false): Promise<ServiceHealth> {
    return tauri() ? invoke("get_service_health", { force }) : { localAiAvailable: true, serviceReachable: true, localAiBusy: false, localAiModel: "Qwen-Image-Edit", localAiDetail: "The local Qwen image editor is ready.", localAiState: "available", localAiUrl: "http://127.0.0.1:7868", analysisModelInstalled: false, analysisAvailable: false, analysisDetail: "Demo mode uses the deterministic controls-only fallback." };
  },
  async configureLocalAi(url: string): Promise<void> {
    if (tauri()) await invoke("configure_local_ai", { url });
  },
  async catalogueIntegrity(): Promise<string> {
    return tauri() ? invoke("check_catalogue_integrity") : "ok";
  },
  async createBackup(): Promise<string> {
    if (!tauri()) return "D:\\Photo Library\\.keepframe\\backups\\catalogue-manual.sqlite";
    const path = await invoke<string>("create_catalogue_backup");
    await revealItemInDir(path);
    return path;
  },
  async restoreBackup(): Promise<boolean> {
    if (!tauri()) return true;
    const path = await open({ multiple: false, directory: false, title: "Choose a verified Keepframe catalogue backup", filters: [{ name: "SQLite catalogue", extensions: ["sqlite"] }] });
    if (typeof path !== "string") return false;
    await invoke("restore_catalogue_backup", { path });
    return true;
  },
  async rebuildThumbnails(): Promise<number> {
    return tauri() ? invoke("rebuild_thumbnails") : browserAssets.length;
  },
  async exportDiagnostics(): Promise<string | null> {
    if (!tauri()) return "D:\\keepframe-diagnostics.json";
    const destination = await open({ directory: true, multiple: false, title: "Choose a diagnostics export folder" });
    if (typeof destination !== "string") return null;
    const path = await invoke<string>("export_diagnostics", { destination });
    await revealItemInDir(path);
    return path;
  },
  async exportSidecars(assetIds: string[], replaceExisting = false): Promise<SidecarExportSummary> {
    if (!tauri()) return { requested: assetIds.length, written: assetIds.length, preservedExisting: 0, failed: [] };
    return invoke("export_xmp_sidecars", { assetIds, replaceExisting });
  },
  async importSidecar(assetId: string): Promise<SidecarImportResult> {
    if (!tauri()) return { tagsImported: false, locationImported: false, triageImported: false, conflicts: [] };
    return invoke("import_xmp_sidecar", { assetId });
  },
  async exportPortableCatalogue(): Promise<string | null> {
    if (!tauri()) return "D:\\Photo Library\\keepframe-portable-catalogue-v1.json";
    const destination = await open({ directory: true, multiple: false, title: "Choose a local folder for the portable catalogue export" });
    if (typeof destination !== "string") return null;
    const path = await invoke<string>("export_portable_catalogue", { destination });
    await revealItemInDir(path);
    return path;
  },
  async rescanLibrary(): Promise<IntegrityReport> {
    if (!tauri()) return { scannedAssets: browserAssets.length, missingOriginals: 0, missingDerivedVersions: 0, modifiedOriginals: 0, untrackedManagedFiles: 0, sidecarConflicts: 0, findings: [] };
    return invoke("rescan_library");
  },
  async relinkSelectedAsset(assetId: string): Promise<string | null> {
    if (!tauri()) return null;
    const directory = await open({ directory: true, multiple: false, title: "Choose a folder to search for an exact relink match" });
    if (typeof directory !== "string") return null;
    const candidates = await invoke<RelinkCandidate[]>("find_relink_candidates", { assetId, directory });
    if (!candidates.length) throw new Error("No hash-confirmed relink candidate was found in that folder.");
    let path = candidates[0].path;
    if (candidates.length > 1) {
      const options = candidates.map((candidate, index) => `${index + 1}. ${candidate.path}`).join("\n");
      const choice = window.prompt(`Multiple exact hash matches were found. Enter the number to relink; cancel leaves the catalogue unchanged.\n\n${options}`, "1");
      const index = Number(choice) - 1;
      if (!Number.isInteger(index) || index < 0 || index >= candidates.length) return null;
      path = candidates[index].path;
    }
    await invoke("relink_asset", { assetId, path });
    return path;
  },
  async addFolderWatch(): Promise<boolean> {
    if (!tauri()) return true;
    const path = await open({ directory: true, multiple: false, title: "Choose a source folder to watch for changes" });
    if (typeof path !== "string") return false;
    await invoke("configure_folder_watch", { path, enabled: true });
    return true;
  },
  async disableFolderWatches(): Promise<void> {
    if (tauri()) await invoke("disable_folder_watches");
  },
  async folderWatchEvents(): Promise<FolderWatchEvent[]> {
    return tauri() ? invoke("list_folder_watch_events") : [];
  },
  async chooseLibrary(): Promise<string | null> {
    if (!tauri()) return "D:\\Photo Library";
    const result = await open({ directory: true, multiple: false, title: "Choose the Keepframe master library" });
    return typeof result === "string" ? result : null;
  },
  async initialiseLibrary(path: string): Promise<LibraryStatus> {
    return tauri() ? invoke("initialise_library", { path }) : demoStatus(browserAssets);
  },
  async chooseImport(): Promise<string[]> {
    if (!tauri()) return ["D:\\Camera card\\DCIM"];
    const result = await open({ directory: true, multiple: false, title: "Choose a folder to import" });
    return typeof result === "string" ? [result] : [];
  },
  async importPhotos(paths: string[], options: ImportOptions = { mode: "copy", duplicateSourcePolicy: "retain" }): Promise<ImportSummary> {
    if (tauri()) return invoke("import_photos", { paths, options });
    const imported = paths.length ? 6 : 0;
    return { importId: crypto.randomUUID(), state: "completed", discovered: imported, imported, copied: options.mode === "copy" ? imported : 0, moved: options.mode === "move" ? imported : 0, sourceRetained: options.mode === "copy" ? imported : 0, duplicates: 0, unsupported: 0, failed: 0 };
  },
  async cancelImport(importId: string): Promise<void> {
    if (tauri()) return invoke("cancel_import", { importId });
  },
  async assets(filter: AssetFilter, offset = 0, limit = 240): Promise<AssetPage> {
    if (tauri()) {
      const page = await invoke<AssetPage>("query_assets", { filter, offset, limit });
      return { ...page, items: page.items.map(withAssetUrls) };
    }
    const query = filter.search.toLocaleLowerCase();
    const matching = browserAssets.filter((asset) => {
      if (browserTrash.has(asset.id) !== Boolean(filter.trashed)) return false;
      if (filter.decision !== "all" && asset.decision !== filter.decision) return false;
      if (filter.year && new Date(asset.capturedAt).getFullYear() !== filter.year) return false;
      if (filter.tag && !asset.tags.includes(filter.tag)) return false;
      return !query || `${asset.filename} ${asset.camera ?? ""} ${asset.tags.join(" ")}`.toLocaleLowerCase().includes(query);
    });
    const items = matching.slice(offset, offset + limit);
    return { items, total: matching.length, offset, limit, hasMore: offset + items.length < matching.length };
  },
  async asset(id: string): Promise<Asset | null> {
    if (tauri()) {
      const asset = await invoke<Asset | null>("get_asset", { assetId: id });
      return asset ? withAssetUrls(asset) : null;
    }
    return browserAssets.find((asset) => asset.id === id) ?? null;
  },
  async mapAssets(filter: AssetFilter, bounds?: MapBounds): Promise<MapAsset[]> {
    if (tauri()) return (await invoke<MapAsset[]>("query_map_assets", { query: { filter, bounds } })).map(withMapAssetUrl);
    const page = await this.assets({ ...filter, located: true }, 0, Number.MAX_SAFE_INTEGER);
    return page.items
      .filter((asset) => asset.latitude != null && asset.longitude != null)
      .filter((asset) => !bounds || (asset.latitude! >= bounds.south && asset.latitude! <= bounds.north && (bounds.west <= bounds.east ? asset.longitude! >= bounds.west && asset.longitude! <= bounds.east : asset.longitude! >= bounds.west || asset.longitude! <= bounds.east)))
      .map((asset) => ({ id: asset.id, filename: asset.filename, latitude: asset.latitude!, longitude: asset.longitude!, capturedAt: asset.capturedAt, thumbnailUrl: asset.thumbnailUrl, decision: asset.decision, locationSource: asset.locationSource ?? "embedded" }));
  },
  async assetIds(filter: AssetFilter): Promise<string[]> {
    if (tauri()) return invoke("query_asset_ids", { filter });
    const page = await this.assets(filter, 0, Number.MAX_SAFE_INTEGER);
    return page.items.map((asset) => asset.id);
  },
  async setDecision(id: string, decision: Decision): Promise<void> {
    if (tauri()) return invoke("set_decision", { assetId: id, decision });
    const asset = browserAssets.find((item) => item.id === id);
    if (asset && asset.decision !== decision) { history.push({ kind: "decision", id, decision: asset.decision }); asset.decision = decision; }
  },
  async undo(): Promise<boolean> {
    if (tauri()) return invoke("undo_last_action");
    const prior = history.pop();
    const asset = prior && browserAssets.find((item) => item.id === prior.id);
    if (!asset || !prior) return false;
    if (prior.kind === "decision") asset.decision = prior.decision;
    if (prior.kind === "tags") asset.tags = prior.tags;
    if (prior.kind === "location") { asset.latitude = prior.latitude; asset.longitude = prior.longitude; }
    return true;
  },
  async moveToTrash(assetIds: string[]): Promise<TrashSummary> {
    if (tauri()) return invoke("move_to_trash", { assetIds });
    let affected = 0;
    for (const id of assetIds) {
      const asset = browserAssets.find((item) => item.id === id);
      if (asset?.decision === "discard" && !browserTrash.has(id)) { browserTrash.add(id); affected += 1; }
    }
    return { affected, failed: 0 };
  },
  async restoreFromTrash(assetIds: string[]): Promise<TrashSummary> {
    if (tauri()) return invoke("restore_from_trash", { assetIds });
    let affected = 0;
    for (const id of assetIds) if (browserTrash.delete(id)) affected += 1;
    return { affected, failed: 0 };
  },
  async emptyTrash(): Promise<TrashSummary> {
    if (tauri()) return invoke("empty_trash", { operationIds: [] });
    const affected = browserTrash.size;
    browserAssets = browserAssets.filter((asset) => !browserTrash.has(asset.id));
    browserTrash.clear();
    return { affected, failed: 0 };
  },
  async updateTags(id: string, tags: string[]): Promise<void> {
    if (tauri()) return invoke("update_tags", { assetId: id, tags });
    const asset = browserAssets.find((item) => item.id === id); if (asset) { history.push({ kind: "tags", id, tags: [...asset.tags] }); asset.tags = tags; }
  },
  async updateLocation(id: string, latitude: number, longitude: number): Promise<void> {
    if (tauri()) return invoke("update_location", { assetId: id, latitude, longitude });
    const asset = browserAssets.find((item) => item.id === id); if (asset) { history.push({ kind: "location", id, latitude: asset.latitude, longitude: asset.longitude }); asset.latitude = latitude; asset.longitude = longitude; asset.locationSource = "manual"; }
  },
  async updateLocations(ids: string[], latitude: number, longitude: number): Promise<number> {
    if (tauri()) return invoke("update_locations", { assetIds: ids, latitude, longitude });
    for (const id of ids) await this.updateLocation(id, latitude, longitude);
    return ids.length;
  },
  async clearManualLocation(id: string): Promise<boolean> {
    if (tauri()) return invoke("clear_manual_location", { assetId: id });
    const asset = browserAssets.find((item) => item.id === id);
    if (!asset || asset.locationSource !== "manual") return false;
    asset.latitude = undefined; asset.longitude = undefined; asset.locationSource = "none";
    return true;
  },
  async analyse(asset: Asset, intent: EditIntent, action?: AiAction, commonBrief?: string): Promise<EditRecipe> {
    if (tauri()) return invoke("create_edit_recipe", { assetId: asset.id, intent, action, commonBrief });
    return { ...makeRecipe(asset, intent), action, commonBrief };
  },
  async prompts(recipe: EditRecipe): Promise<PromptSet> {
    return tauri() ? invoke("render_prompts", { recipe }) : renderPrompts(recipe);
  },
  async copy(text: string): Promise<void> {
    if (tauri()) return writeText(text);
    await navigator.clipboard?.writeText(text);
  },
  async prepareCloud(assetId: string, provider: "chatgpt" | "gemini", prompt: string): Promise<string> {
    if (!tauri()) return `D:\\Photo Library\\Exports\\${assetId}-${provider}.png`;
    const path = await invoke<string>("prepare_cloud_export", { assetId, provider, prompt });
    await revealItemInDir(path); return path;
  },
  async exportExternal(assetId: string, provider: "chatgpt" | "gemini", prompt: string): Promise<string | null> {
    if (!tauri()) return `D:\\External AI\\${assetId}-${provider}.png`;
    const destination = await open({ directory: true, multiple: false, title: "Choose the external AI export folder" });
    if (typeof destination !== "string") return null;
    const path = await invoke<string>("export_external_edit", { assetId, provider, prompt, destination });
    await revealItemInDir(path);
    return path;
  },
  async versions(asset: Asset): Promise<AssetVersion[]> {
    if (!tauri()) return [{ id: `original:${asset.id}`, kind: "original", provider: "Keepframe protected original", createdAt: asset.capturedAt, state: "protected", imageUrl: asset.previewUrl, isPreferred: !asset.preferredVersionUrl }];
    return (await invoke<AssetVersion[]>("list_versions", { assetId: asset.id })).map(withVersionUrl);
  },
  async reviewPreview(asset: Asset): Promise<string> {
    if (!tauri()) return asset.previewUrl;
    return convertFileSrc(await invoke<string>("prepare_review_preview", { assetId: asset.id }));
  },
  async setPreferredVersion(assetId: string, versionId?: string): Promise<void> {
    if (tauri()) return invoke("set_preferred_version", { assetId, versionId: versionId ?? null });
  },
  async autoBasicAdjustments(assetId: string): Promise<BasicAdjustments> {
    if (tauri()) return invoke("auto_basic_adjustments", { assetId });
    return { ...neutralAdjustments, dynamicRange: 100, contrast: 8, highlights: -30, shadows: 20, whites: 6, blacks: -7, clarity: 7, dehaze: 3, colourBoost: 9, saturation: 2 };
  },
  async previewBasicAdjustments(asset: Asset, adjustments: BasicAdjustments): Promise<string> {
    if (tauri()) return convertFileSrc(await invoke<string>("preview_basic_adjustments", { assetId: asset.id, adjustments }));
    return "";
  },
  async applyBasicAdjustments(asset: Asset, adjustments: BasicAdjustments): Promise<AssetVersion> {
    if (tauri()) return withVersionUrl(await invoke<AssetVersion>("apply_basic_adjustments", { assetId: asset.id, adjustments }));
    return { id: crypto.randomUUID(), kind: "adjusted", provider: "keepframe-controls", createdAt: new Date().toISOString(), state: "candidate", imageUrl: asset.preferredVersionUrl ?? asset.previewUrl, isPreferred: false };
  },
  async getDevelopRecipe(assetId: string): Promise<DevelopRecipe> {
    return tauri() ? invoke("get_develop_recipe", { assetId }) : browserDevelopRecipes.get(assetId) ?? { schemaVersion: 1, settings: { ...neutralAdjustments } };
  },
  async saveDevelopRecipe(assetId: string, recipe: DevelopRecipe): Promise<boolean> {
    if (tauri()) return invoke("save_develop_recipe", { assetId, recipe });
    const edited = JSON.stringify(recipe.settings) !== JSON.stringify(neutralAdjustments);
    if (edited) browserDevelopRecipes.set(assetId, structuredClone(recipe)); else browserDevelopRecipes.delete(assetId);
    const asset = browserAssets.find((item) => item.id === assetId); if (asset) asset.hasEdits = edited;
    return edited;
  },
  async previewDevelopRecipe(assetId: string, recipe: DevelopRecipe): Promise<string> {
    return tauri() ? convertFileSrc(await invoke<string>("preview_develop_recipe", { assetId, recipe })) : "";
  },
  async developPresets(): Promise<DevelopPreset[]> { return tauri() ? invoke("list_develop_presets") : structuredClone([...browserBuiltInDevelopPresets, ...browserDevelopPresets]); },
  async saveDevelopPreset(preset: DevelopPreset): Promise<DevelopPreset> {
    if (tauri()) return invoke("save_develop_preset", { preset });
    const stored = { ...structuredClone(preset), builtIn: false }; browserDevelopPresets = [...browserDevelopPresets.filter((entry) => entry.id !== stored.id && entry.name.toLowerCase() !== stored.name.toLowerCase()), stored]; return stored;
  },
  async deleteDevelopPreset(id: string): Promise<void> { if (tauri()) return invoke("delete_develop_preset", { id }); browserDevelopPresets = browserDevelopPresets.filter((preset) => preset.id !== id); },
  async exportDevelopPreset(id: string): Promise<void> {
    if (!tauri()) return;
    const path = await save({ title: "Export Develop preset", defaultPath: "keepframe-preset.keepframe-preset", filters: [{ name: "Keepframe preset", extensions: ["keepframe-preset"] }] });
    if (typeof path === "string") await invoke("export_develop_preset", { id, path });
  },
  async importDevelopPreset(): Promise<DevelopPreset | null> {
    if (!tauri()) return null;
    const path = await open({ multiple: false, directory: false, title: "Import Develop preset", filters: [{ name: "Keepframe preset", extensions: ["keepframe-preset"] }] });
    return typeof path === "string" ? invoke("import_develop_preset", { path }) : null;
  },
  async proposeDevelopAuto(assetId: string, current: BasicAdjustments): Promise<AutoProposal> {
    if (tauri()) return invoke("propose_develop_auto", { assetId, current });
    return { settings: { ...current, exposure: Math.min(3, current.exposure + .25), highlights: Math.max(-100, current.highlights - 12), shadows: Math.min(100, current.shadows + 10) }, explanation: ["Exposure increased because midtones were under-represented.", "Highlights reduced because clipping was detected.", "White balance unchanged because no reliable neutral estimate was found."], confidence: .72, statistics: { luminanceBins: [], redBins: [], greenBins: [], blueBins: [], samples: 0, averageLuminance: .38, p01: .03, p50: .35, p99: .95, shadowClipFraction: .02, highlightClipFraction: .01, averageSaturation: .26, redGreenBlue: [.4, .38, .36], dynamicRange: .92 }, recommendations: ["Landscape"] };
  },
  async chooseReplacement(asset: Asset): Promise<AssetVersion | null> {
    if (!tauri()) return null;
    const path = await open({ multiple: false, directory: false, title: "Choose a replacement image", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "tif", "tiff"] }] });
    if (typeof path !== "string") return null;
    return withVersionUrl(await invoke<AssetVersion>("import_replacement", { assetId: asset.id, path }));
  },
  async exportImage(asset: Asset): Promise<string | null> {
    if (!tauri()) return null;
    const destination = await open({ directory: true, multiple: false, title: "Choose where to export the photograph" });
    if (typeof destination !== "string") return null;
    const path = await invoke<string>("export_asset_image", { assetId: asset.id, destination });
    await revealItemInDir(path);
    return path;
  },
  async importReturned(assetId: string, provider: "chatgpt" | "gemini", prompt: string, recipe?: EditRecipe): Promise<BatchJob | null> {
    if (!tauri()) {
      const asset = browserAssets.find((item) => item.id === assetId); if (!asset) return null;
      const job: BatchJob = { id: crypto.randomUUID(), batchId: crypto.randomUUID(), assetId, assetName: asset.filename, state: "succeeded", prompt, attempts: [], outputUrl: asset.previewUrl };
      browserJobs = [job, ...browserJobs]; return job;
    }
    const path = await open({ multiple: false, directory: false, title: "Choose the returned edited image", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "tif", "tiff"] }] });
    if (typeof path !== "string") return null;
    return invoke("import_returned_edit", { assetId, path, provider, prompt, recipe });
  },
  async jobs(): Promise<BatchJob[]> {
    return tauri() ? (await invoke<BatchJob[]>("list_jobs")).map(withJobUrls) : browserJobs;
  },
  async enqueue(assetIds: string[], prompt: string): Promise<BatchJob[]> {
    if (tauri()) return invoke("enqueue_batch", { assetIds, commonBrief: prompt });
    const additions = assetIds.map((assetId) => {
      const asset = browserAssets.find((item) => item.id === assetId)!;
      const recipe = { ...makeRecipe(asset, "restoration"), commonBrief: prompt };
      return { id: crypto.randomUUID(), batchId: crypto.randomUUID(), assetId, assetName: asset.filename, state: "review_required" as const, prompt: renderPrompts(recipe).local, recipe, attempts: [] };
    });
    browserJobs = [...browserJobs, ...additions]; return additions;
  },
  async enqueueReviewed(assetId: string, recipe: EditRecipe, prompt: string): Promise<BatchJob> {
    if (tauri()) return invoke("enqueue_reviewed_recipe", { assetId, recipe, prompt });
    const asset = browserAssets.find((item) => item.id === assetId)!;
    const job: BatchJob = { id: crypto.randomUUID(), batchId: crypto.randomUUID(), assetId, assetName: asset.filename, state: "review_required", prompt, recipe, attempts: [] };
    browserJobs = [job, ...browserJobs]; return job;
  },
  async saveJobReview(id: string, recipe: EditRecipe, prompt: string): Promise<void> {
    if (tauri()) return invoke("save_job_review", { jobId: id, recipe, prompt });
    const job = browserJobs.find((item) => item.id === id);
    if (job?.state === "review_required") { job.recipe = recipe; job.prompt = prompt; }
  },
  async approveJobs(ids: string[]): Promise<void> {
    if (tauri()) return invoke("approve_jobs", { jobIds: ids });
    for (const job of browserJobs) if (ids.includes(job.id) && job.state === "review_required") job.state = "queued";
  },
  async updateJob(id: string, action: "approve" | "cancel" | "retry" | "accept" | "reject"): Promise<void> {
    if (tauri()) return invoke("update_job", { jobId: id, action });
    const job = browserJobs.find((item) => item.id === id);
    if (job) job.state = ({ approve: "queued", cancel: "cancelled", retry: "queued", accept: "accepted", reject: "rejected" } as const)[action];
  },
};

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { neutralAdjustments, type AiAction, type Asset, type AssetFilter, type AssetPage, type AssetVersion, type AutoProposal, type BasicAdjustments, type BatchAutoSummary, type BatchJob, type BatchSummary, type CatalogueCollection, type CatalogueVersion, type CollectionSet, type Decision, type DevelopPreset, type DevelopRecipe, type DiscoveryGroup, type EditIntent, type EditRecipe, type ExportBatchReport, type ExportConfig, type ExportPreset, type ExportProgress, type FolderWatchEvent, type ImportOptions, type ImportSummary, type IntelligentMaskCategory, type IntelligentMaskHealth, type IntelligentMaskProposal, type IntegrityReport, type LibraryMetadataPatch, type LibraryStatus, type MapAsset, type MapBounds, type PromptSet, type RelinkCandidate, type SemanticIndexStatus, type SemanticSearchRequest, type SemanticSearchResponse, type SemanticSearchResult, type ServiceHealth, type SidecarExportSummary, type SidecarImportResult, type SmartCollectionProposal, type SmartRule, type StackSummary, type SyncCategory, type TrashSummary } from "../types";
import { demoAssets, demoJobs, demoStatus, makeRecipe, renderPrompts } from "./demo";

const tauri = () => "__TAURI_INTERNALS__" in window;
export const defaultExportConfig = (changes: Partial<ExportConfig> = {}): ExportConfig => ({ schemaVersion: 1, format: "jpeg", jpegQuality: 90, pngCompression: "balanced", resizeMode: "original", width: 2400, height: 1600, percentage: 100, noEnlarge: true, ppi: 300, sharpening: "standard", metadata: "all", includeLocation: false, includeKeywords: true, includeRating: true, filenameTemplate: "{stem}-{sequence}", customText: "", sequenceStart: 1, sequencePadding: 3, collision: "unique", colourSpace: "srgb", ...changes });
const intelligentMaskDemo = () => !tauri() && ["localhost", "127.0.0.1"].includes(window.location.hostname) && new URLSearchParams(window.location.search).has("intelligentMaskDemo");
const semanticDemo = () => !tauri() && ["localhost", "127.0.0.1"].includes(window.location.hostname) && new URLSearchParams(window.location.search).has("semanticDemo");
const browserMaskCoverage = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAYCAAAAAC+OKDoAAAAP0lEQVR4nGNgGAKAEZnzH4soEzZ5BgQLSe1/7EYjm4AVMOEwgOE/ySYMCQWMaBJEBxSyRqwxgGwCQhTdvgEGAKyOBxyY54qVAAAAAElFTkSuQmCC";
let browserAssets = structuredClone(demoAssets);
const browserDevelopRecipes = new Map<string, DevelopRecipe>();
let browserDevelopPresets: DevelopPreset[] = [];
let browserExportPresets: ExportPreset[] = [];
const browserBuiltInDevelopPresets: DevelopPreset[] = [
  ["natural", "Natural", {}], ["clean", "Clean", { contrast: 4, clarity: 3, colourBoost: 3 }], ["warm", "Warm", { lightBalance: 14, tint: 2, colourBoost: 6 }], ["cool", "Cool", { lightBalance: -12, tint: -2, highlights: -8 }], ["high-contrast", "High Contrast", { contrast: 20, whites: 8, blacks: -10 }], ["soft-contrast", "Soft Contrast", { contrast: -14, highlights: -12, shadows: 12 }], ["vivid", "Vivid", { contrast: 8, colourBoost: 20, saturation: 5 }], ["muted", "Muted", { contrast: -4, colourBoost: -8, saturation: -22 }], ["portrait", "Portrait", { highlights: -10, shadows: 8, texture: -12, clarity: -5, colourBoost: 5 }], ["landscape", "Landscape", { contrast: 10, dehaze: 7, clarity: 8, colourBoost: 15 }], ["black-and-white", "Black & White", { contrast: 10, clarity: 4, saturation: -100 }], ["high-key-bw", "High-Key B&W", { exposure: .45, contrast: -8, shadows: 18, blacks: 10, saturation: -100 }], ["low-key-bw", "Low-Key B&W", { exposure: -.45, contrast: 18, highlights: -15, blacks: -18, saturation: -100 }],
].map(([id, name, changes]) => ({ schemaVersion: 1, id: id as string, name: name as string, categories: ["whiteBalance", "tone", "presence", "colour"], settings: { ...neutralAdjustments, ...(changes as Partial<BasicAdjustments>) }, builtIn: true }));
let browserJobs = structuredClone(demoJobs);
const browserTrash = new Set<string>();
let browserCollections:CatalogueCollection[]=[];
let browserCollectionSets:CollectionSet[]=[];
let browserCollectionItems=new Map<string,Set<string>>();
let browserStacks:StackSummary[]=[];
const browserDismissedSuggestions=new Set<string>();
type BrowserHistory =
  | { kind: "decision"; id: string; decision: Decision }
  | { kind: "tags"; id: string; tags: string[] }
  | { kind: "location"; id: string; latitude?: number; longitude?: number }
  | { kind: "batch"; assets: Asset[]; recipes: Array<[string, DevelopRecipe | null]> };
const history: BrowserHistory[] = [];
const redoHistory: BrowserHistory[] = [];

const applyBrowserHistory = (entry: BrowserHistory): BrowserHistory | null => {
  if (entry.kind === "batch") {
    const ids = entry.assets.map(asset => asset.id);
    const inverse: BrowserHistory = { kind: "batch", assets: structuredClone(browserAssets.filter(asset => ids.includes(asset.id))), recipes: entry.recipes.map(([id]) => [id, structuredClone(browserDevelopRecipes.get(id) ?? null)]) };
    for (const snapshot of entry.assets) { const index = browserAssets.findIndex(asset => asset.id === snapshot.id); if (index >= 0) browserAssets[index] = structuredClone(snapshot); }
    for (const [id, recipe] of entry.recipes) { if (recipe) browserDevelopRecipes.set(id, structuredClone(recipe)); else browserDevelopRecipes.delete(id); }
    return inverse;
  }
  const asset = browserAssets.find(item => item.id === entry.id); if (!asset) return null;
  if (entry.kind === "decision") { const inverse: BrowserHistory = { kind: "decision", id: entry.id, decision: asset.decision }; asset.decision = entry.decision; return inverse; }
  if (entry.kind === "tags") { const inverse: BrowserHistory = { kind: "tags", id: entry.id, tags: [...asset.tags] }; asset.tags = [...entry.tags]; return inverse; }
  const inverse: BrowserHistory = { kind: "location", id: entry.id, latitude: asset.latitude, longitude: asset.longitude }; asset.latitude = entry.latitude; asset.longitude = entry.longitude; return inverse;
};

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

const snapshotBrowserBatch = (ids: string[]): BrowserHistory => ({ kind: "batch", assets: structuredClone(browserAssets.filter(asset => ids.includes(asset.id))), recipes: ids.map(id => [id, structuredClone(browserDevelopRecipes.get(id) ?? null)]) });
const applyRecipeCategories = (target: DevelopRecipe, source: DevelopRecipe, categories: readonly SyncCategory[]): DevelopRecipe => {
  const next = structuredClone(target); const include = new Set(categories);
  const copy = (keys: Array<keyof BasicAdjustments>) => keys.forEach(key => { (next.settings as unknown as Record<string, unknown>)[key] = source.settings[key]; });
  if (include.has("whiteBalance")) copy(["lightBalance", "tint"]);
  if (include.has("tone")) copy(["exposure", "contrast", "highlights", "shadows", "whites", "blacks", "dynamicRange", "curveHighlights", "curveLights", "curveDarks", "curveShadows"]);
  if (include.has("presence")) copy(["texture", "clarity", "dehaze"]);
  if (include.has("colour")) copy(["colourBoost", "saturation"]);
  if (include.has("transform")) copy(["cropLeft", "cropTop", "cropWidth", "cropHeight", "rotateQuadrants", "straighten", "horizontalFlip", "verticalFlip"]);
  if (include.has("manualMasks") || include.has("intelligentMasks")) {
    const semantic = (mask: DevelopRecipe["masks"][number]) => mask.geometry.kind === "semantic";
    next.masks = next.masks.filter(mask => semantic(mask) ? !include.has("intelligentMasks") : !include.has("manualMasks"));
    next.masks.push(...source.masks.filter(mask => semantic(mask) ? include.has("intelligentMasks") : include.has("manualMasks")).map(mask => ({ ...structuredClone(mask), id: crypto.randomUUID() })));
  }
  return next;
};

const browserRuleMatches=(asset:Asset,rule:SmartRule)=>{const text=(value:unknown)=>String(value??"").toLowerCase();const contains=(value:unknown)=>text(value).includes(text(rule.value));switch(rule.field){case"rating":return rule.operator==="gte"?asset.rating>=Number(rule.value):rule.operator==="lte"?asset.rating<=Number(rule.value):asset.rating===Number(rule.value);case"flag":return asset.decision===rule.value;case"edited":return Boolean(asset.hasEdits)===Boolean(rule.value);case"fileType":return rule.operator==="notContains"?!contains(asset.filename):contains(asset.filename);case"keyword":{const found=asset.tags.some(tag=>rule.operator==="is"?text(tag)===text(rule.value):contains(tag));return rule.operator==="notContains"?!found:found;}case"camera":return rule.operator==="notContains"?!contains(asset.camera):contains(asset.camera);case"versionStatus":return rule.value==="primary"?asset.isPrimary!==false:asset.isPrimary===false;case"hasMultipleVersions":return((asset.sourceVersionCount??1)>1)===Boolean(rule.value);case"stackStatus":return rule.value==="stacked"?Boolean(asset.stackId):!asset.stackId;case"captureDate":case"importDate":{const value=asset.capturedAt;return rule.operator==="before"?value<String(rule.value):rule.operator==="after"?value>String(rule.value):value>=String(rule.value)&&value<=String(rule.secondValue);}default:return true;}};
const browserCollectionMatches=(asset:Asset,id:string)=>{const collection=browserCollections.find(value=>value.id===id);if(!collection)return false;if(collection.kind==="manual")return browserCollectionItems.get(id)?.has(asset.id)??false;const results=collection.rules.map(rule=>browserRuleMatches(asset,rule));return collection.matchMode==="all"?results.every(Boolean):results.some(Boolean);};

export const api = {
  isNative: tauri,
  resetBrowserSession():void { if(tauri())return;browserAssets=structuredClone(demoAssets);browserDevelopRecipes.clear();browserDevelopPresets=[];browserExportPresets=[];browserJobs=structuredClone(demoJobs);browserTrash.clear();browserCollections=[];browserCollectionSets=[];browserCollectionItems=new Map();browserStacks=[];browserDismissedSuggestions.clear();history.length=0;redoHistory.length=0; },
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
    if (!tauri()) return "D:\\Photo Library\\keepframe-portable-catalogue-v4.json";
    const destination = await open({ directory: true, multiple: false, title: "Choose a local folder for the portable catalogue export" });
    if (typeof destination !== "string") return null;
    const path = await invoke<string>("export_portable_catalogue", { destination });
    await revealItemInDir(path);
    return path;
  },
  async importPortableCatalogue():Promise<number|null>{if(!tauri())return browserAssets.length;const path=await open({multiple:false,directory:false,title:"Choose a Keepframe portable catalogue",filters:[{name:"Keepframe portable catalogue",extensions:["json"]}]});if(typeof path!=="string")return null;return invoke("import_portable_catalogue",{path});},
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
      if (filter.rating !== undefined && asset.rating < filter.rating) return false;
      if (filter.edited !== undefined && Boolean(asset.hasEdits) !== filter.edited) return false;
      if (filter.fileType && !asset.filename.toLowerCase().endsWith(`.${filter.fileType.toLowerCase().replace(/^\./, "")}`)) return false;
      if(filter.collectionId&&!browserCollectionMatches(asset,filter.collectionId))return false;
      if(filter.versionMode==="primary"&&asset.isPrimary===false)return false;if(filter.versionMode==="virtual"&&asset.isPrimary!==false)return false;
      if(filter.stackMode==="stacked"&&!asset.stackId)return false;if(filter.stackMode==="unstacked"&&asset.stackId)return false;
      if(asset.versionGroupCollapsed&&asset.isPrimary===false)return false;if(asset.stackCollapsed&&asset.stackId&&!asset.isStackTop)return false;
      return !query || `${asset.filename} ${asset.versionName??""} ${asset.camera ?? ""} ${asset.tags.join(" ")}`.toLocaleLowerCase().includes(query);
    });
    const direction = filter.descending === false ? 1 : -1;
    matching.sort((left, right) => direction * (filter.sort === "filename" ? left.filename.localeCompare(right.filename) : filter.sort === "rating" ? left.rating - right.rating : left.capturedAt.localeCompare(right.capturedAt)) || left.id.localeCompare(right.id));
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
  async catalogueVersions(itemId:string):Promise<CatalogueVersion[]>{if(tauri())return invoke("list_catalogue_versions",{itemId});const item=browserAssets.find(asset=>asset.id===itemId);if(!item)return[];const sourceId=item.sourceId??item.id;return browserAssets.filter(asset=>(asset.sourceId??asset.id)===sourceId).map((asset,index)=>({id:asset.id,sourceId,name:asset.versionName??(asset.isPrimary===false?`Version ${asset.versionIndex??index+1}`:"Primary"),isPrimary:asset.isPrimary!==false,versionIndex:asset.versionIndex??index+1,hasEdits:Boolean(asset.hasEdits),rating:asset.rating,decision:asset.decision}));},
  async createCatalogueVersion(itemId:string,mode:"current"|"default"|"duplicate",name?:string):Promise<Asset>{if(tauri())return withAssetUrls(await invoke("create_catalogue_version",{request:{itemId,mode,name}}));const source=browserAssets.find(asset=>asset.id===itemId);if(!source)throw new Error("That catalogue item no longer exists.");const sourceId=source.sourceId??source.id;const siblings=browserAssets.filter(asset=>(asset.sourceId??asset.id)===sourceId);const versionIndex=Math.max(1,...siblings.map(asset=>asset.versionIndex??1))+1;const id=crypto.randomUUID();const copyMetadata=mode==="duplicate";const created:Asset={...structuredClone(source),id,sourceId,isPrimary:false,versionIndex,versionName:name?.trim()||`Version ${versionIndex}`,decision:copyMetadata?source.decision:"undecided",rating:copyMetadata?source.rating:0,title:copyMetadata?source.title:undefined,caption:copyMetadata?source.caption:undefined,tags:copyMetadata?[...source.tags]:[],sourceVersionCount:siblings.length+1};for(const sibling of siblings)sibling.sourceVersionCount=siblings.length+1;browserAssets.push(created);if(mode!=="default"&&browserDevelopRecipes.has(itemId))browserDevelopRecipes.set(id,structuredClone(browserDevelopRecipes.get(itemId)!));return created;},
  async renameCatalogueVersion(itemId:string,name:string):Promise<void>{if(tauri())return invoke("rename_catalogue_version",{itemId,name});const asset=browserAssets.find(value=>value.id===itemId);if(!asset||asset.isPrimary!==false)throw new Error("Only virtual versions can be renamed.");asset.versionName=name.trim();},
  async deleteCatalogueVersion(itemId:string):Promise<void>{if(tauri()){await invoke("delete_catalogue_version",{itemId});return;}const asset=browserAssets.find(value=>value.id===itemId);if(!asset||asset.isPrimary!==false)throw new Error("The primary item cannot be deleted as a version.");browserAssets=browserAssets.filter(value=>value.id!==itemId);browserDevelopRecipes.delete(itemId);for(const items of browserCollectionItems.values())items.delete(itemId);},
  async setVersionGroupCollapsed(itemId:string,collapsed:boolean):Promise<void>{if(tauri())return invoke("set_version_group_collapsed",{itemId,collapsed});const item=browserAssets.find(value=>value.id===itemId);if(!item)return;for(const asset of browserAssets.filter(value=>(value.sourceId??value.id)===(item.sourceId??item.id)))asset.versionGroupCollapsed=collapsed;},
  async semanticIndexStatus():Promise<SemanticIndexStatus>{if(tauri())return invoke("get_semantic_index_status");const ready=semanticDemo();return{available:ready,installed:ready,runtimeAvailable:ready,loaded:ready,busy:false,paused:false,provider:"Keepframe local SigLIP semantic provider",providerVersion:"1.0.0",model:"google/siglip-base-patch16-224",modelRevision:"7fd15f0689c79d79e38b1c2e2e2370a7bf2761ed",licence:"Apache-2.0",source:"https://huggingface.co/google/siglip-base-patch16-224",approximateBytes:815871927,storagePath:"D:\\AI Models\\Keepframe\\semantic\\siglip-base-patch16-224",executionProvider:ready?"Deterministic browser fixture":"unavailable",inputResolution:224,embeddingDimensions:768,totalSources:browserAssets.length,indexedSources:ready?browserAssets.length:0,queuedSources:ready?0:browserAssets.length,failedSources:0,staleSources:0,storageBytes:ready?browserAssets.length*3080:0,detail:ready?`Semantic index ready: ${browserAssets.length} sources indexed locally.`:"Semantic model not installed. Metadata search and the catalogue remain fully available."};},
  async installSemanticModel(onProgress?:(progress:{receivedBytes:number;totalBytes:number;file:string})=>void):Promise<SemanticIndexStatus>{if(tauri())return invoke("install_semantic_model");onProgress?.({receivedBytes:815871927,totalBytes:815871927,file:"model.safetensors"});return this.semanticIndexStatus();},
  async cancelSemanticModelInstall():Promise<void>{if(tauri())return invoke("cancel_semantic_model_install");},
  async startSemanticIndex(rebuild=false):Promise<SemanticIndexStatus>{if(tauri())return invoke("start_semantic_index",{rebuild});return this.semanticIndexStatus();},
  async pauseSemanticIndex():Promise<void>{if(tauri())return invoke("pause_semantic_index");},
  async cancelSemanticIndex():Promise<void>{if(tauri())return invoke("cancel_semantic_index");},
  async prioritiseSemanticSources(itemIds:string[]):Promise<number>{if(tauri())return invoke("prioritise_semantic_sources",{itemIds});return new Set(itemIds).size;},
  async semanticSearch(request:SemanticSearchRequest):Promise<SemanticSearchResponse>{if(tauri())return invoke("semantic_search",{request});if(!semanticDemo())throw new Error("Semantic search needs the optional local model.");await new Promise(resolve=>setTimeout(resolve,120));const seen=new Set<string>();const candidates=browserAssets.filter(asset=>{const source=asset.sourceId??asset.id;if(seen.has(source))return false;if(request.filter.ratingMin!==undefined&&asset.rating<request.filter.ratingMin)return false;if(request.filter.decision&&asset.decision!==request.filter.decision)return false;if(request.filter.edited!==undefined&&Boolean(asset.hasEdits)!==request.filter.edited)return false;seen.add(source);return true;});const results:SemanticSearchResult[]=candidates.slice(0,request.limit).map((asset,index)=>({assetId:asset.id,sourceId:asset.sourceId??asset.id,score:.92-index*.06,semanticScore:.92-index*.06,metadataScore:index===0?.5:0,strength:index<2?"Strong match":"Moderate match",explanation:["Local source-image semantic similarity",...(index===0?["Keyword, title, caption or filename match"]:[])],missing:asset.missingState!==undefined&&asset.missingState!=="available"}));return{requestId:Date.now(),results,executionProvider:"Deterministic browser fixture",timings:{loadMs:0,preprocessMs:2,inferenceMs:12}};},
  async cancelSemanticSearch():Promise<void>{if(tauri())return invoke("cancel_semantic_search");},
  async findSimilarSources(itemId:string,limit=80):Promise<SemanticSearchResult[]>{if(tauri())return invoke("find_similar_sources",{itemId,limit});if(!semanticDemo())throw new Error("The selected source has not been indexed yet.");const selected=browserAssets.find(asset=>asset.id===itemId);const source=selected?.sourceId??selected?.id;return browserAssets.filter(asset=>(asset.sourceId??asset.id)!==source).slice(0,limit).map((asset,index)=>({assetId:asset.id,sourceId:asset.sourceId??asset.id,score:.96-index*.07,semanticScore:.96-index*.07,metadataScore:0,strength:index<2?"Strong match":"Moderate match",explanation:["Source-image cosine similarity"],missing:false}));},
  async discoverSemanticGroups(kind:"duplicates"|"suggestions"):Promise<DiscoveryGroup[]>{if(tauri())return invoke("discover_semantic_groups",{kind});if(!semanticDemo())return[];const picks=kind==="duplicates"?browserAssets.slice(0,2):browserAssets.slice(2,5);if(picks.length<2)return[];const id=kind==="duplicates"?"fixture-near-duplicate":"fixture-burst";return[{id,kind:kind==="duplicates"?"near_duplicate":"burst",sourceIds:picks.map(asset=>asset.sourceId??asset.id),assetIds:picks.map(asset=>asset.id),score:kind==="duplicates"?.991:.955,explanation:kind==="duplicates"?"Conservative perceptual-hash and embedding agreement":"Capture times within three seconds and strong visual similarity",dismissed:browserDismissedSuggestions.has(id)}];},
  async decideSemanticSuggestion(group:DiscoveryGroup,decision:"dismissed"|"accepted"):Promise<void>{if(tauri())return invoke("decide_semantic_suggestion",{group,decision});if(decision==="dismissed")browserDismissedSuggestions.add(group.id);},
  async proposeSmartCollectionRules(input:string):Promise<SmartCollectionProposal>{if(tauri())return invoke("propose_smart_collection_rules",{input});const match=input.match(/([1-5])\s*(?:star|\+)/i);if(!match)throw new Error("That request does not map safely to supported deterministic Smart Collection fields.");const rules:SmartRule[]=[{field:"rating",operator:/higher|at least|\+/i.test(input)?"gte":"equals",value:Number(match[1])}];const year=input.match(/\b(20\d{2})\b/)?.[1];if(year)rules.push({field:"captureDate",operator:"between",value:`${year}-01-01T00:00:00Z`,secondValue:`${year}-12-31T23:59:59Z`});const camera=input.match(/(?:taken with the|with the|taken with)\s+(.+?)(?:\s+in\s+20\d{2}|$)/i)?.[1];if(camera)rules.push({field:"camera",operator:"contains",value:camera.trim().replace(/^the\s+/i,"")});return{name:input.slice(0,64),matchMode:"all",rules,explanation:"Deterministic local parser; review and edit every rule before saving.",requiresExplicitSave:true};},
  async semanticIndexHealth():Promise<string[]>{return tauri()?invoke("semantic_index_health"):[];},
  async onSemanticInstallProgress(handler:(progress:{receivedBytes:number;totalBytes:number;file:string})=>void):Promise<()=>void>{if(!tauri())return()=>{};return listen("semantic-install-progress",event=>handler(event.payload as {receivedBytes:number;totalBytes:number;file:string}));},
  async onSemanticIndexProgress(handler:(progress:{current:number;total:number;sourceId:string;error?:string})=>void):Promise<()=>void>{if(!tauri())return()=>{};return listen("semantic-index-progress",event=>handler(event.payload as {current:number;total:number;sourceId:string;error?:string}));},
  async collections():Promise<CatalogueCollection[]>{if(tauri())return invoke("list_collections");return browserCollections.map(collection=>({...collection,count:collection.kind==="manual"?(browserCollectionItems.get(collection.id)?.size??0):browserAssets.filter(asset=>browserCollectionMatches(asset,collection.id)).length}));},
  async saveCollection(request:{id?:string;name:string;kind:"manual"|"smart";setId?:string;matchMode:"all"|"any";rules:SmartRule[]}):Promise<string>{if(tauri())return invoke("save_collection",{request});const id=request.id??crypto.randomUUID();const existing=browserCollections.findIndex(value=>value.id===id);const value:CatalogueCollection={...request,id,position:existing<0?browserCollections.length:browserCollections[existing].position,count:0};if(existing<0)browserCollections.push(value);else browserCollections[existing]=value;if(request.kind==="manual"&&!browserCollectionItems.has(id))browserCollectionItems.set(id,new Set());return id;},
  async deleteCollection(collectionId:string):Promise<void>{if(tauri())return invoke("delete_collection",{collectionId});browserCollections=browserCollections.filter(value=>value.id!==collectionId);browserCollectionItems.delete(collectionId);},
  async updateCollectionMembers(collectionId:string,itemIds:string[],add:boolean):Promise<number>{if(tauri())return invoke("update_collection_members",{collectionId,itemIds,add});const items=browserCollectionItems.get(collectionId)??new Set<string>();let changed=0;for(const id of itemIds){if(add&&!items.has(id)){items.add(id);changed++;}else if(!add&&items.delete(id))changed++;}browserCollectionItems.set(collectionId,items);return changed;},
  async collectionSets():Promise<CollectionSet[]>{return tauri()?invoke("list_collection_sets"):structuredClone(browserCollectionSets);},
  async createCollectionSet(name:string):Promise<string>{if(tauri())return invoke("create_collection_set",{name});const id=crypto.randomUUID();browserCollectionSets.push({id,name,position:browserCollectionSets.length});return id;},
  async renameCollectionSet(setId:string,name:string):Promise<void>{if(tauri())return invoke("rename_collection_set",{setId,name});const set=browserCollectionSets.find(value=>value.id===setId);if(set)set.name=name;},
  async deleteCollectionSet(setId:string):Promise<void>{if(tauri())return invoke("delete_collection_set",{setId});browserCollectionSets=browserCollectionSets.filter(value=>value.id!==setId);for(const collection of browserCollections)if(collection.setId===setId)collection.setId=undefined;},
  async stacks():Promise<StackSummary[]>{return tauri()?invoke("list_stacks"):structuredClone(browserStacks);},
  async createStack(itemIds:string[],topItemId:string):Promise<string>{if(tauri())return invoke("create_stack",{itemIds,topItemId});const id=crypto.randomUUID();const members=[topItemId,...[...new Set(itemIds)].filter(value=>value!==topItemId)];browserStacks.push({id,collapsed:true,topItemId,memberIds:members});for(const asset of browserAssets.filter(value=>members.includes(value.id))){asset.stackId=id;asset.stackCount=members.length;asset.stackCollapsed=true;asset.isStackTop=asset.id===topItemId;}return id;},
  async setStackCollapsed(stackId:string,collapsed:boolean):Promise<void>{if(tauri())return invoke("set_stack_collapsed",{stackId,collapsed});const stack=browserStacks.find(value=>value.id===stackId);if(stack)stack.collapsed=collapsed;for(const asset of browserAssets.filter(value=>value.stackId===stackId))asset.stackCollapsed=collapsed;},
  async setStackTop(stackId:string,itemId:string):Promise<void>{if(tauri())return invoke("set_stack_top",{stackId,itemId});const stack=browserStacks.find(value=>value.id===stackId);if(stack)stack.topItemId=itemId;for(const asset of browserAssets.filter(value=>value.stackId===stackId))asset.isStackTop=asset.id===itemId;},
  async addToStack(stackId:string,itemIds:string[]):Promise<number>{if(tauri())return invoke("add_to_stack",{stackId,itemIds});const stack=browserStacks.find(value=>value.id===stackId);if(!stack)throw new Error("That stack no longer exists.");let changed=0;for(const itemId of [...new Set(itemIds)]){const asset=browserAssets.find(value=>value.id===itemId);if(asset&&!asset.stackId&&!stack.memberIds.includes(itemId)){stack.memberIds.push(itemId);asset.stackId=stackId;asset.stackCollapsed=stack.collapsed;asset.isStackTop=false;changed++;}}for(const asset of browserAssets.filter(value=>value.stackId===stackId))asset.stackCount=stack.memberIds.length;return changed;},
  async removeFromStack(itemId:string):Promise<void>{if(tauri())return invoke("remove_from_stack",{itemId});const asset=browserAssets.find(value=>value.id===itemId);if(!asset?.stackId)return;const stack=browserStacks.find(value=>value.id===asset.stackId);if(!stack)return;stack.memberIds=stack.memberIds.filter(value=>value!==itemId);asset.stackId=undefined;asset.stackCount=0;asset.stackCollapsed=false;asset.isStackTop=false;if(stack.memberIds.length<2){await this.unstack(stack.id);return;}if(stack.topItemId===itemId){stack.topItemId=stack.memberIds[0];const top=browserAssets.find(value=>value.id===stack.topItemId);if(top)top.isStackTop=true;}for(const member of browserAssets.filter(value=>value.stackId===stack.id))member.stackCount=stack.memberIds.length;},
  async unstack(stackId:string):Promise<void>{if(tauri())return invoke("unstack",{stackId});browserStacks=browserStacks.filter(value=>value.id!==stackId);for(const asset of browserAssets.filter(value=>value.stackId===stackId)){asset.stackId=undefined;asset.stackCount=0;asset.stackCollapsed=false;asset.isStackTop=false;}},
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
    if (asset && asset.decision !== decision) { history.push({ kind: "decision", id, decision: asset.decision }); redoHistory.length = 0; asset.decision = decision; }
  },
  async undo(): Promise<boolean> {
    if (tauri()) return invoke("undo_last_action");
    const prior = history.pop(); if (!prior) return false;
    const inverse = applyBrowserHistory(prior); if (!inverse) return false; redoHistory.push(inverse); return true;
  },
  async redo(): Promise<boolean> { if (tauri()) return invoke("redo_last_action"); const next=redoHistory.pop();if(!next)return false;const inverse=applyBrowserHistory(next);if(!inverse)return false;history.push(inverse);return true; },
  async batchUpdateMetadata(assetIds: string[], patch: LibraryMetadataPatch): Promise<BatchSummary> {
    if (tauri()) return invoke("batch_update_metadata", { request: { assetIds, patch } });
    history.push(snapshotBrowserBatch(assetIds)); redoHistory.length=0; let changed=0;
    for (const asset of browserAssets.filter(value=>assetIds.includes(value.id))) { const before=JSON.stringify(asset); if ("title" in patch) asset.title=patch.title??undefined;if("caption" in patch)asset.caption=patch.caption??undefined;if("copyright" in patch)asset.copyright=patch.copyright??undefined;if("creator" in patch)asset.creator=patch.creator??undefined;if(patch.rating!==undefined)asset.rating=patch.rating;if(patch.decision)asset.decision=patch.decision;if(patch.replaceKeywords)asset.tags=[...new Set(patch.replaceKeywords)];if(patch.addKeywords)asset.tags=[...new Set([...asset.tags,...patch.addKeywords])];if(patch.removeKeywords){const remove=new Set(patch.removeKeywords.map(value=>value.toLowerCase()));asset.tags=asset.tags.filter(value=>!remove.has(value.toLowerCase()));}if(JSON.stringify(asset)!==before)changed+=1; }
    return { requested: assetIds.length, changed, failed: 0, cancelled: 0 };
  },
  async syncDevelopSettings(sourceId: string, targetIds: string[], categories: SyncCategory[]): Promise<BatchSummary> {
    if (tauri()) return invoke("sync_develop_settings", { request: { sourceId, targetIds, categories } });
    const ids=[...new Set(targetIds.filter(id=>id!==sourceId))];history.push(snapshotBrowserBatch(ids));redoHistory.length=0;const source=await this.getDevelopRecipe(sourceId);for(const id of ids){const recipe=applyRecipeCategories(await this.getDevelopRecipe(id),source,categories);browserDevelopRecipes.set(id,recipe);const asset=browserAssets.find(value=>value.id===id);if(asset)asset.hasEdits=true;}return {requested:ids.length,changed:ids.length,failed:0,cancelled:0};
  },
  async applyPresetToSelection(assetIds:string[],presetId:string):Promise<BatchSummary>{
    if(tauri())return invoke("apply_preset_to_selection",{request:{assetIds,presetId}});const preset=[...browserBuiltInDevelopPresets,...browserDevelopPresets].find(value=>value.id===presetId);if(!preset)throw new Error("That preset no longer exists.");const ids=[...new Set(assetIds)];history.push(snapshotBrowserBatch(ids));redoHistory.length=0;const source:DevelopRecipe={schemaVersion:2,settings:preset.settings,masks:[]};for(const id of ids){const recipe=applyRecipeCategories(await this.getDevelopRecipe(id),source,preset.categories as SyncCategory[]);browserDevelopRecipes.set(id,recipe);const asset=browserAssets.find(value=>value.id===id);if(asset)asset.hasEdits=true;}return{requested:ids.length,changed:ids.length,failed:0,cancelled:0};
  },
  async previewBatchAuto(assetIds:string[]):Promise<BatchAutoSummary>{if(tauri())return invoke("preview_batch_auto",{assetIds});return{requested:assetIds.length,changed:0,failed:0,cancelled:0,analyzable:assetIds.length,lowConfidenceWhiteBalance:0,skipped:0};},
  async applyBatchAuto(assetIds:string[]):Promise<BatchAutoSummary>{if(tauri())return invoke("apply_batch_auto",{assetIds});const ids=[...new Set(assetIds)];history.push(snapshotBrowserBatch(ids));redoHistory.length=0;for(const id of ids){const asset=browserAssets.find(value=>value.id===id);if(!asset)continue;const recipe=await this.getDevelopRecipe(id);const seed=[...asset.filename].reduce((sum,value)=>sum+value.charCodeAt(0),0);recipe.settings={...recipe.settings,exposure:((seed%9)-4)/20,contrast:6+(seed%7),highlights:-18-(seed%13),shadows:12+(seed%11),clarity:3+(seed%5),colourBoost:5+(seed%8)};browserDevelopRecipes.set(id,recipe);asset.hasEdits=true;}return{requested:ids.length,changed:ids.length,failed:0,cancelled:0,analyzable:ids.length,lowConfidenceWhiteBalance:0,skipped:0};},
  async cancelLibraryBatch():Promise<void>{if(tauri())return invoke("cancel_library_batch");},
  async onBatchProgress(callback:(progress:{kind:string;current:number;total:number;failed:number})=>void):Promise<()=>void>{if(!tauri())return()=>{};return listen("batch-progress",event=>callback(event.payload as {kind:string;current:number;total:number;failed:number}));},
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
    return tauri() ? invoke("get_develop_recipe", { assetId }) : browserDevelopRecipes.get(assetId) ?? { schemaVersion: 2, settings: { ...neutralAdjustments }, masks: [] };
  },
  async saveDevelopRecipe(assetId: string, recipe: DevelopRecipe): Promise<boolean> {
    if (tauri()) return invoke("save_develop_recipe", { assetId, recipe });
    const edited = JSON.stringify(recipe.settings) !== JSON.stringify(neutralAdjustments) || recipe.masks.length > 0;
    if (edited) browserDevelopRecipes.set(assetId, structuredClone(recipe)); else browserDevelopRecipes.delete(assetId);
    const asset = browserAssets.find((item) => item.id === assetId); if (asset) asset.hasEdits = edited;
    return edited;
  },
  async previewDevelopRecipe(assetId: string, recipe: DevelopRecipe): Promise<string> {
    return tauri() ? convertFileSrc(await invoke<string>("preview_develop_recipe", { assetId, recipe })) : "";
  },
  async intelligentMaskHealth(): Promise<IntelligentMaskHealth> {
    if (tauri()) return invoke("get_intelligent_mask_health");
    if (intelligentMaskDemo()) return { available: true, installed: true, runtimeAvailable: true, loaded: true, busy: false, provider: "Deterministic browser UI fixture", providerVersion: "1.0.0", model: "fixture/semantic-mask", modelRevision: "1", licence: "generated fixture", source: "local test fixture", approximateBytes: 0, storagePath: "Browser memory only", executionProvider: "mock CPU", detail: "Browser UI fixture ready." };
    return { available: false, installed: false, runtimeAvailable: false, loaded: false, busy: false, provider: "Keepframe BEiT semantic segmentation", providerVersion: "1.0.0", model: "microsoft/beit-base-finetuned-ade-640-640", modelRevision: "a8b6f5ef4acb2ea55d882989deaa02d39401e2b2", licence: "Apache-2.0", source: "https://huggingface.co/microsoft/beit-base-finetuned-ade-640-640", approximateBytes: 899902905, storagePath: "D:\\AI Models\\Keepframe\\segmentation\\beit-base-ade20k-640", executionProvider: "unavailable", detail: "Intelligent masking model not installed. Manual masks remain available." };
  },
  async installIntelligentMaskModel(onProgress: (progress: { receivedBytes: number; totalBytes: number; file: string }) => void): Promise<IntelligentMaskHealth> {
    if (!tauri()) return { available: false, installed: false, runtimeAvailable: false, loaded: false, busy: false, provider: "Keepframe BEiT semantic segmentation", providerVersion: "1.0.0", model: "microsoft/beit-base-finetuned-ade-640-640", modelRevision: "a8b6f5ef4acb2ea55d882989deaa02d39401e2b2", licence: "Apache-2.0", source: "https://huggingface.co/microsoft/beit-base-finetuned-ade-640-640", approximateBytes: 899902905, storagePath: "D:\\AI Models\\Keepframe\\segmentation\\beit-base-ade20k-640", executionProvider: "unavailable", detail: "Intelligent masking is installed only in the desktop app. Manual masks remain available." };
    const unlisten = await listen<{ receivedBytes: number; totalBytes: number; file: string }>("intelligent-mask-install-progress", (event) => onProgress(event.payload));
    try { return await invoke("install_intelligent_mask_model"); } finally { unlisten(); }
  },
  async cancelIntelligentMaskInstall(): Promise<void> { if (tauri()) return invoke("cancel_intelligent_mask_install"); },
  async proposeIntelligentMask(assetId: string, category: IntelligentMaskCategory): Promise<IntelligentMaskProposal> {
    if (tauri()) return invoke("propose_intelligent_mask", { assetId, category });
    if (intelligentMaskDemo()) { await new Promise((resolve) => window.setTimeout(resolve, 1000)); return { requestId: Date.now(), category, confidence: .88, coverageFraction: .34, elapsedMs: 1000, timings: { loadMs: 0, preprocessMs: 3, inferenceMs: 12, postprocessMs: 4 }, mask: { id: crypto.randomUUID(), name: category[0].toUpperCase()+category.slice(1), enabled: true, inverted: false, opacity: 1, feather: 0, geometry: { kind: "semantic", width: 32, height: 24, coveragePng: browserMaskCoverage, checksum: "0".repeat(64), provenance: { provider: "Deterministic browser UI fixture", providerVersion: "1.0.0", model: "fixture/semantic-mask", modelRevision: "1", modelSha256: "0".repeat(64), category, executionProvider: "mock CPU" }, refinements: [] }, adjustments: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, lightBalance: 0, tint: 0, saturation: 0, clarity: 0, dehaze: 0, texture: 0 } } }; }
    throw new Error("Intelligent masking model not installed.");
  },
  async cancelIntelligentMask(): Promise<void> { if (tauri()) return invoke("cancel_intelligent_mask"); },
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
  async chooseExportFolder(): Promise<string | null> {
    if (!tauri()) return "D:\\Exports";
    const path = await open({ directory: true, multiple: false, title: "Choose the export folder" });
    return typeof path === "string" ? path : null;
  },
  async startExportBatch(assetIds: string[], destination: string, config: ExportConfig): Promise<ExportBatchReport> {
    if (tauri()) return invoke("start_export_batch", { request: { assetIds, destination, config } });
    await new Promise((resolve) => window.setTimeout(resolve, 120));
    return { batchId: crypto.randomUUID(), destination, requested: assetIds.length, complete: assetIds.length, failed: 0, cancelled: 0, skipped: 0, elapsedMs: 120, peakWorkingBytes: 18_000_000, concurrency: 1, items: assetIds.map((assetId) => { const asset=browserAssets.find((value)=>value.id===assetId); return { assetId, filename: asset?.filename ?? "Photograph", state: "complete", path: `${destination}\\${asset?.filename ?? assetId}.jpg`, width: asset?.width, height: asset?.height, bytes: 2_400_000, elapsedMs: 120 }; }) };
  },
  async cancelExportBatch(): Promise<void> { if (tauri()) await invoke("cancel_export_batch"); },
  async onExportProgress(handler: (progress: ExportProgress) => void): Promise<() => void> { if (!tauri()) return () => undefined; return listen<ExportProgress>("export-progress", (event) => handler(event.payload)); },
  async exportPresets(): Promise<ExportPreset[]> {
    if (tauri()) return invoke("list_export_presets");
    return [{ id: "web-jpeg", name: "Web JPEG", builtIn: true, config: defaultExportConfig({ resizeMode: "longedge", width: 2400, ppi: 96 }) }, { id: "full-jpeg", name: "Full-size JPEG", builtIn: true, config: defaultExportConfig() }, { id: "archive-tiff", name: "Archive TIFF", builtIn: true, config: defaultExportConfig({ format: "tiff", sharpening: "none" }) }, ...browserExportPresets];
  },
  async saveExportPreset(preset: ExportPreset): Promise<ExportPreset> { if (tauri()) return invoke("save_export_preset", { preset }); const next={...preset,builtIn:false};browserExportPresets=browserExportPresets.filter((value)=>value.id!==next.id).concat(next);return next; },
  async deleteExportPreset(id: string): Promise<void> { if (tauri()) await invoke("delete_export_preset", { id }); else browserExportPresets=browserExportPresets.filter((value)=>value.id!==id); },
  async exportExportPreset(id: string): Promise<void> { if (!tauri()) return; const path=await save({ title:"Export output preset",defaultPath:"keepframe-output.keepframe-export-preset",filters:[{name:"Keepframe output preset",extensions:["keepframe-export-preset"]}]});if(typeof path==="string")await invoke("export_export_preset",{id,path}); },
  async importExportPreset(): Promise<ExportPreset | null> { if (!tauri()) return null;const path=await open({multiple:false,directory:false,title:"Import output preset",filters:[{name:"Keepframe output preset",extensions:["keepframe-export-preset"]}]});return typeof path==="string"?invoke("import_export_preset",{path}):null; },
  async revealExport(destination: string): Promise<void> { if (tauri()) await revealItemInDir(destination); },
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

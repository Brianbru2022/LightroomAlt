import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Asset, AssetFilter, AssetPage, AssetVersion, BatchJob, Decision, DeleteSummary, EditIntent, EditRecipe, ImportSummary, LibraryStatus, PromptSet, ServiceHealth } from "../types";
import { demoAssets, demoJobs, demoStatus, makeRecipe, renderPrompts } from "./demo";

const tauri = () => "__TAURI_INTERNALS__" in window;
let browserAssets = structuredClone(demoAssets);
let browserJobs = structuredClone(demoJobs);
type BrowserHistory =
  | { kind: "decision"; id: string; decision: Decision }
  | { kind: "tags"; id: string; tags: string[] }
  | { kind: "location"; id: string; latitude?: number; longitude?: number };
const history: BrowserHistory[] = [];

const withAssetUrls = (asset: Asset): Asset => {
  const normalised = { ...asset, latitude: asset.latitude ?? undefined, longitude: asset.longitude ?? undefined };
  if (!tauri()) return normalised;
  const toUrl = (value: string) => value.startsWith("data:") || value.startsWith("http") ? value : convertFileSrc(value);
  return {
    ...normalised,
    previewUrl: toUrl(normalised.previewUrl),
    thumbnailUrl: toUrl(normalised.thumbnailUrl),
    preferredVersionUrl: normalised.preferredVersionUrl ? toUrl(normalised.preferredVersionUrl) : undefined,
  };
};
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
  async serviceHealth(): Promise<ServiceHealth> {
    return tauri() ? invoke("get_service_health") : { localAiAvailable: true, serviceReachable: true, localAiBusy: false, localAiModel: "Qwen-Image-Edit", localAiDetail: "The local Qwen image editor is ready.", analysisModelInstalled: false };
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
  async importPhotos(paths: string[]): Promise<ImportSummary> {
    if (tauri()) return invoke("import_photos", { paths });
    return { importId: crypto.randomUUID(), discovered: paths.length ? 6 : 0, imported: paths.length ? 6 : 0, duplicates: 0, unsupported: 0, failed: 0 };
  },
  async assets(filter: AssetFilter, offset = 0, limit = 240): Promise<AssetPage> {
    if (tauri()) {
      const page = await invoke<AssetPage>("query_assets", { filter, offset, limit });
      return { ...page, items: page.items.map(withAssetUrls) };
    }
    const query = filter.search.toLocaleLowerCase();
    const matching = browserAssets.filter((asset) => {
      if (filter.decision !== "all" && asset.decision !== filter.decision) return false;
      if (filter.year && new Date(asset.capturedAt).getFullYear() !== filter.year) return false;
      if (filter.tag && !asset.tags.includes(filter.tag)) return false;
      return !query || `${asset.filename} ${asset.camera ?? ""} ${asset.tags.join(" ")}`.toLocaleLowerCase().includes(query);
    });
    const items = matching.slice(offset, offset + limit);
    return { items, total: matching.length, offset, limit, hasMore: offset + items.length < matching.length };
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
  async deleteDiscarded(assetIds: string[]): Promise<DeleteSummary> {
    if (tauri()) return invoke("delete_discarded_assets", { assetIds });
    const selected = new Set(assetIds);
    const before = browserAssets.length;
    browserAssets = browserAssets.filter((asset) => !selected.has(asset.id) || asset.decision !== "discard");
    return { deleted: before - browserAssets.length, failed: 0 };
  },
  async updateTags(id: string, tags: string[]): Promise<void> {
    if (tauri()) return invoke("update_tags", { assetId: id, tags });
    const asset = browserAssets.find((item) => item.id === id); if (asset) { history.push({ kind: "tags", id, tags: [...asset.tags] }); asset.tags = tags; }
  },
  async updateLocation(id: string, latitude: number, longitude: number): Promise<void> {
    if (tauri()) return invoke("update_location", { assetId: id, latitude, longitude });
    const asset = browserAssets.find((item) => item.id === id); if (asset) { history.push({ kind: "location", id, latitude: asset.latitude, longitude: asset.longitude }); asset.latitude = latitude; asset.longitude = longitude; }
  },
  async analyse(asset: Asset, intent: EditIntent, commonBrief?: string): Promise<EditRecipe> {
    if (tauri()) return invoke("create_edit_recipe", { assetId: asset.id, intent, commonBrief });
    return { ...makeRecipe(asset, intent), commonBrief };
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
  async setPreferredVersion(assetId: string, versionId?: string): Promise<void> {
    if (tauri()) return invoke("set_preferred_version", { assetId, versionId: versionId ?? null });
  },
  async chooseReplacement(asset: Asset): Promise<AssetVersion | null> {
    if (!tauri()) return null;
    const path = await open({ multiple: false, directory: false, title: "Choose a replacement image", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "tif", "tiff"] }] });
    if (typeof path !== "string") return null;
    return withVersionUrl(await invoke<AssetVersion>("import_replacement", { assetId: asset.id, path }));
  },
  async importReturned(assetId: string, provider: "chatgpt" | "gemini", prompt: string): Promise<BatchJob | null> {
    if (!tauri()) {
      const asset = browserAssets.find((item) => item.id === assetId); if (!asset) return null;
      const job: BatchJob = { id: crypto.randomUUID(), batchId: crypto.randomUUID(), assetId, assetName: asset.filename, state: "succeeded", prompt, attempts: [], outputUrl: asset.previewUrl };
      browserJobs = [job, ...browserJobs]; return job;
    }
    const path = await open({ multiple: false, directory: false, title: "Choose the returned edited image", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "tif", "tiff"] }] });
    if (typeof path !== "string") return null;
    return invoke("import_returned_edit", { assetId, path, provider, prompt });
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

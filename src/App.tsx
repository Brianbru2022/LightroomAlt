import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { SetupScreen } from "./components/SetupScreen";
import { ImportDialog } from "./components/ImportDialog";
import { TagDialog } from "./components/TagDialog";
import { Topbar } from "./components/Topbar";
import { api } from "./lib/bridge";
import { clearSelection, emptySelection, navigateSelection, selectAll, selectId, visibleSelectionCount, type SelectionState } from "./lib/selection";
import { decisionLabel } from "./lib/format";
import type { Asset, AssetFilter, BasicAdjustments, BatchJob, Decision, EditIntent, EditRecipe, ImportOptions, LibraryMetadataPatch, LibraryStatus, MapAsset, MapBounds, ServiceHealth, SyncCategory, ViewName } from "./types";
import { LibraryView } from "./views/LibraryView";
import { LibraryProductivityView, type LibraryLayout } from "./views/LibraryProductivityView";
import { MapView } from "./views/MapView";
import { TriageView } from "./views/TriageView";
import { DevelopView } from "./views/DevelopView";
import { ExportDialog } from "./components/ExportDialog";
import { WorkshopView } from "./views/WorkshopView";
import { SettingsView } from "./views/SettingsView";

const PAGE_SIZE = 240;
const emptyStatus: LibraryStatus = { configured: false, counts: { total: 0, keep: 0, undecided: 0, discard: 0 } };
const emptyHealth: ServiceHealth = { localAiAvailable: false, serviceReachable: false, localAiBusy: false, localAiDetail: "No local image service is responding.", localAiState: "service_not_running", localAiUrl: "http://127.0.0.1:7868", analysisModelInstalled: false, analysisAvailable: false, analysisDetail: "The image-analysis worker is unavailable. Recipes will use the controls-only fallback." };
type OperationProgress = { kind: "Import" | "Trash" | "Cache" | "Batch Auto"; current: number; total: number; importId?: string; file?: string; error?: string };

export function App() {
  const [status, setStatus] = useState<LibraryStatus>(emptyStatus);
  const [health, setHealth] = useState<ServiceHealth>(emptyHealth);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [visibleTotal, setVisibleTotal] = useState(0);
  const [hasMoreAssets, setHasMoreAssets] = useState(false);
  const [loadingAssets, setLoadingAssets] = useState(true);
  const [mapAssets, setMapAssets] = useState<MapAsset[]>([]);
  const [mapLoading, setMapLoading] = useState(false);
  const [spatialBounds, setSpatialBounds] = useState<MapBounds | undefined>();
  const [jobs, setJobs] = useState<BatchJob[]>([]);
  const [view, setView] = useState<ViewName>("library");
  const [selection, setSelection] = useState<SelectionState>(emptySelection);
  const [resultIds, setResultIds] = useState<string[]>([]);
  const [libraryLayout, setLibraryLayout] = useState<LibraryLayout>("grid");
  const [autoAdvance, setAutoAdvanceState] = useState(()=>localStorage.getItem("keepframe.autoAdvance")==="true");
  const [filter, setFilter] = useState<AssetFilter>({ decision: "all", search: "" });
  const [busy, setBusy] = useState(true);
  const [tagAsset, setTagAsset] = useState<Asset | null>(null);
  const [exportAsset, setExportAsset] = useState<Asset | null>(null);
  const [exportSelection, setExportSelection] = useState<string[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [importPaths, setImportPaths] = useState<string[]>([]);
  const [operationProgress, setOperationProgress] = useState<OperationProgress | null>(null);
  const requestId = useRef(0);
  const deferredSearch = useDeferredValue(filter.search);
  const effectiveFilter = useMemo<AssetFilter>(() => ({ ...filter, search: deferredSearch, trashed: view === "trash" }), [deferredSearch, filter, view]);
  const knownTags = useMemo(() => [...new Set(assets.flatMap((asset) => asset.tags))].sort((left, right) => left.localeCompare(right)), [assets]);
  const selectedId=selection.activeId;
  const selected = useMemo(() => assets.find((asset) => asset.id === selectedId) ?? null, [assets, selectedId]);
  const selectedAssetId = selected?.id;
  const visibleSelectedCount = useMemo(() => visibleSelectionCount(selection.selectedIds, resultIds), [resultIds, selection.selectedIds]);
  const setSingleSelection=(id:string|null)=>setSelection(id?{activeId:id,anchorId:id,selectedIds:new Set([id])}:clearSelection());
  const setAutoAdvance=(value:boolean)=>{setAutoAdvanceState(value);localStorage.setItem("keepframe.autoAdvance",String(value));};

  const loadFirstPage = useCallback(async () => {
    const request = ++requestId.current;
    setLoadingAssets(true);
    try {
      const [page,ids] = await Promise.all([api.assets(effectiveFilter, 0, PAGE_SIZE),api.assetIds(effectiveFilter)]);
      if (request !== requestId.current) return;
      setAssets(page.items);
      setVisibleTotal(page.total);
      setHasMoreAssets(page.hasMore);
      setResultIds(ids);
      setSelection(current => current.activeId ? current : (page.items[0] ? selectId(current,page.items[0].id,ids) : current));
    } catch (error) {
      if (request === requestId.current) setNotice(`Could not load photographs: ${String(error)}`);
    } finally {
      if (request === requestId.current) setLoadingAssets(false);
    }
  }, [effectiveFilter]);

  const loadMoreAssets = useCallback(async () => {
    if (loadingAssets || !hasMoreAssets) return;
    const request = requestId.current;
    const offset = assets.length;
    setLoadingAssets(true);
    try {
      const page = await api.assets(effectiveFilter, offset, PAGE_SIZE);
      if (request !== requestId.current) return;
      setAssets((current) => {
        const existing = new Set(current.map((asset) => asset.id));
        return [...current, ...page.items.filter((asset) => !existing.has(asset.id))];
      });
      setVisibleTotal(page.total);
      setHasMoreAssets(page.hasMore);
    } catch (error) {
      if (request === requestId.current) setNotice(`Could not load more photographs: ${String(error)}`);
    } finally {
      if (request === requestId.current) setLoadingAssets(false);
    }
  }, [assets.length, effectiveFilter, hasMoreAssets, loadingAssets]);

  const refreshCatalogue = useCallback(async () => {
    const [currentStatus] = await Promise.all([api.status(), loadFirstPage()]);
    setStatus(currentStatus);
  }, [loadFirstPage]);

  const loadMapAssets = useCallback(async () => {
    setMapLoading(true);
    try { setMapAssets(await api.mapAssets(effectiveFilter, spatialBounds)); }
    catch (error) { setNotice(`Could not load map markers: ${String(error)}`); }
    finally { setMapLoading(false); }
  }, [effectiveFilter, spatialBounds]);

  const undoCatalogue = useCallback(async () => {
    try {
      const changed = await api.undo();
      setNotice(changed ? "Latest catalogue change undone." : "Nothing to undo.");
      await refreshCatalogue();
    } catch (error) {
      setNotice(`Undo failed: ${String(error)}`);
    }
  }, [refreshCatalogue]);
  const redoCatalogue = useCallback(async () => {
    try { const changed=await api.redo();setNotice(changed?"Latest catalogue change redone.":"Nothing to redo.");await refreshCatalogue(); }
    catch(error){setNotice(`Redo failed: ${String(error)}`);}
  },[refreshCatalogue]);
  const applySelectionMetadata = useCallback(async (patch:LibraryMetadataPatch) => {
    const ids=[...selection.selectedIds];if(!ids.length&&selection.activeId)ids.push(selection.activeId);if(!ids.length)return {requested:0,changed:0,failed:0,cancelled:0};
    const summary=await api.batchUpdateMetadata(ids,patch);setNotice(`${summary.changed} of ${summary.requested} photographs updated as one undoable action.`);await refreshCatalogue();return summary;
  },[refreshCatalogue,selection]);
  const reviewOne=useCallback(async(id:string,patch:LibraryMetadataPatch)=>{await api.batchUpdateMetadata([id],patch);await refreshCatalogue();},[refreshCatalogue]);

  useEffect(() => {
    let active = true;
    api.resetBrowserSession();
    api.status()
      .then(async (currentStatus) => {
        if (!active) return;
        setStatus(currentStatus);
        if (currentStatus.configured) setJobs(await api.jobs());
      })
      .catch((error) => active && setNotice(`Could not open the catalogue: ${String(error)}`))
      .finally(() => active && setBusy(false));
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (status.configured && view === "map") void loadMapAssets();
  }, [loadMapAssets, status.configured, view]);
  useEffect(() => {
    if (status.configured && status.recoveryNotice) setNotice(status.recoveryNotice);
  }, [status.configured, status.recoveryNotice]);
  useEffect(() => {
    if (!api.isNative()) return;
    let active = true;
    const unlisteners: Array<() => void> = [];
    void import("@tauri-apps/api/event").then(async ({ listen }) => {
      const registrations = await Promise.all([
        listen<Record<string, unknown>>("import-progress", ({ payload }) => active && setOperationProgress({ kind: "Import", current: Number(payload.current ?? 0), total: Number(payload.total ?? 0), importId: String(payload.importId ?? ""), file: String(payload.file ?? ""), error: payload.error ? String(payload.error) : undefined })),
        listen<Record<string, unknown>>("trash-progress", ({ payload }) => active && setOperationProgress({ kind: "Trash", current: Number(payload.current ?? 0), total: Number(payload.total ?? 0), error: payload.error ? String(payload.error) : undefined })),
        listen<Record<string, unknown>>("cache-progress", ({ payload }) => active && setOperationProgress({ kind: "Cache", current: Number(payload.current ?? 0), total: Number(payload.total ?? 0), error: payload.error ? String(payload.error) : undefined })),
        listen<Record<string, unknown>>("batch-progress", ({ payload }) => active && setOperationProgress({ kind: "Batch Auto", current: Number(payload.current ?? 0), total: Number(payload.total ?? 0), error: Number(payload.failed??0)>0?`${payload.failed} failed so far`:undefined })),
      ]);
      if (active) unlisteners.push(...registrations); else registrations.forEach((unlisten) => unlisten());
    });
    return () => { active = false; unlisteners.forEach((unlisten) => unlisten()); };
  }, []);
  useEffect(() => { if (status.configured) void loadFirstPage(); }, [loadFirstPage, status.configured]);
  useEffect(() => {
    let active = true;
    const check = () => api.serviceHealth().then((value) => { if (active) setHealth(value); }).catch(() => { if (active) setHealth(emptyHealth); });
    void check();
    const timer = window.setInterval(check, 30_000);
    return () => { active = false; window.clearInterval(timer); };
  }, []);
  useEffect(() => {
    if (!jobs.some((job) => ["analysing", "queued", "running"].includes(job.state))) return;
    const timer = window.setInterval(() => { api.jobs().then(setJobs).catch((error) => setNotice(String(error))); }, 1500);
    return () => window.clearInterval(timer);
  }, [jobs]);
  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      const target = event.target;
      const isEditing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
      const isOtherControl = target instanceof HTMLElement && target.closest("button, [role='button']") && !target.closest(".asset-card");
      if (event.ctrlKey && key === "k") {
        event.preventDefault();
        document.querySelector<HTMLInputElement>(".search-field input")?.focus();
        return;
      }
      if (!isEditing && event.ctrlKey && key === "z") {
        event.preventDefault();
        void (event.shiftKey?redoCatalogue():undoCatalogue());
        return;
      }
      if (busy || isEditing || isOtherControl || event.altKey || event.metaKey || view !== "library") return;
      if((event.ctrlKey&&event.shiftKey&&key==="a")||(!event.ctrlKey&&event.key==="Escape")){event.preventDefault();setSelection(clearSelection());return;}
      if(event.ctrlKey&&key==="a"){event.preventDefault();setSelection(selectAll(resultIds,selection.activeId));return;}
      if(event.ctrlKey)return;
      if(key==="g"){setLibraryLayout("grid");return;}if(key==="c"){setLibraryLayout("compare");return;}if(key==="n"){setLibraryLayout("survey");return;}if(key===" "){event.preventDefault();setLibraryLayout("loupe");return;}if(key==="d"||event.key==="Enter"){if(selected){event.preventDefault();setView("develop");}return;}
      if(!selectedAssetId)return;

      if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
        const cards = Array.from(document.querySelectorAll<HTMLElement>(".asset-card[data-asset-id]"));
        const currentIndex = Math.max(0, cards.findIndex((card) => card.dataset.assetId === selectedAssetId));
        const current = cards[currentIndex];
        if (!current) return;
        let next: HTMLElement | undefined;
        if (event.key === "ArrowLeft") next = cards[currentIndex - 1];
        if (event.key === "ArrowRight") next = cards[currentIndex + 1];
        if (event.key === "ArrowUp" || event.key === "ArrowDown") {
          const currentRect = current.getBoundingClientRect();
          const direction = event.key === "ArrowUp" ? -1 : 1;
          let nearest: HTMLElement | undefined;
          let nearestDistance = Number.POSITIVE_INFINITY;
          for (const card of cards) {
            const rect = card.getBoundingClientRect();
            const isCandidate = direction < 0 ? rect.bottom <= currentRect.top + 1 : rect.top >= currentRect.bottom - 1;
            if (!isCandidate) continue;
            const distance = Math.abs(rect.top - currentRect.top) * 1000 + Math.abs((rect.left + rect.right) / 2 - (currentRect.left + currentRect.right) / 2);
            if (distance < nearestDistance) {
              nearest = card;
              nearestDistance = distance;
            }
          }
          next = nearest;
          if (!next && currentRect.width === 0) next = cards[currentIndex + direction];
        }
        const nextId = next?.dataset.assetId;
        if (!next || !nextId) return;
        event.preventDefault();
        setSingleSelection(nextId);
        next.focus({ preventScroll: true });
        next.scrollIntoView?.({ block: "nearest", inline: "nearest" });
        return;
      }

      if (/^[0-5]$/.test(key) || ["p","x","u","m"].includes(key)) {
        event.preventDefault();
        const patch:LibraryMetadataPatch=/^[0-5]$/.test(key)?{rating:Number(key)}:{decision:key==="x"?"discard":key==="u"?"undecided":"keep"};
        void applySelectionMetadata(patch).then(()=>{if(autoAdvance&&selection.selectedIds.size<=1)setSelection(current=>navigateSelection(current,resultIds,1));});
      }
    };
    window.addEventListener("keydown", shortcut); return () => window.removeEventListener("keydown", shortcut);
  }, [applySelectionMetadata, autoAdvance, busy, redoCatalogue, resultIds, selected, selectedAssetId, selection, undoCatalogue, view]);

  useEffect(() => {
    const index = assets.findIndex((asset) => asset.id === selectedAssetId);
    if (index >= assets.length - 20 && hasMoreAssets) void loadMoreAssets();
  }, [assets, hasMoreAssets, loadMoreAssets, selectedAssetId]);

  const decide = async (id: string, decision: Decision) => {
    const index = assets.findIndex((asset) => asset.id === id);
    const current = assets[index];
    const next = assets[index + 1];
    const changed = current?.decision !== decision;
    if (next) setSingleSelection(next.id);
    try {
      await api.setDecision(id, decision);
      setNotice(!changed
        ? `${current.filename} was already marked ${decisionLabel[decision]}.`
        : `${current?.filename ?? "Photograph"} marked ${decisionLabel[decision]}. Ctrl+Z to undo.`);
      await refreshCatalogue();
    } catch (error) {
      setSingleSelection(id);
      setNotice(`Could not update decision: ${String(error)}`);
    }
  };
  const importPhotos = async () => {
    const paths = await api.chooseImport();
    if (paths.length) setImportPaths(paths);
  };
  const startImport = async (options: ImportOptions) => {
    const paths = importPaths;
    setImportPaths([]);
    setBusy(true);
    try {
      const result = await api.importPhotos(paths, options);
      setNotice(`${result.copied} copied; ${result.moved} moved; ${result.sourceRetained} sources retained; ${result.duplicates} exact duplicates; ${result.failed} failed.`);
      await refreshCatalogue();
    } catch (error) { setNotice(`Import could not complete: ${String(error)}`); } finally { setBusy(false); setOperationProgress(null); }
  };
  const moveAllDiscarded = async () => {
    if (!visibleTotal) return;
    const scope = filter.search ? " matching the current search" : "";
    if (!window.confirm(`Move all ${visibleTotal} discarded ${visibleTotal === 1 ? "photograph" : "photographs"}${scope} to Keepframe Trash?\n\nThey remain catalogued and can be restored until you separately empty Trash.`)) return;
    setBusy(true);
    try {
      const ids = await api.assetIds({ ...effectiveFilter, decision: "discard" });
      const result = await api.moveToTrash(ids);
      setNotice(`${result.affected} discarded ${result.affected === 1 ? "photograph" : "photographs"} moved to Keepframe Trash${result.failed ? `; ${result.failed} need attention` : ""}.`);
      await refreshCatalogue();
    } catch (error) {
      setNotice(`Could not move discarded photographs to Trash: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };
  const restoreAllTrash = async () => {
    const ids = await api.assetIds({ ...effectiveFilter, decision: "all", trashed: true });
    if (!ids.length) return;
    setBusy(true);
    try {
      const result = await api.restoreFromTrash(ids);
      setNotice(`${result.affected} ${result.affected === 1 ? "photograph" : "photographs"} restored${result.failed ? `; ${result.failed} need attention` : ""}.`);
      await refreshCatalogue();
    } catch (error) { setNotice(`Restore failed: ${String(error)}`); } finally { setBusy(false); }
  };
  const emptyAllTrash = async () => {
    if (!visibleTotal || !window.confirm("Empty Keepframe Trash?\n\nFiles will be sent to the Windows Recycle Bin. This cannot be undone inside Keepframe.")) return;
    setBusy(true);
    try {
      const result = await api.emptyTrash();
      setNotice(`${result.affected} managed ${result.affected === 1 ? "file was" : "files were"} sent to the Windows Recycle Bin${result.failed ? `; ${result.failed} need attention` : ""}.`);
      await refreshCatalogue();
    } catch (error) { setNotice(`Empty Trash failed: ${String(error)}`); } finally { setBusy(false); }
  };
  const saveTags = async (tags: string[]) => { if (!tagAsset) return; await api.updateTags(tagAsset.id, tags); setTagAsset(null); setNotice("Tags updated. Ctrl+Z to undo."); await refreshCatalogue(); };
  const saveLocations = async (ids: string[], latitude: number, longitude: number) => {
    try {
      const updated = await api.updateLocations(ids, latitude, longitude);
      setNotice(`${updated} ${updated === 1 ? "map location updated" : "map locations updated"}. Ctrl+Z reverses one catalogue action at a time.`);
      await refreshCatalogue(); await loadMapAssets();
    } catch (error) { setNotice(`Could not update map location: ${String(error)}`); }
  };
  const clearManualLocation = async (id: string) => {
    try {
      const cleared = await api.clearManualLocation(id);
      setNotice(cleared ? "Manual map pin cleared; embedded GPS was restored where available." : "This photograph has no manual map pin to clear.");
      await refreshCatalogue(); await loadMapAssets();
    } catch (error) { setNotice(`Could not clear manual map pin: ${String(error)}`); }
  };
  const selectMapAsset = async (id: string) => {
    const existing = assets.find((asset) => asset.id === id);
    if (existing) { setSingleSelection(id); return; }
    try {
      const asset = await api.asset(id);
      if (!asset) { setNotice("That map photograph is no longer in the catalogue."); return; }
      setAssets((current) => current.some((item) => item.id === id) ? current : [asset, ...current]);
      setSingleSelection(id);
    } catch (error) { setNotice(`Could not select map photograph: ${String(error)}`); }
  };
  const openAsset = (asset: Asset) => { setSelection(current=>current.selectedIds.has(asset.id)?{...current,activeId:asset.id}:{activeId:asset.id,anchorId:asset.id,selectedIds:new Set([asset.id])}); setView("develop"); };
  const selectLibraryAsset=(asset:Asset,event?:{ctrlKey:boolean;metaKey:boolean;shiftKey:boolean})=>setSelection(current=>selectId(current,asset.id,resultIds,{toggle:Boolean(event?.ctrlKey||event?.metaKey),range:Boolean(event?.shiftKey),additiveRange:Boolean(event?.shiftKey&&(event?.ctrlKey||event?.metaKey))}));
  const removeSelectedId=(id:string)=>setSelection(current=>{const selectedIds=new Set(current.selectedIds);selectedIds.delete(id);const activeId=current.activeId===id?(assets.find(asset=>selectedIds.has(asset.id))?.id??null):current.activeId;return{activeId,anchorId:current.anchorId===id?activeId:current.anchorId,selectedIds};});
  const syncSelection=async(sourceId:string,categories:SyncCategory[])=>{const summary=await api.syncDevelopSettings(sourceId,[...selection.selectedIds],categories);setNotice(`${summary.changed} destination photographs synchronised as one undoable action; mask IDs were regenerated.`);await refreshCatalogue();return summary;};
  const applyPresetSelection=async(presetId:string)=>{const summary=await api.applyPresetToSelection([...selection.selectedIds],presetId);setNotice(`${summary.changed} photographs received the preset as one undoable action.`);await refreshCatalogue();return summary;};
  const runBatchAuto=async()=>{const ids=[...selection.selectedIds];const preview=await api.previewBatchAuto(ids);if(!window.confirm(`Batch Auto analysed ${preview.analyzable} of ${preview.requested} photographs independently.\n\n${preview.lowConfidenceWhiteBalance} have low-confidence white balance; ${preview.skipped} would be skipped. Apply the proposals?`))return preview;setOperationProgress({kind:"Batch Auto",current:0,total:ids.length});try{const summary=await api.applyBatchAuto(ids);setNotice(`Batch Auto changed ${summary.changed}; ${summary.failed} failed; ${summary.cancelled} cancelled. The accepted batch is one undo item.`);await refreshCatalogue();return summary;}finally{setOperationProgress(null);}};
  const exportSelected=()=>{if(!selected)return;setExportSelection([...selection.selectedIds]);setExportAsset(selected);};
  const enqueue = async (assetIds: string[], brief: string) => { const unique = [...new Set(assetIds)]; if (!unique.length) return; await api.enqueue(unique, brief); setJobs(await api.jobs()); setNotice(`${unique.length} ${unique.length === 1 ? "photograph is" : "photographs are"} being analysed for individual recipe review.`); };
  const runLocal = async (assetId: string, recipe: EditRecipe, prompt: string) => {
    try {
      const job = await api.enqueueReviewed(assetId, recipe, prompt);
      await api.updateJob(job.id, "approve");
      setJobs(await api.jobs());
      setNotice("Local edit started. Keepframe will update the job as it runs.");
    } catch (error) { setNotice(`Could not start local edit: ${String(error)}`); }
  };
  const updateJob = async (id: string, action: "approve" | "cancel" | "retry" | "accept" | "reject") => { try { await api.updateJob(id, action); setJobs(await api.jobs()); } catch (error) { setNotice(`Job action failed: ${String(error)}`); } };
  const saveJobReview = async (id: string, recipe: EditRecipe, prompt: string) => { try { await api.saveJobReview(id, recipe, prompt); setJobs(await api.jobs()); setNotice("Image-specific recipe saved."); } catch (error) { setNotice(`Could not save recipe: ${String(error)}`); } };
  const approveJobs = async (ids: string[]) => { try { await api.approveJobs(ids); setJobs(await api.jobs()); setNotice(`${ids.length} reviewed ${ids.length === 1 ? "job" : "jobs"} approved for local editing.`); } catch (error) { setNotice(`Bulk approval failed: ${String(error)}`); } };
  const exportImage = async (asset: Asset) => {
    try {
      const path = await api.exportImage(asset);
      if (path) setNotice(`Photograph exported to ${path}`);
    } catch (error) { setNotice(`Could not export photograph: ${String(error)}`); }
  };
  const replaceImage = async (asset: Asset) => {
    try {
      const version = await api.chooseReplacement(asset);
      if (version) {
        await refreshCatalogue();
        setNotice("Replacement imported and tagged 'replaced'; the protected original and catalogue metadata are unchanged.");
      }
    } catch (error) { setNotice(`Could not import replacement: ${String(error)}`); }
  };
  const exportSidecars = async (scope: "selected" | "filtered" | "library", replaceExisting: boolean) => {
    try {
      const ids = scope === "selected" && selected ? [selected.id] : await api.assetIds(scope === "library" ? { decision: "all", search: "" } : effectiveFilter);
      if (!ids.length) { setNotice("No photographs match that sidecar export scope."); return; }
      if (scope === "library" && !window.confirm(`Export XMP sidecars for all ${ids.length} catalogue photographs? This writes local metadata files beside originals and may include GPS coordinates.`)) return;
      const replace = replaceExisting || window.confirm("Replace any existing XMP sidecars in this explicit export? Choose Cancel to preserve existing sidecars.");
      setBusy(true);
      const summary = await api.exportSidecars(ids, replace);
      const failures = summary.failed.length ? ` ${summary.failed.length} could not be written.` : "";
      setNotice(`${summary.written} XMP sidecar${summary.written === 1 ? "" : "s"} written; ${summary.preservedExisting} existing sidecar${summary.preservedExisting === 1 ? "" : "s"} preserved.${failures}`);
    } catch (error) { setNotice(`XMP export failed: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const importSidecar = async () => {
    if (!selected) return;
    setBusy(true);
    try {
      const result = await api.importSidecar(selected.id);
      await refreshCatalogue();
      const changes = [result.tagsImported && "tags", result.locationImported && "location", result.triageImported && "triage"].filter(Boolean);
      setNotice(`${changes.length ? `Sidecar imported: ${changes.join(", ")}.` : "Sidecar contained no safe catalogue changes."}${result.conflicts.length ? ` ${result.conflicts.join(" ")}` : ""}`);
    } catch (error) { setNotice(`XMP import failed: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const exportPortableCatalogue = async () => {
    setBusy(true);
    try { const path = await api.exportPortableCatalogue(); if (path) setNotice(`Portable catalogue exported to ${path}`); }
    catch (error) { setNotice(`Portable catalogue export failed: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const rescanLibrary = async () => {
    setBusy(true);
    try {
      const report = await api.rescanLibrary();
      await refreshCatalogue();
      setNotice(`Rescan checked ${report.scannedAssets} assets: ${report.missingOriginals} missing originals, ${report.missingDerivedVersions} missing derived versions, ${report.modifiedOriginals} modified originals, ${report.untrackedManagedFiles} untracked managed files, ${report.sidecarConflicts} sidecar conflicts.`);
    } catch (error) { setNotice(`Library rescan failed: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const relinkSelected = async () => {
    if (!selected) return;
    setBusy(true);
    try { const path = await api.relinkSelectedAsset(selected.id); if (path) { await refreshCatalogue(); setNotice(`Selected photograph relinked after hash confirmation: ${path}`); } }
    catch (error) { setNotice(`Relink failed: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const addFolderWatch = async () => {
    setBusy(true);
    try { if (await api.addFolderWatch()) setNotice("Folder watch enabled. Changes will appear in the local Changes-detected Inbox; no files are imported automatically."); }
    catch (error) { setNotice(`Folder watch could not be enabled: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const disableFolderWatches = async () => {
    setBusy(true);
    try { await api.disableFolderWatches(); setNotice("All folder watches are disabled. Existing Inbox findings were retained."); }
    catch (error) { setNotice(`Folder watches could not be disabled: ${String(error)}`); }
    finally { setBusy(false); }
  };
  const showFolderWatchEvents = async () => {
    try {
      const events = await api.folderWatchEvents();
      if (!events.length) { setNotice("The Changes-detected Inbox is empty."); return; }
      const counts = events.reduce<Record<string, number>>((value, event) => ({ ...value, [event.kind]: (value[event.kind] ?? 0) + 1 }), {});
      setNotice(`Changes-detected Inbox: ${events.length} finding${events.length === 1 ? "" : "s"} (${counts.new_file ?? 0} new, ${counts.file_changed ?? 0} changed, ${counts.file_removed ?? 0} removed).`);
    } catch (error) { setNotice(`Could not read folder-watch findings: ${String(error)}`); }
  };

  if (busy && !status.configured) return <div className="app-loading"><span className="brand-mark">K</span><p>Opening Keepframe…</p></div>;
  if (!status.configured) return <SetupScreen issue={status.libraryIssue} onChoose={api.chooseLibrary} onCreate={async (path) => { setBusy(true); try { setStatus(await api.initialiseLibrary(path)); } catch (error) { setNotice(`Could not open the library: ${String(error)}`); } finally { setBusy(false); } }} />;

  return (
    <div className="app-shell">
      <Sidebar view={view} status={{ ...status, ...health }} onView={(nextView) => { setView(nextView); if (nextView === "trash") setFilter((value) => ({ ...value, decision: "all" })); }} />
      <div className="workspace">
        <Topbar search={filter.search} decision={filter.decision} busy={busy} onSearch={(search) => setFilter((value) => ({ ...value, search }))} onDecision={(decision) => setFilter((value) => ({ ...value, decision }))} tags={knownTags} selectedTag={filter.tag} onTag={(tag) => setFilter((value) => ({ ...value, tag }))} dateFrom={filter.dateFrom} dateTo={filter.dateTo} onDateRange={(dateFrom, dateTo) => setFilter((value) => ({ ...value, dateFrom, dateTo }))} onImport={importPhotos} onUndo={undoCatalogue} discardCount={filter.decision === "discard" ? visibleTotal : status.counts.discard} onDeleteAll={moveAllDiscarded} />
        {view === "library" ? <LibraryProductivityView assets={assets} total={visibleTotal} visibleSelectedCount={visibleSelectedCount} loading={loadingAssets} hasMore={hasMoreAssets} onLoadMore={loadMoreAssets} filter={filter} onFilter={patch=>setFilter(value=>({...value,...patch}))} active={selected} activeId={selection.activeId} selectedIds={selection.selectedIds} layout={libraryLayout} onLayout={setLibraryLayout} onSelect={selectLibraryAsset} onSetActive={id=>setSelection(current=>current.selectedIds.has(id)?{...current,activeId:id}:selectId(current,id,resultIds))} onRemove={removeSelectedId} onClear={()=>setSelection(clearSelection())} onOpenDevelop={openAsset} onMetadata={applySelectionMetadata} onReviewAsset={reviewOne} onSync={syncSelection} onPreset={applyPresetSelection} onBatchAuto={runBatchAuto} onExport={exportSelected} autoAdvance={autoAdvance} onAutoAdvance={setAutoAdvance} /> : null}
        {view === "develop" ? <DevelopView assets={assets} selected={selected} selectedIds={selection.selectedIds} onSelect={selectLibraryAsset} onOpenSync={()=>{setLibraryLayout("grid");setView("library");setNotice("Selection retained. Choose Sync settings to select categories and apply from the active photograph.");}} onExport={async (asset) => {setExportSelection([...selection.selectedIds]);setExportAsset(asset);}} onRecipeSaved={() => { void refreshCatalogue(); }} /> : null}
        {view === "triage" ? <TriageView assets={assets} total={visibleTotal} hasMore={hasMoreAssets} loading={loadingAssets} onLoadMore={loadMoreAssets} selected={selected} onSelect={(asset) => setSingleSelection(asset.id)} onDecision={decide} onWorkshop={() => setView("workshop")} onMap={() => setView("map")} onTags={setTagAsset} onExport={exportImage} onReplace={replaceImage} onAutoAdjustments={api.autoBasicAdjustments} onPreviewAdjustments={api.previewBasicAdjustments} onApplyAdjustments={api.applyBasicAdjustments} onAdjustmentSaved={() => setNotice("Adjustment saved as a candidate version; the protected original is unchanged.")} /> : null}
        {view === "map" ? <MapView assets={assets} mapAssets={mapAssets} mapLoading={mapLoading} total={visibleTotal} selected={selected} onSelect={selectMapAsset} onLocations={saveLocations} onClearManualLocation={clearManualLocation} onOpenSelected={() => setView("triage")} onLocatedFilter={(located) => { setSpatialBounds(undefined); setFilter((value) => ({ ...value, located })); }} spatialBounds={spatialBounds} onUseVisibleBounds={setSpatialBounds} onClearSpatialBounds={() => setSpatialBounds(undefined)} /> : null}
        {view === "workshop" ? <WorkshopView asset={selected} assets={assets} jobs={jobs} serviceHealth={health} onAnalyse={(asset, intent: EditIntent, action, brief) => api.analyse(asset, intent, action, brief)} onPrompts={api.prompts} onCopy={api.copy} onPrepare={async (assetId, provider, prompt) => { const path = await api.prepareCloud(assetId, provider, prompt); setJobs(await api.jobs()); setNotice(`Prepared a local PNG and provider-specific instruction at ${path}. Upload it yourself, then import the returned image.`); }} onExportExternal={async (assetId, provider, prompt) => { const path = await api.exportExternal(assetId, provider, prompt); if (path) setNotice(`External AI image and prompt exported to ${path}`); }} onImportReturned={async (assetId, provider, prompt, recipe) => { const job = await api.importReturned(assetId, provider, prompt, recipe); if (job) { setJobs(await api.jobs()); setNotice("Returned edit validated and imported as a candidate version. The original is unchanged."); } }} onLoadVersions={api.versions} onSetPreferred={async (assetId, versionId) => { await api.setPreferredVersion(assetId, versionId); await refreshCatalogue(); setNotice("Preferred display version updated; catalogue metadata is unchanged."); }} onExport={exportImage} onReplace={replaceImage} onAutoAdjustments={api.autoBasicAdjustments} onPreviewAdjustments={api.previewBasicAdjustments} onApplyAdjustments={async (asset, adjustments: BasicAdjustments) => { const version = await api.applyBasicAdjustments(asset, adjustments); setNotice("Adjustment saved as a candidate version; the protected original is unchanged."); return version; }} onEnqueue={enqueue} onRunLocal={runLocal} onJob={updateJob} onSaveJobReview={saveJobReview} onApproveJobs={approveJobs} /> : null}
        {view === "trash" ? <LibraryView mode="trash" assets={assets} total={visibleTotal} loading={loadingAssets} hasMore={hasMoreAssets} onLoadMore={loadMoreAssets} selected={selected} onSelect={(asset) => setSingleSelection(asset.id)} onOpen={(asset) => setSingleSelection(asset.id)} onRestoreAll={restoreAllTrash} onEmptyTrash={emptyAllTrash} /> : null}
        {view === "settings" ? <SettingsView status={status} health={health} busy={busy} selectedAssetName={selected?.filename} onExportSidecars={exportSidecars} onImportSidecar={importSidecar} onExportPortableCatalogue={exportPortableCatalogue} onRescanLibrary={rescanLibrary} onRelinkSelected={relinkSelected} onAddFolderWatch={addFolderWatch} onDisableFolderWatches={disableFolderWatches} onShowFolderWatchEvents={showFolderWatchEvents} onRefreshAi={async () => { setHealth(await api.serviceHealth(true)); }} onConfigureAi={async (url) => { await api.configureLocalAi(url); setHealth(await api.serviceHealth(true)); setNotice("Local AI service address saved. Keepframe permits loopback addresses only."); }} onIntegrity={async () => { setBusy(true); try { setNotice(`Catalogue integrity: ${await api.catalogueIntegrity()}.`); } catch (error) { setNotice(`Integrity check failed: ${String(error)}`); } finally { setBusy(false); } }} onBackup={async () => { setBusy(true); try { setNotice(`Verified backup created at ${await api.createBackup()}`); } catch (error) { setNotice(`Backup failed: ${String(error)}`); } finally { setBusy(false); } }} onRestore={async () => { if (!window.confirm("Restore a catalogue backup? Keepframe will first create a safety backup of the current catalogue.")) return; setBusy(true); try { if (await api.restoreBackup()) { await refreshCatalogue(); setNotice("Catalogue backup restored and verified."); } } catch (error) { setNotice(`Restore failed: ${String(error)}`); } finally { setBusy(false); } }} onRebuild={async () => { setBusy(true); try { const count = await api.rebuildThumbnails(); await refreshCatalogue(); setNotice(`${count} thumbnails rebuilt.`); } catch (error) { setNotice(`Thumbnail rebuild failed: ${String(error)}`); } finally { setBusy(false); } }} onDiagnostics={async () => { setBusy(true); try { const path = await api.exportDiagnostics(); if (path) setNotice(`Privacy-safe diagnostics exported to ${path}`); } catch (error) { setNotice(`Diagnostics export failed: ${String(error)}`); } finally { setBusy(false); } }} /> : null}
      </div>
      {tagAsset ? <TagDialog asset={tagAsset} onClose={() => setTagAsset(null)} onSave={saveTags} /> : null}
      {exportAsset ? <ExportDialog assets={assets} selected={exportAsset} initialAssetIds={exportSelection} onClose={() => setExportAsset(null)} /> : null}
      {importPaths.length ? <ImportDialog sourceCount={importPaths.length} onClose={() => setImportPaths([])} onStart={startImport} /> : null}
      {notice ? <button className="toast" aria-live="polite" onClick={() => setNotice(null)}>{notice}</button> : null}
      {operationProgress ? <section className="operation-progress" aria-live="polite"><div><strong>{operationProgress.kind}</strong><span>{operationProgress.current} of {operationProgress.total}</span></div><progress max={Math.max(1, operationProgress.total)} value={operationProgress.current} /><small>{operationProgress.error ?? operationProgress.file ?? "Working safely in the background…"}</small>{operationProgress.kind === "Import" && operationProgress.importId ? <button className="quiet-button" onClick={() => api.cancelImport(operationProgress.importId!).then(() => setNotice("Import cancellation requested; the current safe boundary will finish first."))}>Cancel import</button> : null}{operationProgress.kind==="Batch Auto"?<button className="quiet-button" onClick={()=>void api.cancelLibraryBatch().then(()=>setNotice("Batch Auto cancellation requested; no catalogue change is committed until analysis completes."))}>Cancel Batch Auto</button>:null}</section> : null}
    </div>
  );
}

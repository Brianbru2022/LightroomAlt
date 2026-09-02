import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { SetupScreen } from "./components/SetupScreen";
import { TagDialog } from "./components/TagDialog";
import { Topbar } from "./components/Topbar";
import { api } from "./lib/bridge";
import { decisionLabel } from "./lib/format";
import type { Asset, AssetFilter, BatchJob, Decision, EditIntent, EditRecipe, LibraryStatus, ServiceHealth, ViewName } from "./types";
import { LibraryView } from "./views/LibraryView";
import { MapView } from "./views/MapView";
import { TriageView } from "./views/TriageView";
import { WorkshopView } from "./views/WorkshopView";

const PAGE_SIZE = 240;
const emptyStatus: LibraryStatus = { configured: false, counts: { total: 0, keep: 0, undecided: 0, discard: 0 } };
const emptyHealth: ServiceHealth = { localAiAvailable: false, serviceReachable: false, localAiBusy: false, localAiDetail: "No local image service is responding.", analysisModelInstalled: false };

export function App() {
  const [status, setStatus] = useState<LibraryStatus>(emptyStatus);
  const [health, setHealth] = useState<ServiceHealth>(emptyHealth);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [visibleTotal, setVisibleTotal] = useState(0);
  const [hasMoreAssets, setHasMoreAssets] = useState(false);
  const [loadingAssets, setLoadingAssets] = useState(true);
  const [jobs, setJobs] = useState<BatchJob[]>([]);
  const [view, setView] = useState<ViewName>("library");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState<AssetFilter>({ decision: "all", search: "" });
  const [busy, setBusy] = useState(true);
  const [tagAsset, setTagAsset] = useState<Asset | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestId = useRef(0);
  const deferredSearch = useDeferredValue(filter.search);
  const effectiveFilter = useMemo<AssetFilter>(() => ({ ...filter, search: deferredSearch }), [deferredSearch, filter.decision, filter.tag, filter.year]);
  const selected = useMemo(() => assets.find((asset) => asset.id === selectedId) ?? assets[0] ?? null, [assets, selectedId]);
  const selectedAssetId = selected?.id;

  const loadFirstPage = useCallback(async () => {
    const request = ++requestId.current;
    setLoadingAssets(true);
    try {
      const page = await api.assets(effectiveFilter, 0, PAGE_SIZE);
      if (request !== requestId.current) return;
      setAssets(page.items);
      setVisibleTotal(page.total);
      setHasMoreAssets(page.hasMore);
      setSelectedId((current) => current && page.items.some((asset) => asset.id === current) ? current : page.items[0]?.id ?? null);
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

  const undoCatalogue = useCallback(async () => {
    try {
      const changed = await api.undo();
      setNotice(changed ? "Latest catalogue change undone." : "Nothing to undo.");
      await refreshCatalogue();
    } catch (error) {
      setNotice(`Undo failed: ${String(error)}`);
    }
  }, [refreshCatalogue]);

  useEffect(() => {
    let active = true;
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
      if (event.ctrlKey && key === "k") {
        event.preventDefault();
        document.querySelector<HTMLInputElement>(".search-field input")?.focus();
        return;
      }
      if (event.ctrlKey && key === "z") {
        event.preventDefault();
        void undoCatalogue();
        return;
      }

      const target = event.target;
      const isEditing = target instanceof HTMLInputElement
        || target instanceof HTMLTextAreaElement
        || target instanceof HTMLSelectElement
        || (target instanceof HTMLElement && target.isContentEditable);
      const isOtherControl = target instanceof HTMLElement
        && target.closest("button, [role='button']")
        && !target.closest(".asset-card");
      if (busy || isEditing || isOtherControl || event.ctrlKey || event.altKey || event.metaKey || view !== "library" || !selectedAssetId) return;

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
        setSelectedId(nextId);
        next.focus({ preventScroll: true });
        next.scrollIntoView?.({ block: "nearest", inline: "nearest" });
        return;
      }

      if (key === "x" || key === "m") {
        event.preventDefault();
        void api.setDecision(selectedAssetId, key === "x" ? "discard" : "keep").then(refreshCatalogue);
      }
    };
    window.addEventListener("keydown", shortcut); return () => window.removeEventListener("keydown", shortcut);
  }, [busy, refreshCatalogue, selectedAssetId, undoCatalogue, view]);

  useEffect(() => {
    const index = assets.findIndex((asset) => asset.id === selectedAssetId);
    if (index >= assets.length - 20 && hasMoreAssets) void loadMoreAssets();
  }, [assets, hasMoreAssets, loadMoreAssets, selectedAssetId]);

  const decide = async (id: string, decision: Decision) => {
    const index = assets.findIndex((asset) => asset.id === id);
    const current = assets[index];
    const next = assets[index + 1];
    const changed = current?.decision !== decision;
    if (next) setSelectedId(next.id);
    try {
      await api.setDecision(id, decision);
      setNotice(!changed
        ? `${current.filename} was already marked ${decisionLabel[decision]}.`
        : `${current?.filename ?? "Photograph"} marked ${decisionLabel[decision]}. Ctrl+Z to undo.`);
      await refreshCatalogue();
    } catch (error) {
      setSelectedId(id);
      setNotice(`Could not update decision: ${String(error)}`);
    }
  };
  const importPhotos = async () => {
    const paths = await api.chooseImport(); if (!paths.length) return;
    setBusy(true); try { const result = await api.importPhotos(paths); setNotice(`Imported ${result.imported}; ${result.duplicates} duplicates skipped; ${result.failed} failed.`); await refreshCatalogue(); } finally { setBusy(false); }
  };
  const deleteAllDiscarded = async () => {
    if (!visibleTotal) return;
    const scope = filter.search ? " matching the current search" : "";
    if (!window.confirm(`Move all ${visibleTotal} discarded ${visibleTotal === 1 ? "photograph" : "photographs"}${scope} to the Windows Recycle Bin?\n\nThis cannot be undone inside Keepframe.`)) return;
    setBusy(true);
    try {
      const ids = await api.assetIds({ ...effectiveFilter, decision: "discard" });
      const result = await api.deleteDiscarded(ids);
      setNotice(`${result.deleted} discarded ${result.deleted === 1 ? "photograph" : "photographs"} moved to the Recycle Bin${result.failed ? `; ${result.failed} could not be removed` : ""}.`);
      await refreshCatalogue();
    } catch (error) {
      setNotice(`Could not delete discarded photographs: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };
  const saveTags = async (tags: string[]) => { if (!tagAsset) return; await api.updateTags(tagAsset.id, tags); setTagAsset(null); setNotice("Tags updated. Ctrl+Z to undo."); await refreshCatalogue(); };
  const saveLocation = async (id: string, latitude: number, longitude: number) => { await api.updateLocation(id, latitude, longitude); setNotice("Map location updated. Ctrl+Z to undo."); await refreshCatalogue(); };
  const openAsset = (asset: Asset) => { setSelectedId(asset.id); setView("triage"); };
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

  if (busy && !status.configured) return <div className="app-loading"><span className="brand-mark">K</span><p>Opening Keepframe…</p></div>;
  if (!status.configured) return <SetupScreen onChoose={api.chooseLibrary} onCreate={async (path) => { setBusy(true); try { setStatus(await api.initialiseLibrary(path)); } finally { setBusy(false); } }} />;

  return (
    <div className="app-shell">
      <Sidebar view={view} status={{ ...status, ...health }} onView={setView} />
      <div className="workspace">
        <Topbar search={filter.search} decision={filter.decision} busy={busy} onSearch={(search) => setFilter((value) => ({ ...value, search }))} onDecision={(decision) => setFilter((value) => ({ ...value, decision }))} onImport={importPhotos} onUndo={undoCatalogue} discardCount={filter.decision === "discard" ? visibleTotal : status.counts.discard} onDeleteAll={deleteAllDiscarded} />
        {view === "library" ? <LibraryView assets={assets} total={visibleTotal} loading={loadingAssets} hasMore={hasMoreAssets} onLoadMore={loadMoreAssets} selected={selected} onSelect={(asset) => setSelectedId(asset.id)} onOpen={openAsset} /> : null}
        {view === "triage" ? <TriageView assets={assets} total={visibleTotal} hasMore={hasMoreAssets} loading={loadingAssets} onLoadMore={loadMoreAssets} selected={selected} onSelect={(asset) => setSelectedId(asset.id)} onDecision={decide} onWorkshop={() => setView("workshop")} onMap={() => setView("map")} onTags={setTagAsset} /> : null}
        {view === "map" ? <MapView assets={assets} total={visibleTotal} hasMore={hasMoreAssets} loading={loadingAssets} onLoadMore={loadMoreAssets} selected={selected} onSelect={(asset) => setSelectedId(asset.id)} onLocation={saveLocation} /> : null}
        {view === "workshop" ? <WorkshopView asset={selected} assets={assets} jobs={jobs} serviceHealth={health} onAnalyse={(asset, intent: EditIntent, brief) => api.analyse(asset, intent, brief)} onPrompts={api.prompts} onCopy={api.copy} onPrepare={async (assetId, provider, prompt) => { const path = await api.prepareCloud(assetId, provider, prompt); setNotice(`Prepared cloud-edit PNG and prompt at ${path}`); }} onExportExternal={async (assetId, provider, prompt) => { const path = await api.exportExternal(assetId, provider, prompt); if (path) setNotice(`External AI image and prompt exported to ${path}`); }} onImportReturned={async (assetId, provider, prompt) => { const job = await api.importReturned(assetId, provider, prompt); if (job) { setJobs(await api.jobs()); setNotice("Returned edit imported as a candidate version."); } }} onLoadVersions={api.versions} onSetPreferred={async (assetId, versionId) => { await api.setPreferredVersion(assetId, versionId); await refreshCatalogue(); setNotice("Preferred display version updated; catalogue metadata is unchanged."); }} onReplace={async (asset) => { const version = await api.chooseReplacement(asset); if (version) { await refreshCatalogue(); setNotice("Replacement imported as a derived version; original metadata and file are preserved."); } }} onEnqueue={enqueue} onRunLocal={runLocal} onJob={updateJob} onSaveJobReview={saveJobReview} onApproveJobs={approveJobs} /> : null}
      </div>
      {tagAsset ? <TagDialog asset={tagAsset} onClose={() => setTagAsset(null)} onSave={saveTags} /> : null}
      {notice ? <button className="toast" aria-live="polite" onClick={() => setNotice(null)}>{notice}</button> : null}
    </div>
  );
}

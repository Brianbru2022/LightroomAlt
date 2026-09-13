import { Activity, Archive, BrainCircuit, Eye, FileHeart, FileOutput, HardDrive, Link2, RefreshCw, ScanSearch, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import type { LibraryStatus, SemanticIndexStatus, ServiceHealth } from "../types";
import { api } from "../lib/bridge";

type Props = {
  status: LibraryStatus;
  health: ServiceHealth;
  busy: boolean;
  onIntegrity: () => void;
  onBackup: () => void;
  onRestore: () => void;
  onRebuild: () => void;
  onDiagnostics: () => void;
  onRefreshAi: () => Promise<void>;
  onConfigureAi: (url: string) => Promise<void>;
  selectedAssetName?: string;
  onExportSidecars: (scope: "selected" | "filtered" | "library", replace: boolean) => void;
  onImportSidecar: () => void;
  onExportPortableCatalogue: () => void;
  onImportPortableCatalogue?: () => void;
  onRescanLibrary: () => void;
  onRelinkSelected: () => void;
  onAddFolderWatch: () => void;
  onDisableFolderWatches: () => void;
  onShowFolderWatchEvents: () => void;
};

function SemanticSettings({busy,onNotice}:{busy:boolean;onNotice?:(message:string)=>void}){const [status,setStatus]=useState<SemanticIndexStatus|null>(null);const refresh=()=>api.semanticIndexStatus().then(setStatus).catch(error=>onNotice?.(String(error)));useEffect(()=>{void refresh();},[]);return <section className="settings-card semantic-settings"><BrainCircuit size={22}/><div><h2>Local semantic index</h2><p>{status?.detail??"Checking derived semantic-search state…"}</p>{status?<><small>{status.model} · {status.modelRevision.slice(0,12)} · {status.licence} · {status.embeddingDimensions} dimensions · {(status.approximateBytes/1024/1024).toFixed(0)} MB</small><div className="settings-actions">{!status.installed?<button className="quiet-button" disabled={busy} onClick={()=>{if(window.confirm("Download about 778 MB of Apache-2.0 SigLIP files into D:\\AI Models\\Keepframe\\semantic? No photographs are uploaded."))void api.installSemanticModel().then(setStatus).catch(error=>onNotice?.(String(error)));}}>Install model…</button>:<><button className="quiet-button" disabled={busy||status.busy||(!status.paused&&status.queuedSources===0)} onClick={()=>void api.startSemanticIndex(false).then(setStatus).catch(error=>onNotice?.(String(error)))}>{status.paused?"Resume":status.queuedSources?"Index missing":"Index ready"}</button><button className="quiet-button" disabled={busy||status.paused||!status.busy} onClick={()=>void api.pauseSemanticIndex().then(refresh)}>Pause</button><button className="quiet-button" disabled={busy||!status.busy} onClick={()=>void api.cancelSemanticIndex().then(refresh)}>Cancel</button><button className="quiet-button" disabled={busy} onClick={()=>{if(window.confirm("Rebuild only the derived semantic index? Photos and catalogue organisation stay unchanged."))void api.startSemanticIndex(true).then(setStatus).catch(error=>onNotice?.(String(error)));}}>Rebuild</button></>}</div><small>{(status.storageBytes/1024/1024).toFixed(1)} MB indexed vectors · model files and vectors are excluded from portable catalogues.</small></>:null}</div></section>}

export function SettingsView({ status, health, busy, onIntegrity, onBackup, onRestore, onRebuild, onDiagnostics, onRefreshAi, onConfigureAi, selectedAssetName, onExportSidecars, onImportSidecar, onExportPortableCatalogue, onImportPortableCatalogue=()=>{if(window.confirm("Import catalogue organisation? Keepframe creates a verified backup first and does not alter source files."))void api.importPortableCatalogue().then(count=>{if(count!==null)window.dispatchEvent(new CustomEvent("keepframe-catalogue-restored",{detail:{count}}));});}, onRescanLibrary, onRelinkSelected, onAddFolderWatch, onDisableFolderWatches, onShowFolderWatchEvents }: Props) {
  const [url, setUrl] = useState(health.localAiUrl);
  useEffect(() => setUrl(health.localAiUrl), [health.localAiUrl]);
  return <main className="view settings-view">
    <div className="view-heading"><div><span className="eyebrow">Maintenance and privacy</span><h1>Settings & Library Health</h1><p>Recovery tools act on the selected master library only.</p></div><span className="privacy-badge"><ShieldCheck size={16} /> Local only</span></div>
    <div className="settings-grid">
      <section className="settings-card"><HardDrive size={22} /><div><h2>Master library</h2><p>{status.libraryRoot}</p><small>A persistent library marker and exclusive lock protect this catalogue.</small></div></section>
      <section className="settings-card"><Activity size={22} /><div><h2>Catalogue integrity</h2><p>Run SQLite’s full integrity check without changing photographs.</p><button className="quiet-button" disabled={busy} onClick={onIntegrity}>Check now</button></div></section>
      <section className="settings-card"><Archive size={22} /><div><h2>Verified backup</h2><p>Creates a WAL-consistent backup and checks it before reporting success.</p><div className="settings-actions"><button className="quiet-button" disabled={busy} onClick={onBackup}>Create backup</button><button className="quiet-button" disabled={busy} onClick={onRestore}>Restore backup…</button></div></div></section>
      <section className="settings-card"><RefreshCw size={22} /><div><h2>Disposable previews</h2><p>Rebuild thumbnails from catalogued originals. Decisions, tags and locations are untouched.</p><button className="quiet-button" disabled={busy} onClick={onRebuild}>Rebuild thumbnails</button></div></section>
      <section className="settings-card"><FileHeart size={22} /><div><h2>Support diagnostics</h2><p>Exports version, schema, integrity and counts. No pixels, prompts or recipes are included.</p><button className="quiet-button" disabled={busy} onClick={onDiagnostics}>Export diagnostics…</button></div></section>
      <section className="settings-card"><FileOutput size={22} /><div><h2>Portable metadata</h2><p>XMP sidecars are explicit, local and never alter the photograph. Existing sidecars are preserved unless you deliberately replace them.</p><div className="settings-actions"><button className="quiet-button" disabled={busy || !selectedAssetName} onClick={() => onExportSidecars("selected", false)}>Export selected XMP</button><button className="quiet-button" disabled={busy} onClick={() => onExportSidecars("filtered", false)}>Export filtered XMP</button><button className="quiet-button" disabled={busy} onClick={() => onExportSidecars("library", false)}>Export all XMP…</button><button className="quiet-button" disabled={busy || !selectedAssetName} onClick={onImportSidecar}>Read selected XMP</button></div><small>{selectedAssetName ? `Selected: ${selectedAssetName}` : "Select a photograph to export or read one sidecar."}</small></div></section>
      <section className="settings-card"><Archive size={22} /><div><h2>Portable catalogue</h2><p>Schema v4 preserves source/version relationships, recipes, Collections, Smart rules, Stacks and explicit semantic suggestion decisions. Derived vectors and model files remain excluded.</p><div className="settings-actions"><button className="quiet-button" disabled={busy} onClick={onExportPortableCatalogue}>Export catalogue JSON…</button><button className="quiet-button" disabled={busy||!onImportPortableCatalogue} onClick={onImportPortableCatalogue}>Import catalogue JSON…</button></div></div></section>
      <section className="settings-card"><ScanSearch size={22} /><div><h2>Library rescan</h2><p>Checks missing or changed originals, derived versions, untracked managed files and previously exported sidecars. It never removes or relinks anything automatically.</p><div className="settings-actions"><button className="quiet-button" disabled={busy} onClick={onRescanLibrary}>Scan library</button><button className="quiet-button" disabled={busy || !selectedAssetName} onClick={onRelinkSelected}><Link2 size={14} /> Relink selected…</button></div></div></section>
      <section className="settings-card"><Eye size={22} /><div><h2>Changes-detected Inbox</h2><p>Watch only folders you choose. New, changed and removed files are debounced into a local Inbox; nothing is imported, removed or relinked automatically.</p><div className="settings-actions"><button className="quiet-button" disabled={busy} onClick={onAddFolderWatch}>Watch a folder…</button><button className="quiet-button" disabled={busy} onClick={onShowFolderWatchEvents}>Show findings</button><button className="quiet-button" disabled={busy} onClick={onDisableFolderWatches}>Disable watches</button></div></div></section>
      <section className="settings-card"><ShieldCheck size={22} /><div><h2>Local image editor</h2><p>{health.localAiDetail}</p><small>State: {health.localAiState.replaceAll("_", " ")}. {health.localAiModel ?? "Qwen-Image-Edit is optional."}</small><label className="ai-url-field"><span>Loopback service address</span><input value={url} onChange={(event) => setUrl(event.target.value)} aria-label="Local AI service address" /><button className="quiet-button" disabled={busy} onClick={() => void onConfigureAi(url)}>Save address</button><button className="quiet-button" disabled={busy} onClick={() => void onRefreshAi()}>Refresh status</button></label></div></section>
      <section className="settings-card"><Activity size={22} /><div><h2>Image analysis</h2><p>{health.analysisDetail}</p><small>{health.analysisAvailable ? "Photographs are analysed locally." : "Build Recipe remains available using your selected controls and brief."}</small></div></section>
      <SemanticSettings busy={busy}/>
    </div>
  </main>;
}

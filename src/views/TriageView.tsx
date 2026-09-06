import { Check, CircleHelp, Crop, Download, Info, LoaderCircle, MapPin, Maximize2, RotateCcw, SlidersHorizontal, Sparkles, Tag, Upload, WandSparkles, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { neutralAdjustments, type Asset, type AssetVersion, type BasicAdjustments, type Decision } from "../types";
import { formatDate } from "../lib/format";
import { api } from "../lib/bridge";
import { PhotoAdjustmentControls } from "../components/PhotoAdjustmentControls";
import { Histogram } from "../components/Histogram";
import { CropOverlay } from "../components/CropOverlay";
import { adjustmentPresets, applyAdjustmentPreset } from "../lib/adjustmentPresets";
import { useAdjustmentPreview } from "../hooks/useAdjustmentPreview";

type Props = {
  assets: Asset[];
  total: number;
  hasMore: boolean;
  loading: boolean;
  onLoadMore: () => void;
  selected: Asset | null;
  onSelect: (asset: Asset) => void;
  onDecision: (id: string, decision: Decision) => void;
  onWorkshop: () => void;
  onMap: () => void;
  onTags: (asset: Asset) => void;
  onExport: (asset: Asset) => Promise<void>;
  onReplace: (asset: Asset) => Promise<void>;
  onAutoAdjustments: (assetId: string) => Promise<BasicAdjustments>;
  onPreviewAdjustments: (asset: Asset, adjustments: BasicAdjustments) => Promise<string>;
  onApplyAdjustments: (asset: Asset, adjustments: BasicAdjustments) => Promise<AssetVersion>;
  onAdjustmentSaved: () => void;
};

export function TriageView({ assets, total, hasMore, loading, onLoadMore, selected, onSelect, onDecision, onWorkshop, onMap, onTags, onExport, onReplace, onAutoAdjustments, onPreviewAdjustments, onApplyAdjustments, onAdjustmentSaved }: Props) {
  const [zoomed, setZoomed] = useState(false);
  const [reviewUrl, setReviewUrl] = useState<string | null>(null);
  const [reviewError, setReviewError] = useState<string | null>(null);
  const [adjustments, setAdjustments] = useState<BasicAdjustments>(neutralAdjustments);
  const [adjusting, setAdjusting] = useState<"auto" | "apply" | null>(null);
  const [adjustmentError, setAdjustmentError] = useState<string | null>(null);
  const [adjustmentPreviewUrl, setAdjustmentPreviewUrl] = useState<string | null>(null);
  const [showOriginal, setShowOriginal] = useState(false);
  const [cropping, setCropping] = useState(false);
  const adjustmentsRef = useRef(neutralAdjustments);
  adjustmentsRef.current = adjustments;
  const selectedId = selected?.id;
  const decisionRef = useRef(onDecision); decisionRef.current = onDecision;
  const selectRef = useRef(onSelect); selectRef.current = onSelect;
  const workshopRef = useRef(onWorkshop); workshopRef.current = onWorkshop;
  const mapRef = useRef(onMap); mapRef.current = onMap;
  const tagsRef = useRef(onTags); tagsRef.current = onTags;
  const assetsRef = useRef(assets); assetsRef.current = assets;
  const selectedRef = useRef(selected); selectedRef.current = selected;
  const selectedIndex = Math.max(0, assets.findIndex((asset) => asset.id === selectedId));
  const filmstripStart = Math.max(0, selectedIndex - 30);
  const filmstripAssets = assets.slice(filmstripStart, selectedIndex + 31);
  const { request: requestAdjustmentPreview, cancel: cancelAdjustmentPreview } = useAdjustmentPreview(onPreviewAdjustments, setAdjustmentPreviewUrl, (error) => setAdjustmentError(String(error)));
  useEffect(() => {
    cancelAdjustmentPreview();
    adjustmentsRef.current = neutralAdjustments;
    setZoomed(false); setReviewUrl(null); setReviewError(null); setAdjustments(neutralAdjustments); setAdjustmentError(null); setAdjustmentPreviewUrl(null); setShowOriginal(false); setCropping(false);
  }, [selectedId, cancelAdjustmentPreview]);
  const toggleZoom = () => {
    if (zoomed) { setZoomed(false); return; }
    if (!selected) return;
    setReviewError(null);
    void api.reviewPreview(selected).then((url) => { setReviewUrl(url); setZoomed(true); }).catch((error) => setReviewError(String(error)));
  };
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target;
      const isEditing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable);
      if (!selectedId || isEditing || event.ctrlKey || event.altKey || event.metaKey) return;
      const key = event.key.toLowerCase();
      if (event.shiftKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
        const current = assetsRef.current.findIndex((asset) => asset.id === selectedId);
        const next = assetsRef.current[current + (event.key === "ArrowLeft" ? -1 : 1)];
        if (next) {
          event.preventDefault();
          selectRef.current(next);
        }
        return;
      }
      if (key === "e") { event.preventDefault(); workshopRef.current(); return; }
      if (key === "m") { event.preventDefault(); mapRef.current(); return; }
      if (key === "t" && selectedRef.current) { event.preventDefault(); tagsRef.current(selectedRef.current); return; }
      if (event.code === "Backslash") { event.preventDefault(); setShowOriginal((current) => !current); return; }
      const decision = event.key === "ArrowRight" || key === "k" ? "keep" : event.key === "ArrowLeft" || key === "x" ? "discard" : event.key === "ArrowUp" || key === "u" ? "undecided" : null;
      if (decision) { event.preventDefault(); decisionRef.current(selectedId, decision); }
      if (event.code === "Space") { event.preventDefault(); toggleZoom(); }
    };
    window.addEventListener("keydown", handler); return () => window.removeEventListener("keydown", handler);
  }, [selectedId]);

  const setAdjustment = (key: keyof BasicAdjustments, value: number) => {
    if (!selected) return;
    const next = { ...adjustmentsRef.current, [key]: value };
    adjustmentsRef.current = next;
    setAdjustments(next);
    setAdjustmentError(null);
    requestAdjustmentPreview(selected, next);
  };
  const replaceAdjustments = (next: BasicAdjustments) => {
    if (!selected) return;
    adjustmentsRef.current = next; setAdjustments(next); setAdjustmentError(null); requestAdjustmentPreview(selected, next);
  };
  const autoAdjust = async () => {
    if (!selected) return;
    setAdjusting("auto"); setAdjustmentError(null);
    try {
      const next = await onAutoAdjustments(selected.id);
      adjustmentsRef.current = next;
      setAdjustments(next);
      cancelAdjustmentPreview();
      const url = await onPreviewAdjustments(selected, next);
      setAdjustmentPreviewUrl(url);
    } catch (error) { setAdjustmentError(String(error)); } finally { setAdjusting(null); }
  };
  const choosePreset = (presetId: string) => {
    if (!selected) return;
    const preset = adjustmentPresets.find((entry) => entry.id === presetId); if (!preset) return;
    const next = applyAdjustmentPreset(adjustmentsRef.current, preset);
    adjustmentsRef.current = next; setAdjustments(next); setAdjustmentError(null); requestAdjustmentPreview(selected, next);
  };
  const applyAdjustments = async () => {
    if (!selected) return;
    setAdjusting("apply"); setAdjustmentError(null);
    try { await onApplyAdjustments(selected, adjustments); onAdjustmentSaved(); } catch (error) { setAdjustmentError(String(error)); } finally { setAdjusting(null); }
  };
  if (!selected) return <main className="view empty-state"><CircleHelp size={42} /><h2>Nothing to triage</h2></main>;
  return (
    <main className="view triage-view">
      <div className="triage-stage">
        <div className={`hero-photo ${zoomed ? "zoomed" : ""}`}><img src={showOriginal ? selected.previewUrl : adjustmentPreviewUrl || (zoomed && reviewUrl ? reviewUrl : selected.preferredVersionUrl ?? selected.previewUrl)} alt={selected.filename} />{cropping && !zoomed ? <CropOverlay value={adjustments} onChange={(change) => replaceAdjustments({ ...adjustmentsRef.current, ...change })} /> : null}{reviewError ? <span className="review-error">Full-resolution review unavailable: {reviewError}</span> : null}</div>
        <button className="before-after-button" onPointerDown={() => setShowOriginal(true)} onPointerUp={() => setShowOriginal(false)} onPointerLeave={() => setShowOriginal(false)} onClick={() => setShowOriginal((current) => !current)}>Hold original <kbd>\\</kbd></button>
        <div className="triage-actions">
          <button className={`triage-button discard ${selected.decision === "discard" ? "active" : ""}`} onClick={() => onDecision(selected.id, "discard")}><X size={20} /><span>Discard</span><kbd>←</kbd></button>
          <button className={`triage-button undecided ${selected.decision === "undecided" ? "active" : ""}`} onClick={() => onDecision(selected.id, "undecided")}><CircleHelp size={20} /><span>Undecided</span><kbd>↑</kbd></button>
          <button className={`triage-button keep ${selected.decision === "keep" ? "active" : ""}`} onClick={() => onDecision(selected.id, "keep")}><Check size={20} /><span>Keep</span><kbd>→</kbd></button>
        </div>
        <div className="filmstrip" aria-label="Triage filmstrip">
          {filmstripStart > 0 ? <span className="filmstrip-count">+{filmstripStart} earlier</span> : null}
          {filmstripAssets.map((asset) => <button key={asset.id} className={asset.id === selected.id ? "active" : ""} onClick={() => onSelect(asset)}><img src={asset.thumbnailUrl} alt={asset.filename} /><span className={`mini-state ${asset.decision}`} /></button>)}
          {hasMore ? <button className="filmstrip-more" disabled={loading} onClick={onLoadMore}>{loading ? "…" : `+${Math.max(0, total - assets.length)}`}</button> : assets.length > filmstripStart + filmstripAssets.length ? <span className="filmstrip-count">+{assets.length - filmstripStart - filmstripAssets.length} later</span> : null}
        </div>
      </div>
      <aside className="inspector">
        <div className="inspector-heading"><div><span className="eyebrow">Current frame</span><h2>{selected.filename}</h2></div><Info size={18} /></div>
        <dl className="metadata-list">
          <div><dt>Captured</dt><dd>{formatDate(selected.capturedAt, { dateStyle: "medium", timeStyle: "short" })}{selected.dateFallback ? <em>File date</em> : null}</dd></div>
          <div><dt>Camera</dt><dd>{selected.camera ?? "Unknown"}</dd></div>
          <div><dt>Dimensions</dt><dd>{selected.width} × {selected.height}</dd></div>
          <div><dt>Files</dt><dd>{selected.representationCount > 1 ? `${selected.representationCount} paired representations` : "1 original"}</dd></div>
          <div><dt>Format</dt><dd>{selected.filename.split(".").pop()?.toUpperCase() ?? "Unknown"}{selected.latitude !== null && selected.longitude !== null ? " · location available" : ""}</dd></div>
        </dl>
        <Histogram src={adjustmentPreviewUrl || selected.previewUrl} />
        <div className="tag-list">{selected.tags.map((tag) => <span key={tag}>{tag}</span>)}<button onClick={() => onTags(selected)}><Tag size={13} /> Edit <kbd>T</kbd></button></div>
        <section className="triage-adjustments" aria-labelledby="triage-adjustments-heading">
          <div className="triage-adjustment-heading"><h3 id="triage-adjustments-heading"><SlidersHorizontal size={16} /> Adjust</h3><span><button title="Crop image" aria-label="Crop image" onClick={() => setCropping((current) => !current)}><Crop size={14} /></button><button title="Reset adjustments" aria-label="Reset adjustments" disabled={adjusting !== null} onClick={() => { cancelAdjustmentPreview(); adjustmentsRef.current = neutralAdjustments; setAdjustments(neutralAdjustments); setAdjustmentPreviewUrl(null); }}><RotateCcw size={14} /></button></span></div>
          <label className="preset-picker">Preset<select aria-label="Adjustment preset" defaultValue="" onChange={(event) => { choosePreset(event.target.value); event.currentTarget.value = ""; }}><option value="" disabled>Choose a look…</option>{adjustmentPresets.map((preset) => <option key={preset.id} value={preset.id}>{preset.label}</option>)}</select></label>
          <PhotoAdjustmentControls compact value={adjustments} onChange={setAdjustment} onReplace={replaceAdjustments} />
          <button className="auto-range-button" disabled={adjusting !== null} onClick={autoAdjust}>{adjusting === "auto" ? <LoaderCircle className="spin" size={15} /> : <WandSparkles size={15} />} Maximise range</button>
          <button className="save-adjustment-button" disabled={adjusting !== null} onClick={applyAdjustments}>{adjusting === "apply" ? <LoaderCircle className="spin" size={15} /> : <Check size={15} />} Save as candidate</button>
          {adjustmentError ? <p role="alert">Could not apply adjustments: {adjustmentError}</p> : <small>Original remains protected. Saved adjustments appear in Versions.</small>}
        </section>
        <div className="inspector-actions">
          <button onClick={() => void onExport(selected)}><Download size={17} /> Export image</button>
          <button onClick={() => void onReplace(selected)}><Upload size={17} /> Import replacement</button>
          <button onClick={onWorkshop}><Sparkles size={17} /> Edit with AI <kbd>E</kbd></button>
          <button onClick={onMap}><MapPin size={17} /> Show on map <kbd>M</kbd></button>
          <button onClick={toggleZoom}><Maximize2 size={17} /> {zoomed ? "Fit image" : "Load full-resolution 100%"} <kbd>Space</kbd></button>
        </div>
        <div className="safety-note"><Check size={15} /><span>Original protected<br /><small>Shift + ←/→ browses without deciding. Ctrl+Z undoes the latest change.</small></span></div>
      </aside>
    </main>
  );
}

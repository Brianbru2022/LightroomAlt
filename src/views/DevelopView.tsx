import { Clipboard, ClipboardPaste, Eye, Maximize2, Minus, RotateCcw, Undo2, Redo2, ZoomIn } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { CropOverlay } from "../components/CropOverlay";
import { PhotoAdjustmentControls } from "../components/PhotoAdjustmentControls";
import { useAdjustmentPreview } from "../hooks/useAdjustmentPreview";
import { api } from "../lib/bridge";
import { neutralAdjustments, type Asset, type BasicAdjustments, type DevelopRecipe } from "../types";

type Props = { assets: Asset[]; selected: Asset | null; onSelect: (asset: Asset) => void; onExport: (asset: Asset) => Promise<void>; onRecipeSaved: () => void };

let copiedSettings: BasicAdjustments | null = null;
const copySettings = (settings: BasicAdjustments): BasicAdjustments => structuredClone(settings);

export function DevelopView({ assets, selected, onSelect, onExport, onRecipeSaved }: Props) {
  const [settings, setSettings] = useState<BasicAdjustments>(neutralAdjustments);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recipeEdited, setRecipeEdited] = useState(false);
  const [hasClipboard, setHasClipboard] = useState(Boolean(copiedSettings));
  const [before, setBefore] = useState(false);
  const [cropping, setCropping] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [drag, setDrag] = useState<{ x: number; y: number; pan: { x: number; y: number } } | null>(null);
  const history = useRef<BasicAdjustments[]>([neutralAdjustments]);
  const historyIndex = useRef(0);
  const groupTimer = useRef<number | null>(null);
  const saveTimer = useRef<number | null>(null);
  const latest = useRef(settings);
  latest.current = settings;
  const selectedId = selected?.id;
  const { request: requestPreview, cancel: cancelPreview } = useAdjustmentPreview(
    (asset, next) => api.previewDevelopRecipe(asset.id, { schemaVersion: 1, settings: next }),
    setPreviewUrl,
    (reason) => setError(String(reason)),
  );

  const persist = (next: BasicAdjustments) => {
    if (!selected) return;
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      void api.saveDevelopRecipe(selected.id, { schemaVersion: 1, settings: next })
        .then(() => onRecipeSaved())
        .catch((reason) => setError(String(reason)));
    }, 450);
  };
  const settleHistory = () => {
    if (groupTimer.current === null) return;
    window.clearTimeout(groupTimer.current); groupTimer.current = null;
    const current = latest.current;
    if (JSON.stringify(history.current[historyIndex.current]) !== JSON.stringify(current)) {
      history.current = [...history.current.slice(0, historyIndex.current + 1), copySettings(current)].slice(-80);
      historyIndex.current = history.current.length - 1;
    }
  };
  const change = (next: BasicAdjustments) => {
    if (!selected) return;
    latest.current = next; setSettings(next); setRecipeEdited(JSON.stringify(next) !== JSON.stringify(neutralAdjustments)); setError(null); requestPreview(selected, next); persist(next);
    if (groupTimer.current !== null) window.clearTimeout(groupTimer.current);
    groupTimer.current = window.setTimeout(settleHistory, 350);
  };
  const replace = (next: BasicAdjustments) => change(next);
  const undo = () => {
    settleHistory();
    if (historyIndex.current <= 0) return;
    historyIndex.current -= 1; change(copySettings(history.current[historyIndex.current])); settleHistory();
  };
  const redo = () => {
    settleHistory();
    if (historyIndex.current >= history.current.length - 1) return;
    historyIndex.current += 1; change(copySettings(history.current[historyIndex.current])); settleHistory();
  };

  useEffect(() => {
    cancelPreview(); setPreviewUrl(null); setBefore(false); setCropping(false); setZoom(1); setPan({ x: 0, y: 0 }); setError(null); setRecipeEdited(false);
    if (!selected) return;
    let active = true;
    void api.getDevelopRecipe(selected.id).then((recipe) => {
      if (!active) return;
      latest.current = recipe.settings; setSettings(recipe.settings); setRecipeEdited(JSON.stringify(recipe.settings) !== JSON.stringify(neutralAdjustments)); history.current = [copySettings(recipe.settings)]; historyIndex.current = 0;
      if (JSON.stringify(recipe.settings) !== JSON.stringify(neutralAdjustments)) requestPreview(selected, recipe.settings);
    }).catch((reason) => active && setError(String(reason)));
    return () => { active = false; };
  }, [cancelPreview, requestPreview, selected, selectedId]);

  useEffect(() => () => { if (groupTimer.current !== null) window.clearTimeout(groupTimer.current); if (saveTimer.current !== null) window.clearTimeout(saveTimer.current); }, []);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const element = event.target as HTMLElement | null;
      if (!selected || element?.matches("input,textarea,select") || element?.isContentEditable) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") { event.preventDefault(); event.shiftKey ? redo() : undo(); return; }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") { event.preventDefault(); copiedSettings = copySettings(latest.current); setHasClipboard(true); return; }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v") { event.preventDefault(); if (copiedSettings) change(copySettings(copiedSettings)); return; }
      if (event.code === "Backslash") { event.preventDefault(); setBefore((value) => !value); return; }
      if (event.key === "0") { event.preventDefault(); setZoom(1); setPan({ x: 0, y: 0 }); return; }
      if (event.key === "1") { event.preventDefault(); setZoom(1.75); setPan({ x: 0, y: 0 }); return; }
      if (event.key === "r") { event.preventDefault(); change(copySettings(neutralAdjustments)); return; }
      const index = assets.findIndex((asset) => asset.id === selected.id);
      if (event.key === "ArrowLeft" && assets[index - 1]) { event.preventDefault(); onSelect(assets[index - 1]); }
      if (event.key === "ArrowRight" && assets[index + 1]) { event.preventDefault(); onSelect(assets[index + 1]); }
    };
    window.addEventListener("keydown", handler); return () => window.removeEventListener("keydown", handler);
  }, [assets, selected, settings]);

  if (!selected) return <main className="view empty-state"><Eye size={42} /><h2>Choose a photograph to develop</h2></main>;
  const fallbackFilter = previewUrl || before ? undefined : { filter: `brightness(${Math.pow(2, settings.exposure)}) contrast(${1 + settings.contrast / 150}) saturate(${1 + settings.saturation / 100})` };
  const source = before ? selected.previewUrl : previewUrl || selected.previewUrl;
  return <main className="view develop-view">
    <section className="develop-stage" onWheel={(event) => { event.preventDefault(); setZoom((value) => Math.max(0.5, Math.min(4, value + (event.deltaY < 0 ? .15 : -.15)))); }}>
      <div className="develop-toolbar"><span className="eyebrow">Non-destructive Develop</span><strong>{selected.filename}</strong><span className={recipeEdited ? "edited-state" : "edited-state neutral"}>{recipeEdited ? "Edited" : "As shot"}</span></div>
      <div className="develop-canvas" onPointerDown={(event) => zoom > 1 && setDrag({ x: event.clientX, y: event.clientY, pan })} onPointerMove={(event) => drag && setPan({ x: drag.pan.x + event.clientX - drag.x, y: drag.pan.y + event.clientY - drag.y })} onPointerUp={() => setDrag(null)} onPointerLeave={() => setDrag(null)}>
        <img src={source} alt={selected.filename} draggable={false} style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`, ...fallbackFilter }} />
        {cropping && !before && zoom === 1 ? <CropOverlay value={settings} onChange={(part) => replace({ ...latest.current, ...part })} /> : null}
      </div>
      <div className="develop-actions"><button type="button" aria-pressed={before} onPointerDown={() => setBefore(true)} onPointerUp={() => setBefore(false)} onPointerLeave={() => setBefore(false)} onClick={() => setBefore((value) => !value)}><Eye size={15} /> Original <kbd>\</kbd></button><button type="button" onClick={undo} title="Undo Develop change"><Undo2 size={15} /> Undo</button><button type="button" onClick={redo} title="Redo Develop change"><Redo2 size={15} /> Redo</button><button type="button" onClick={() => { copiedSettings = copySettings(settings); setHasClipboard(true); }}><Clipboard size={15} /> Copy settings</button><button type="button" disabled={!hasClipboard} onClick={() => copiedSettings && change(copySettings(copiedSettings))}><ClipboardPaste size={15} /> Paste settings</button><button type="button" aria-pressed={cropping} onClick={() => setCropping((value) => !value)}>Crop</button><button type="button" onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }}><Maximize2 size={15} /> Fit <kbd>0</kbd></button><button type="button" onClick={() => setZoom(1.75)}><ZoomIn size={15} /> 100% <kbd>1</kbd></button><button type="button" onClick={() => setZoom((value) => Math.max(.5, value - .25))}><Minus size={15} /> Zoom</button></div>
    </section>
    <aside className="develop-panel">
      <div className="develop-panel-heading"><div><span className="eyebrow">Settings</span><h2>Develop</h2></div><button type="button" title="Reset all Develop settings" onClick={() => change(copySettings(neutralAdjustments))}><RotateCcw size={16} /> Reset all <kbd>R</kbd></button></div>
      <p className="develop-note">Edits are saved as a compact catalogue recipe. The original file is never changed.</p>
      <PhotoAdjustmentControls value={settings} onChange={(key, value) => change({ ...latest.current, [key]: value })} onReplace={replace} />
      {error ? <p className="develop-error" role="alert">Develop preview: {error}</p> : null}
      <button className="primary-button full" type="button" onClick={() => void onExport(selected)}>Export full-resolution edit</button>
      <p className="develop-shortcuts">Ctrl/Cmd+Z undo · Ctrl/Cmd+Shift+Z redo · Ctrl/Cmd+C/V copy/paste · \ original · 0 fit · 1 100%</p>
    </aside>
  </main>;
}

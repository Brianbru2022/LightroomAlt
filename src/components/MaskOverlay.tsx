import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { canonicalToDisplay, displayToCanonical, type MaskTool } from "../lib/masks";
import type { BasicAdjustments, DevelopMask, MaskPoint } from "../types";

type Props = {
  mask: DevelopMask | null;
  settings: BasicAdjustments;
  sourceAspect: number;
  tool: MaskTool;
  brush: { radius: number; feather: number; flow: number };
  showCoverage: boolean;
  onCommit: (mask: DevelopMask) => void;
  onCancel: () => void;
};

type Drag = { kind: "create" | "linear-start" | "linear-end" | "radial-centre" | "radial-x" | "radial-y" | "brush"; origin: MaskPoint; original: DevelopMask };
const pct = (value: number) => value * 100;

export function MaskOverlay({ mask, settings, sourceAspect, tool, brush, showCoverage, onCommit, onCancel }: Props) {
  const svg = useRef<SVGSVGElement>(null); const [draft, setDraft] = useState<DevelopMask | null>(mask); const draftRef = useRef<DevelopMask | null>(mask); const [drag, setDrag] = useState<Drag | null>(null); draftRef.current = draft;
  useEffect(() => { if (!drag) setDraft(mask); }, [mask, drag]);
  const fromEvent = (event: Pick<PointerEvent, "clientX" | "clientY">) => { const bounds = svg.current?.getBoundingClientRect(); if (!bounds) return { x: 0, y: 0 }; return displayToCanonical({ x: (event.clientX - bounds.left) / bounds.width, y: (event.clientY - bounds.top) / bounds.height }, settings, sourceAspect); };
  useEffect(() => {
    if (!drag) return;
    const move = (event: PointerEvent) => {
      const point = fromEvent(event); const next = structuredClone(draftRef.current ?? drag.original);
      if (drag.kind === "linear-start" && next.geometry.kind === "linear") next.geometry.start = point;
      if (drag.kind === "linear-end" && next.geometry.kind === "linear") next.geometry.end = point;
      if (drag.kind === "radial-centre" && next.geometry.kind === "radial") next.geometry.centre = point;
      if (drag.kind === "radial-x" && next.geometry.kind === "radial") next.geometry.radiusX = Math.max(.005, Math.abs(point.x - next.geometry.centre.x));
      if (drag.kind === "radial-y" && next.geometry.kind === "radial") next.geometry.radiusY = Math.max(.005, Math.abs(point.y - next.geometry.centre.y));
      if (drag.kind === "create" && next.geometry.kind === "linear") next.geometry.end = point;
      if (drag.kind === "create" && next.geometry.kind === "radial") { next.geometry.radiusX = Math.max(.005, Math.abs(point.x - drag.origin.x)); next.geometry.radiusY = Math.max(.005, Math.abs(point.y - drag.origin.y)); }
      if (drag.kind === "brush" && next.geometry.kind === "brush") {
        const stroke = next.geometry.strokes.at(-1); if (stroke) { const previous = stroke.points.at(-1); if (!previous || Math.hypot(point.x - previous.x, point.y - previous.y) > .002) stroke.points.push(point); }
      }
      draftRef.current = next; setDraft(next);
    };
    const up = () => { if (draftRef.current) onCommit(draftRef.current); setDrag(null); };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", up);
    return () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
  }, [drag, draft, onCommit, settings]);
  useEffect(() => { const escape = (event: KeyboardEvent) => { if (event.key === "Escape" && drag) { setDraft(mask); setDrag(null); onCancel(); } }; window.addEventListener("keydown", escape); return () => window.removeEventListener("keydown", escape); }, [drag, mask, onCancel]);
  if (!mask || !draft) return null;
  const begin = (kind: Drag["kind"]) => (event: ReactPointerEvent) => { event.preventDefault(); event.stopPropagation(); const point = fromEvent(event.nativeEvent); const original = structuredClone(draft);
    if (kind === "create" && original.geometry.kind === "linear") { original.geometry.start = point; original.geometry.end = { x: Math.min(1, point.x + .001), y: point.y }; }
    if (kind === "create" && original.geometry.kind === "radial") { original.geometry.centre = point; original.geometry.radiusX = .005; original.geometry.radiusY = .005; }
    if (kind === "brush" && original.geometry.kind === "brush") original.geometry.strokes.push({ points: [point], radius: brush.radius, feather: brush.feather, flow: brush.flow, erase: tool === "erase" });
    draftRef.current = original; setDraft(original); setDrag({ kind, origin: point, original });
  };
  const linear = draft.geometry.kind === "linear" ? draft.geometry : null; const radial = draft.geometry.kind === "radial" ? draft.geometry : null; const brushing = draft.geometry.kind === "brush" ? draft.geometry : null;
  const shown = (point: MaskPoint) => canonicalToDisplay(point, settings, sourceAspect);
  const start = linear ? shown(linear.start) : null; const end = linear ? shown(linear.end) : null; const centre = radial ? shown(radial.centre) : null;
  const radialPoint = (angle: number) => { if (!radial) return { x: 0, y: 0 }; const x = radial.radiusX * Math.cos(angle); const y = radial.radiusY * Math.sin(angle); const rotation = radial.rotation * Math.PI / 180; return shown({ x: radial.centre.x + Math.cos(rotation) * x - Math.sin(rotation) * y, y: radial.centre.y + Math.sin(rotation) * x + Math.cos(rotation) * y }); };
  const radialPoints = radial ? Array.from({ length: 48 }, (_, index) => { const point = radialPoint(index / 48 * Math.PI * 2); return `${pct(point.x)},${pct(point.y)}`; }).join(" ") : "";
  return <svg ref={svg} className={`mask-overlay ${tool ? "drawing" : ""}`} viewBox="0 0 100 100" preserveAspectRatio="none" aria-label="Active mask overlay" onPointerDown={tool === "linear" || tool === "radial" ? begin("create") : tool === "brush" || tool === "erase" ? begin("brush") : undefined}>
    <defs>{linear && start && end ? <linearGradient id={`gradient-${draft.id}`} gradientUnits="userSpaceOnUse" x1={pct(start.x)} y1={pct(start.y)} x2={pct(end.x)} y2={pct(end.y)}><stop offset="0" stopColor="#e95858" stopOpacity="0" /><stop offset="1" stopColor="#e95858" stopOpacity={showCoverage ? .48 : .18} /></linearGradient> : null}</defs>
    {linear ? <><rect width="100" height="100" fill={`url(#gradient-${draft.id})`} /><line x1={pct(start!.x)} y1={pct(start!.y)} x2={pct(end!.x)} y2={pct(end!.y)} /><circle className="mask-handle" cx={pct(start!.x)} cy={pct(start!.y)} r="1.4" onPointerDown={begin("linear-start")} /><circle className="mask-handle" cx={pct(end!.x)} cy={pct(end!.y)} r="1.4" onPointerDown={begin("linear-end")} /></> : null}
    {radial && centre ? <><polygon points={radialPoints} fill="#e95858" fillOpacity={showCoverage ? .38 : .14} /><polygon className="mask-outline" points={radialPoints} /><circle className="mask-handle" cx={pct(centre.x)} cy={pct(centre.y)} r="1.4" onPointerDown={begin("radial-centre")} /><circle className="mask-handle" cx={pct(radialPoint(0).x)} cy={pct(radialPoint(0).y)} r="1.4" onPointerDown={begin("radial-x")} /><circle className="mask-handle" cx={pct(radialPoint(Math.PI / 2).x)} cy={pct(radialPoint(Math.PI / 2).y)} r="1.4" onPointerDown={begin("radial-y")} /></> : null}
    {brushing ? brushing.strokes.map((stroke, index) => <polyline key={index} points={stroke.points.map((point) => { const value = shown(point); return `${pct(value.x)},${pct(value.y)}`; }).join(" ")} fill="none" stroke={stroke.erase ? "#74a7ff" : "#e95858"} strokeOpacity={showCoverage ? Math.max(.2, stroke.flow * .6) : .25} strokeWidth={stroke.radius * 200 / Math.max(settings.cropWidth, settings.cropHeight)} strokeLinecap="round" strokeLinejoin="round" />) : null}
  </svg>;
}

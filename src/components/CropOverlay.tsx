import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { BasicAdjustments } from "../types";

type Edge = "move" | "nw" | "ne" | "sw" | "se";
type Props = { value: BasicAdjustments; onChange: (next: Partial<BasicAdjustments>) => void };

export function CropOverlay({ value, onChange }: Props) {
  const frame = useRef<HTMLDivElement>(null); const [drag, setDrag] = useState<{ edge: Edge; x: number; y: number; state: BasicAdjustments } | null>(null);
  useEffect(() => {
    if (!drag) return;
    const move = (event: PointerEvent) => {
      const bounds = frame.current?.getBoundingClientRect(); if (!bounds) return;
      const dx = (event.clientX - drag.x) / bounds.width; const dy = (event.clientY - drag.y) / bounds.height;
      let { cropLeft: left, cropTop: top, cropWidth: width, cropHeight: height } = drag.state;
      if (drag.edge === "move") { left = Math.max(0, Math.min(1 - width, left + dx)); top = Math.max(0, Math.min(1 - height, top + dy)); }
      if (drag.edge.includes("w")) { left = Math.max(0, Math.min(left + width - .05, left + dx)); width = drag.state.cropWidth + drag.state.cropLeft - left; }
      if (drag.edge.includes("e")) width = Math.max(.05, Math.min(1 - left, width + dx));
      if (drag.edge.includes("n")) { top = Math.max(0, Math.min(top + height - .05, top + dy)); height = drag.state.cropHeight + drag.state.cropTop - top; }
      if (drag.edge.includes("s")) height = Math.max(.05, Math.min(1 - top, height + dy));
      onChange({ cropLeft: left, cropTop: top, cropWidth: width, cropHeight: height });
    };
    const up = () => setDrag(null); window.addEventListener("pointermove", move); window.addEventListener("pointerup", up); return () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
  }, [drag, onChange]);
  const start = (edge: Edge) => (event: ReactPointerEvent) => { event.preventDefault(); setDrag({ edge, x: event.clientX, y: event.clientY, state: value }); };
  return <div ref={frame} className="crop-overlay" aria-label="Crop frame"><div className="crop-frame" style={{ left: `${value.cropLeft * 100}%`, top: `${value.cropTop * 100}%`, width: `${value.cropWidth * 100}%`, height: `${value.cropHeight * 100}%` }} onPointerDown={start("move")}><i className="crop-handle nw" onPointerDown={start("nw")} /><i className="crop-handle ne" onPointerDown={start("ne")} /><i className="crop-handle sw" onPointerDown={start("sw")} /><i className="crop-handle se" onPointerDown={start("se")} /></div></div>;
}

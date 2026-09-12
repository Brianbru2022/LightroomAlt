import { Check, CheckCircle2, CircleHelp, Layers3, MapPin, SlidersHorizontal, TriangleAlert, X } from "lucide-react";
import type { Asset } from "../types";
import type { MouseEvent } from "react";
import { formatDate } from "../lib/format";

type Props = { asset: Asset; selected: boolean; active?: boolean; onSelect: (asset: Asset, event?: Pick<MouseEvent, "ctrlKey" | "metaKey" | "shiftKey">) => void; onOpen: (asset: Asset) => void };

const statusIcon = { keep: Check, undecided: CircleHelp, discard: X };

export function AssetCard({ asset, selected, active = selected, onSelect, onOpen }: Props) {
  const Status = statusIcon[asset.decision];
  return (
    <article className={`asset-card ${selected ? "selected" : ""} ${active ? "active" : ""}`} role="button" tabIndex={active ? 0 : -1} aria-pressed={selected} aria-selected={selected} aria-current={active ? "true" : undefined} aria-label={`${asset.filename}, ${asset.decision}`} title={`${asset.rating} stars${active ? " · active photograph" : ""}`} data-asset-id={asset.id} onClick={(event) => onSelect(asset, event)} onDoubleClick={() => onOpen(asset)} onKeyDown={(event) => { if (event.key === "Enter") onOpen(asset); else if (event.key === " ") { event.preventDefault(); onSelect(asset); } }}>
      <div className="asset-image-wrap">
        <img src={asset.preferredVersionUrl ?? asset.thumbnailUrl} alt={asset.filename} loading="lazy" />
        <span className={`decision-badge ${asset.decision}`} title={asset.decision}><Status size={14} /></span>
        {selected ? <span className="selection-check" aria-hidden="true"><CheckCircle2 size={17}/></span> : null}
        {asset.missingState && asset.missingState !== "available" ? <span className="pair-badge missing-badge" title={asset.missingState.replaceAll("_", " ")}><TriangleAlert size={13} /> Missing</span> : null}
        {asset.hasEdits ? <span className="pair-badge edited-badge" title="Non-destructive Develop settings applied"><SlidersHorizontal size={13} /> Edited</span> : null}
        {asset.representationCount > 1 ? <span className="pair-badge" title={`${asset.representationCount} paired files`}><Layers3 size={13} /> {asset.representationCount}</span> : null}
      </div>
      <div className="asset-card-copy">
        <strong title={asset.filename}>{asset.filename}</strong>
        <span>{"★".repeat(asset.rating)}{"☆".repeat(5-asset.rating)} · {formatDate(asset.capturedAt)} {asset.latitude !== undefined ? <MapPin size={12} /> : null}</span>
      </div>
    </article>
  );
}

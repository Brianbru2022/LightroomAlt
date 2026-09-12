import { Check, CircleHelp, Layers3, MapPin, SlidersHorizontal, TriangleAlert, X } from "lucide-react";
import type { Asset } from "../types";
import { formatDate } from "../lib/format";

type Props = { asset: Asset; selected: boolean; onSelect: (asset: Asset) => void; onOpen: (asset: Asset) => void };

const statusIcon = { keep: Check, undecided: CircleHelp, discard: X };

export function AssetCard({ asset, selected, onSelect, onOpen }: Props) {
  const Status = statusIcon[asset.decision];
  return (
    <article className={`asset-card ${selected ? "selected" : ""}`} role="button" tabIndex={selected ? 0 : -1} aria-current={selected ? "true" : undefined} aria-label={`${asset.filename}, ${asset.decision}`} data-asset-id={asset.id} onClick={() => onSelect(asset)} onDoubleClick={() => onOpen(asset)} onKeyDown={(event) => { if (event.key === "Enter") onOpen(asset); else if (event.key === " ") { event.preventDefault(); onSelect(asset); } }}>
      <div className="asset-image-wrap">
        <img src={asset.preferredVersionUrl ?? asset.thumbnailUrl} alt={asset.filename} loading="lazy" />
        <span className={`decision-badge ${asset.decision}`} title={asset.decision}><Status size={14} /></span>
        {asset.missingState && asset.missingState !== "available" ? <span className="pair-badge missing-badge" title={asset.missingState.replaceAll("_", " ")}><TriangleAlert size={13} /> Missing</span> : null}
        {asset.hasEdits ? <span className="pair-badge edited-badge" title="Non-destructive Develop settings applied"><SlidersHorizontal size={13} /> Edited</span> : null}
        {asset.representationCount > 1 ? <span className="pair-badge" title={`${asset.representationCount} paired files`}><Layers3 size={13} /> {asset.representationCount}</span> : null}
      </div>
      <div className="asset-card-copy">
        <strong title={asset.filename}>{asset.filename}</strong>
        <span>{formatDate(asset.capturedAt)} {asset.latitude !== undefined ? <MapPin size={12} /> : null}</span>
      </div>
    </article>
  );
}

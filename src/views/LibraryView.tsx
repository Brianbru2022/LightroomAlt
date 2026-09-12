import { CalendarDays, ChevronRight, ImageOff, LoaderCircle } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import type { Asset } from "../types";
import { AssetCard } from "../components/AssetCard";

type Props = {
  assets: Asset[];
  total: number;
  loading: boolean;
  hasMore: boolean;
  onLoadMore: () => void;
  selected: Asset | null;
  selectedIds?: ReadonlySet<string>;
  activeId?: string | null;
  onSelect: (asset: Asset, event?: Pick<MouseEvent, "ctrlKey" | "metaKey" | "shiftKey">) => void;
  onClearSelection?: () => void;
  onOpen: (asset: Asset) => void;
  mode?: "library" | "trash";
  onRestoreAll?: () => void;
  onEmptyTrash?: () => void;
};

type VirtualRow =
  | { key: string; kind: "year"; year: string; count: number; top: number; height: number }
  | { key: string; kind: "photos"; assets: Asset[]; columns: number; top: number; height: number };

export function LibraryView({ assets, total, loading, hasMore, onLoadMore, selected, selectedIds, activeId, onSelect, onClearSelection, onOpen, mode = "library", onRestoreAll, onEmptyTrash }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ width: 1000, height: 600, top: 0 });
  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const measure = () => setViewport((value) => ({ ...value, width: node.clientWidth || 1000, height: node.clientHeight || 600 }));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const { rows, height } = useMemo(() => {
    const columns = Math.max(1, Math.floor((viewport.width - 46) / 204));
    const usableWidth = Math.max(190, viewport.width - 60);
    const cardWidth = (usableWidth - Math.max(0, columns - 1) * 14) / columns;
    const photoHeight = Math.max(188, Math.round(cardWidth * 2 / 3 + 58));
    const grouped = new Map<string, Asset[]>();
    for (const asset of assets) {
      const year = new Date(asset.capturedAt).getFullYear().toString();
      const group = grouped.get(year);
      if (group) group.push(asset); else grouped.set(year, [asset]);
    }
    const nextRows: VirtualRow[] = [];
    let top = 0;
    for (const [year, yearAssets] of grouped) {
      nextRows.push({ key: `year-${year}`, kind: "year", year, count: yearAssets.length, top, height: 43 });
      top += 43;
      for (let index = 0; index < yearAssets.length; index += columns) {
        nextRows.push({ key: `${year}-${index}`, kind: "photos", assets: yearAssets.slice(index, index + columns), columns, top, height: photoHeight });
        top += photoHeight;
      }
    }
    return { rows: nextRows, height: top };
  }, [assets, viewport.width]);

  const visibleRows = useMemo(() => {
    const start = viewport.top - 500;
    const end = viewport.top + viewport.height + 500;
    return rows.filter((row) => row.top + row.height >= start && row.top <= end);
  }, [rows, viewport.height, viewport.top]);

  const onScroll = () => {
    const node = scrollRef.current;
    if (!node) return;
    setViewport((value) => ({ ...value, top: node.scrollTop }));
    if (hasMore && !loading && node.scrollTop + node.clientHeight >= node.scrollHeight - 900) onLoadMore();
  };

  return (
    <main className="view library-view">
      <div className="view-heading">
        <div><span className="eyebrow">{mode === "trash" ? "Reversible holding area" : "Master library"}</span><h1>{mode === "trash" ? "Trash" : "Photographs"}</h1><p>{total} visible · {assets.length < total ? `${assets.length} loaded · ` : ""}{mode === "trash" ? "catalogue records retained" : "originals protected"}</p></div>
        {mode === "trash" ? <div className="trash-actions"><button className="quiet-button" disabled={!total} onClick={onRestoreAll}>Restore all</button><button className="danger-button" disabled={!total} onClick={onEmptyTrash}>Empty Trash</button></div> : <div className="library-shortcuts" aria-label="Library keyboard shortcuts">
          <span><kbd>←↑↓→</kbd> Browse</span>
          <span><kbd>M</kbd> Keep</span>
          <span><kbd>X</kbd> Mark for removal</span>
        </div>}
      </div>
      {assets.length === 0 && !loading ? (
        <div className="empty-state"><ImageOff size={40} /><h2>{mode === "trash" ? "Trash is empty" : "No photographs match"}</h2><p>{mode === "trash" ? "Discarded photographs moved here can be restored before Empty Trash is used." : "Clear a filter or import a folder to begin."}</p></div>
      ) : (
        <div ref={scrollRef} className="library-scroll" onScroll={onScroll} onClick={event=>{if(event.target===event.currentTarget)onClearSelection?.();}}>
          <div className="virtual-library" role="listbox" aria-label="Photographs" aria-multiselectable="true" style={{ height }} onClick={event=>{if(event.target===event.currentTarget)onClearSelection?.();}}>
            {visibleRows.map((row) => row.kind === "year" ? (
              <div key={row.key} className="virtual-library-row year-heading" style={{ top: row.top, height: row.height }}><CalendarDays size={16} /><h2>{row.year}</h2><span>{row.count} loaded</span><ChevronRight size={15} /></div>
            ) : (
              <div key={row.key} className="virtual-library-row photo-grid" style={{ top: row.top, height: row.height, gridTemplateColumns: `repeat(${row.columns}, minmax(0, 1fr))` }}>
                {row.assets.map((asset) => <AssetCard key={asset.id} asset={asset} selected={selectedIds?.has(asset.id) ?? selected?.id === asset.id} active={(activeId ?? selected?.id) === asset.id} onSelect={onSelect} onOpen={onOpen} />)}
              </div>
            ))}
          </div>
          {loading ? <div className="catalogue-loading" role="status"><LoaderCircle className="spin" size={16} /> Loading photographs…</div> : null}
          {hasMore && !loading ? <button className="quiet-button load-more" onClick={onLoadMore}>Load more photographs</button> : null}
        </div>
      )}
    </main>
  );
}

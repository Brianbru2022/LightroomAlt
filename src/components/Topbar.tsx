import { FolderInput, Search, Trash2, Undo2 } from "lucide-react";
import type { Decision } from "../types";

type Props = {
  search: string;
  decision: Decision | "all";
  busy: boolean;
  onSearch: (value: string) => void;
  onDecision: (value: Decision | "all") => void;
  onImport: () => void;
  onUndo: () => void;
  discardCount: number;
  onDeleteAll: () => void;
  tags: string[];
  selectedTag?: string;
  onTag: (value?: string) => void;
  dateFrom?: string;
  dateTo?: string;
  onDateRange: (from?: string, to?: string) => void;
};

export function Topbar({ search, decision, busy, onSearch, onDecision, onImport, onUndo, discardCount, onDeleteAll, tags, selectedTag, onTag, dateFrom, dateTo, onDateRange }: Props) {
  return (
    <header className="topbar">
      <label className="search-field">
        <Search size={18} />
        <span className="sr-only">Search photographs</span>
        <input aria-label="Search photographs" value={search} onChange={(event) => onSearch(event.target.value)} placeholder="Search filename, camera or tag" />
        <kbd>Ctrl K</kbd>
      </label>
      <div className="segmented" aria-label="Triage filter">
        {(["all", "keep", "undecided", "discard"] as const).map((value) => (
          <button key={value} className={decision === value ? "selected" : ""} onClick={() => onDecision(value)}>{value === "all" ? "All" : value[0].toUpperCase() + value.slice(1)}</button>
        ))}
      </div>
      <details className="filter-menu"><summary>Filter</summary><div>
        <label>From<input aria-label="From date" type="date" value={dateFrom ?? ""} onChange={(event) => onDateRange(event.target.value || undefined, dateTo)} /></label>
        <label>To<input aria-label="To date" type="date" value={dateTo ?? ""} onChange={(event) => onDateRange(dateFrom, event.target.value || undefined)} /></label>
        <label>Tag<select aria-label="Tag filter" value={selectedTag ?? ""} onChange={(event) => onTag(event.target.value || undefined)}><option value="">All tags</option>{tags.map((tag) => <option key={tag} value={tag}>{tag}</option>)}</select></label>
      </div></details>
      {decision === "discard" ? <button className="danger-button" onClick={onDeleteAll} disabled={busy || discardCount === 0} aria-label={`Move all ${discardCount} discarded photographs to Trash`}><Trash2 size={17} /> Move to Trash ({discardCount})</button> : null}
      <button className="icon-button" onClick={onUndo} title="Undo last catalogue action" aria-label="Undo last catalogue action"><Undo2 size={18} /></button>
      <button className="primary-button" onClick={onImport} disabled={busy}><FolderInput size={17} /> {busy ? "Importing…" : "Import"}</button>
    </header>
  );
}

import { Aperture, Images, Map, ScanSearch, Settings, Sparkles, Trash2 } from "lucide-react";
import type { LibraryStatus, ServiceHealth, ViewName } from "../types";

const items: Array<{ id: ViewName; label: string; icon: typeof Images }> = [
  { id: "library", label: "Library", icon: Images },
  { id: "triage", label: "Triage", icon: ScanSearch },
  { id: "map", label: "Map", icon: Map },
  { id: "workshop", label: "AI Workshop", icon: Sparkles },
  { id: "trash", label: "Trash", icon: Trash2 },
  { id: "settings", label: "Settings & Health", icon: Settings },
];

type Props = { view: ViewName; status: LibraryStatus & ServiceHealth; onView: (view: ViewName) => void };

export function Sidebar({ view, status, onView }: Props) {
  return (
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark"><Aperture size={20} /></span><span>Keepframe</span></div>
      <nav aria-label="Main navigation">
        {items.map(({ id, label, icon: Icon }) => (
          <button key={id} className={`nav-item ${view === id ? "active" : ""}`} onClick={() => onView(id)} aria-current={view === id ? "page" : undefined}>
            <Icon size={19} strokeWidth={1.8} /><span>{label}</span>
          </button>
        ))}
      </nav>
      <div className="sidebar-spacer" />
      <section className="sidebar-stats" aria-label="Library summary">
        <div className="stats-title">This library</div>
        <div className="stat-row"><span>Keep</span><strong>{status.counts.keep}</strong></div>
        <div className="stat-row"><span>Undecided</span><strong>{status.counts.undecided}</strong></div>
        <div className="stat-row"><span>Discard</span><strong>{status.counts.discard}</strong></div>
        <div className="storage-rule" />
        <div className="library-path" title={status.libraryRoot}>{status.libraryRoot ?? "No library"}</div>
      </section>
      <div className="local-status" title={status.localAiDetail}><span className={`status-dot ${status.localAiAvailable ? "online" : ""}`} /> Local AI {status.localAiBusy ? "busy" : status.localAiAvailable ? "ready" : status.serviceReachable ? "unavailable" : "offline"}</div>
    </aside>
  );
}

import { Activity, Archive, FileHeart, HardDrive, RefreshCw, ShieldCheck } from "lucide-react";
import type { LibraryStatus, ServiceHealth } from "../types";

type Props = {
  status: LibraryStatus;
  health: ServiceHealth;
  busy: boolean;
  onIntegrity: () => void;
  onBackup: () => void;
  onRestore: () => void;
  onRebuild: () => void;
  onDiagnostics: () => void;
};

export function SettingsView({ status, health, busy, onIntegrity, onBackup, onRestore, onRebuild, onDiagnostics }: Props) {
  return <main className="view settings-view">
    <div className="view-heading"><div><span className="eyebrow">Maintenance and privacy</span><h1>Settings & Library Health</h1><p>Recovery tools act on the selected master library only.</p></div><span className="privacy-badge"><ShieldCheck size={16} /> Local only</span></div>
    <div className="settings-grid">
      <section className="settings-card"><HardDrive size={22} /><div><h2>Master library</h2><p>{status.libraryRoot}</p><small>A persistent library marker and exclusive lock protect this catalogue.</small></div></section>
      <section className="settings-card"><Activity size={22} /><div><h2>Catalogue integrity</h2><p>Run SQLite’s full integrity check without changing photographs.</p><button className="quiet-button" disabled={busy} onClick={onIntegrity}>Check now</button></div></section>
      <section className="settings-card"><Archive size={22} /><div><h2>Verified backup</h2><p>Creates a WAL-consistent backup and checks it before reporting success.</p><div className="settings-actions"><button className="quiet-button" disabled={busy} onClick={onBackup}>Create backup</button><button className="quiet-button" disabled={busy} onClick={onRestore}>Restore backup…</button></div></div></section>
      <section className="settings-card"><RefreshCw size={22} /><div><h2>Disposable previews</h2><p>Rebuild thumbnails from catalogued originals. Decisions, tags and locations are untouched.</p><button className="quiet-button" disabled={busy} onClick={onRebuild}>Rebuild thumbnails</button></div></section>
      <section className="settings-card"><FileHeart size={22} /><div><h2>Support diagnostics</h2><p>Exports version, schema, integrity and counts. No pixels, prompts or recipes are included.</p><button className="quiet-button" disabled={busy} onClick={onDiagnostics}>Export diagnostics…</button></div></section>
      <section className="settings-card"><ShieldCheck size={22} /><div><h2>Optional AI services</h2><p>{health.localAiDetail}</p><small>Catalogue and manual external-edit workflows continue when AI services are unavailable.</small></div></section>
    </div>
  </main>;
}

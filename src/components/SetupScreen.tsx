import { Aperture, Check, Folder, HardDrive, ShieldCheck } from "lucide-react";
import { useState } from "react";

type Props = { issue?: string; onChoose: () => Promise<string | null>; onCreate: (path: string) => Promise<void> };

export function SetupScreen({ issue, onChoose, onCreate }: Props) {
  const [path, setPath] = useState(""); const [busy, setBusy] = useState(false);
  const choose = async () => { const value = await onChoose(); if (value) setPath(value); };
  const create = async () => { if (!path) return; setBusy(true); try { await onCreate(path); } finally { setBusy(false); } };
  return <main className="setup-screen"><div className="setup-card"><div className="setup-brand"><span><Aperture size={25} /></span>Keepframe</div><span className="eyebrow">{issue ? "Library recovery" : "First-run setup"}</span><h1>{issue ? <>Reconnect your<br />master library.</> : <>Give your photographs<br />a safe home.</>}</h1><p>{issue ?? "Choose a master-library folder with enough room for your originals. Import defaults to Copy, so source photographs stay untouched."}</p><button className="folder-picker" onClick={choose}><Folder size={23} /><span>{path || (issue ? "Locate the existing master library" : "Choose master-library folder")}<small>{path ? "Ready to open" : "The folder may be on another drive"}</small></span></button><div className="setup-points"><span><ShieldCheck size={16} /> Originals protected</span><span><HardDrive size={16} /> Library stays portable</span><span><Check size={16} /> No subscription</span></div><button className="primary-button full large" disabled={!path || busy} onClick={create}>{busy ? "Opening library…" : issue ? "Reconnect library" : "Create or open library"}</button></div></main>;
}

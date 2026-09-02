import { Aperture, Check, Folder, HardDrive, ShieldCheck } from "lucide-react";
import { useState } from "react";

type Props = { onChoose: () => Promise<string | null>; onCreate: (path: string) => Promise<void> };

export function SetupScreen({ onChoose, onCreate }: Props) {
  const [path, setPath] = useState(""); const [busy, setBusy] = useState(false);
  const choose = async () => { const value = await onChoose(); if (value) setPath(value); };
  const create = async () => { if (!path) return; setBusy(true); try { await onCreate(path); } finally { setBusy(false); } };
  return <main className="setup-screen"><div className="setup-card"><div className="setup-brand"><span><Aperture size={25} /></span>Keepframe</div><span className="eyebrow">First-run setup</span><h1>Give your photographs<br />a safe home.</h1><p>Choose a master-library folder with enough room for your originals. Keepframe verifies every staged copy before moving the source into the library.</p><button className="folder-picker" onClick={choose}><Folder size={23} /><span>{path || "Choose master-library folder"}<small>{path ? "Ready to create" : "A large D: drive is ideal"}</small></span></button><div className="setup-points"><span><ShieldCheck size={16} /> Originals protected</span><span><HardDrive size={16} /> Library stays portable</span><span><Check size={16} /> No subscription</span></div><button className="primary-button full large" disabled={!path || busy} onClick={create}>{busy ? "Creating library…" : "Create Keepframe library"}</button></div></main>;
}

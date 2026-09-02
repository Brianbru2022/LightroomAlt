import { X } from "lucide-react";
import { useState } from "react";
import type { Asset } from "../types";

type Props = { asset: Asset; onClose: () => void; onSave: (tags: string[]) => void };
export function TagDialog({ asset, onClose, onSave }: Props) {
  const [value, setValue] = useState(asset.tags.join(", "));
  return <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}><section className="modal" role="dialog" aria-modal="true" aria-labelledby="tag-title"><button className="modal-close" onClick={onClose} aria-label="Close tag editor"><X size={18} /></button><span className="eyebrow">Catalogue metadata</span><h2 id="tag-title">Tags for {asset.filename}</h2><p>Separate tags with commas. Use a slash for hierarchy, for example <code>People/Hazel</code>.</p><input autoFocus value={value} aria-label="Comma-separated tags" onChange={(event) => setValue(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") onSave(value.split(",").map((item) => item.trim()).filter(Boolean)); }} /><button className="primary-button full" onClick={() => onSave(value.split(",").map((item) => item.trim()).filter(Boolean))}>Save tags</button></section></div>;
}

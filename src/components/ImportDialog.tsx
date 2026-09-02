import { Copy, MoveRight, ShieldCheck, X } from "lucide-react";
import { useState } from "react";
import type { ImportOptions } from "../types";

type Props = {
  sourceCount: number;
  onClose: () => void;
  onStart: (options: ImportOptions) => void;
};

export function ImportDialog({ sourceCount, onClose, onStart }: Props) {
  const [mode, setMode] = useState<ImportOptions["mode"]>("copy");
  const [duplicateSourcePolicy, setDuplicatePolicy] = useState<ImportOptions["duplicateSourcePolicy"]>("retain");
  return (
    <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="modal import-dialog" role="dialog" aria-modal="true" aria-labelledby="import-title">
        <button className="modal-close" onClick={onClose} aria-label="Close import choices"><X size={18} /></button>
        <span className="eyebrow">Safe managed import</span>
        <h2 id="import-title">How should Keepframe import?</h2>
        <p>{sourceCount} source {sourceCount === 1 ? "location is" : "locations are"} ready. Keepframe always stages, hashes and verifies each photograph.</p>
        <fieldset className="import-choices">
          <legend>Source handling</legend>
          <label className={mode === "copy" ? "selected" : ""}><input type="radio" name="import-mode" checked={mode === "copy"} onChange={() => setMode("copy")} /><Copy size={20} /><span><strong>Copy — safest</strong><small>Leave every source file exactly where it is.</small></span></label>
          <label className={mode === "move" ? "selected" : ""}><input type="radio" name="import-mode" checked={mode === "move"} onChange={() => setMode("move")} /><MoveRight size={20} /><span><strong>Move after verification</strong><small>Remove each source only after its managed copy and catalogue record are verified.</small></span></label>
        </fieldset>
        <label className="duplicate-choice"><input type="checkbox" checked={duplicateSourcePolicy === "remove_after_verified_match"} disabled={mode !== "move"} onChange={(event) => setDuplicatePolicy(event.target.checked ? "remove_after_verified_match" : "retain")} /><span><strong>Remove exact duplicate sources</strong><small>Off by default. Existing managed content must match byte-for-byte.</small></span></label>
        <div className="import-safety"><ShieldCheck size={18} /><span><strong>{mode === "copy" ? "Sources will be retained" : "Source removal is opt-in"}</strong><small>Unsupported or failed files are never removed.</small></span></div>
        <button className="primary-button full" onClick={() => onStart({ mode, duplicateSourcePolicy: mode === "copy" ? "retain" : duplicateSourcePolicy })}>Start {mode === "copy" ? "copy" : "verified move"}</button>
      </section>
    </div>
  );
}

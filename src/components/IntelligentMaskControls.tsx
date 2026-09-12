import type { IntelligentMaskCategory, IntelligentMaskHealth, IntelligentMaskProposal } from "../types";

type Props = {
  health: IntelligentMaskHealth | null;
  analysing: IntelligentMaskCategory | null;
  proposal: IntelligentMaskProposal | null;
  installing: boolean;
  progress: { receivedBytes: number; totalBytes: number } | null;
  onCreate: (category: IntelligentMaskCategory) => void;
  onAccept: () => void;
  onCancel: () => void;
  onInstall: () => void;
  onCancelInstall: () => void;
};

const label = (category: IntelligentMaskCategory) => category[0].toUpperCase() + category.slice(1);

export function IntelligentMaskControls({ health, analysing, proposal, installing, progress, onCreate, onAccept, onCancel, onInstall, onCancelInstall }: Props) {
  const available = Boolean(health?.available);
  return <div className="intelligent-mask-controls">
    <div className="mask-create" aria-label="Intelligent masks">
      {(["subject", "people", "sky"] as IntelligentMaskCategory[]).map((category) => <button type="button" key={category} disabled={!available || Boolean(analysing) || Boolean(proposal)} onClick={() => onCreate(category)}>{label(category)}</button>)}
    </div>
    {!health ? <p className="mask-provider-state">Checking local intelligent masking…</p> : !available ? <div className="mask-provider-state"><p>{health.detail}</p><small>{health.model} · {Math.round(health.approximateBytes / 1_000_000)} MB · {health.licence}<br />{health.storagePath}</small>{!health.installed ? <button type="button" disabled={installing} onClick={onInstall}>{installing ? "Installing…" : "Install 900 MB model"}</button> : null}{installing ? <><progress max={progress?.totalBytes ?? 1} value={progress?.receivedBytes ?? 0} /><button type="button" onClick={onCancelInstall}>Cancel installation</button></> : null}</div> : <p className="mask-provider-state">Local model ready · {health.executionProvider === "unloaded" ? "loads on first use" : health.executionProvider}</p>}
    {analysing ? <section className="intelligent-mask-proposal" aria-label="Intelligent mask analysis"><strong>Analysing {label(analysing)}…</strong><p>Processing locally. The current recipe has not changed.</p><button type="button" onClick={onCancel}>Cancel</button></section> : null}
    {proposal ? <section className="intelligent-mask-proposal" aria-label="Intelligent mask prediction"><strong>{label(proposal.category)} prediction</strong><p>{Math.round(proposal.confidence * 100)}% mean confidence · {Math.round(proposal.coverageFraction * 100)}% coverage · {proposal.elapsedMs} ms</p><small>Load {proposal.timings.loadMs} ms · preprocess {proposal.timings.preprocessMs} ms · inference {proposal.timings.inferenceMs} ms · cleanup {proposal.timings.postprocessMs} ms</small><div><button className="primary-button" type="button" onClick={onAccept}>Accept</button><button type="button" onClick={onCancel}>Cancel</button></div></section> : null}
  </div>;
}

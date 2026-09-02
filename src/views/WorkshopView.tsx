import { Check, ChevronDown, Clipboard, Cloud, Cpu, Download, FolderOutput, LoaderCircle, Play, RotateCcw, Sparkles, Upload } from "lucide-react";
import { useEffect, useState } from "react";
import type { Asset, AssetVersion, BatchJob, EditIntent, EditRecipe, PreserveConstraint, PromptSet, ServiceHealth } from "../types";
import { displayIntent } from "../lib/format";

const intents: EditIntent[] = ["restoration", "scratch_repair", "denoise", "sharpen", "upscale", "lighting_correction", "object_removal", "sky_replacement", "colourisation", "custom"];
const preservation: PreserveConstraint[] = ["identity_faces", "composition", "text", "period_detail", "skin_texture", "grain", "monochrome_tonality"];

type Props = {
  asset: Asset | null;
  assets: Asset[];
  jobs: BatchJob[];
  serviceHealth: ServiceHealth;
  onAnalyse: (asset: Asset, intent: EditIntent, brief?: string) => Promise<EditRecipe>;
  onPrompts: (recipe: EditRecipe) => Promise<PromptSet>;
  onCopy: (text: string) => Promise<void>;
  onPrepare: (assetId: string, provider: "chatgpt" | "gemini", prompt: string) => Promise<void>;
  onExportExternal: (assetId: string, provider: "chatgpt" | "gemini", prompt: string) => Promise<void>;
  onImportReturned: (assetId: string, provider: "chatgpt" | "gemini", prompt: string) => Promise<void>;
  onLoadVersions: (asset: Asset) => Promise<AssetVersion[]>;
  onSetPreferred: (assetId: string, versionId?: string) => Promise<void>;
  onReplace: (asset: Asset) => Promise<void>;
  onEnqueue: (assetIds: string[], brief: string) => Promise<void>;
  onRunLocal: (assetId: string, recipe: EditRecipe, prompt: string) => Promise<void>;
  onJob: (id: string, action: "approve" | "cancel" | "retry" | "accept" | "reject") => Promise<void>;
  onSaveJobReview: (id: string, recipe: EditRecipe, prompt: string) => Promise<void>;
  onApproveJobs: (ids: string[]) => Promise<void>;
};

function BatchRecipeReview({ job, onSave }: { job: BatchJob; onSave: Props["onSaveJobReview"] }) {
  const [observations, setObservations] = useState(job.recipe?.observations.join("\n") ?? "");
  const [constraints, setConstraints] = useState(job.recipe?.negativeConstraints.join("\n") ?? "");
  const [prompt, setPrompt] = useState(job.prompt);
  const [strength, setStrength] = useState<EditRecipe["strength"]>(job.recipe?.strength ?? "subtle");
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    setObservations(job.recipe?.observations.join("\n") ?? "");
    setConstraints(job.recipe?.negativeConstraints.join("\n") ?? "");
    setPrompt(job.prompt);
    setStrength(job.recipe?.strength ?? "subtle");
  }, [job.id, job.prompt, job.recipe]);
  if (!job.recipe) return <p className="job-error">No valid recipe was produced for this photograph.</p>;
  const save = async () => {
    const lines = (value: string) => value.split("\n").map((line) => line.trim()).filter(Boolean);
    const next = { ...job.recipe!, observations: lines(observations), negativeConstraints: lines(constraints), strength };
    setSaving(true);
    try { await onSave(job.id, next, prompt); } finally { setSaving(false); }
  };
  return (
    <div className="batch-review-editor">
      <label><span>Image observations</span><textarea rows={3} value={observations} onChange={(event) => setObservations(event.target.value)} /></label>
      <label><span>Negative constraints</span><textarea rows={3} value={constraints} onChange={(event) => setConstraints(event.target.value)} /></label>
      <label><span>Strength</span><select value={strength} onChange={(event) => setStrength(event.target.value as EditRecipe["strength"])}><option value="subtle">Subtle</option><option value="balanced">Balanced</option><option value="strong">Strong</option></select></label>
      <label><span>Final local prompt</span><textarea rows={5} value={prompt} onChange={(event) => setPrompt(event.target.value)} /></label>
      <button className="quiet-button" disabled={saving || !observations.trim() || !constraints.trim() || !prompt.trim()} onClick={save}><Check size={14} /> {saving ? "Saving…" : "Save this recipe"}</button>
    </div>
  );
}

export function WorkshopView({ asset, assets, jobs, serviceHealth, onAnalyse, onPrompts, onCopy, onPrepare, onExportExternal, onImportReturned, onLoadVersions, onSetPreferred, onReplace, onEnqueue, onRunLocal, onJob, onSaveJobReview, onApproveJobs }: Props) {
  const [intent, setIntent] = useState<EditIntent>("restoration");
  const [brief, setBrief] = useState("Restore naturally, retain character and make no unrequested changes.");
  const [recipe, setRecipe] = useState<EditRecipe | null>(null);
  const [prompts, setPrompts] = useState<PromptSet | null>(null);
  const [provider, setProvider] = useState<"local" | "chatgpt" | "gemini">("local");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [versions, setVersions] = useState<AssetVersion[]>([]);
  const [selectedVersionId, setSelectedVersionId] = useState<string>("");
  const refreshVersions = async (current: Asset) => {
    const next = await onLoadVersions(current);
    setVersions(next);
    setSelectedVersionId((selected) => next.some((item) => item.id === selected) ? selected : next.find((item) => item.isPreferred)?.id ?? next[0]?.id ?? "");
  };
  useEffect(() => {
    setRecipe(null); setPrompts(null); setVersions([]); setSelectedVersionId("");
    if (asset) void refreshVersions(asset);
  }, [asset?.id]);
  const selectedVersion = versions.find((item) => item.id === selectedVersionId) ?? versions[0];

  const analyse = async () => {
    if (!asset) return; setBusy(true);
    try { const next = await onAnalyse(asset, intent, brief); setRecipe(next); setPrompts(await onPrompts(next)); } finally { setBusy(false); }
  };
  const togglePreserve = async (value: PreserveConstraint) => {
    if (!recipe) return;
    const next = { ...recipe, preserve: recipe.preserve.includes(value) ? recipe.preserve.filter((item) => item !== value) : [...recipe.preserve, value] };
    setRecipe(next); setPrompts(await onPrompts(next));
  };
  const copy = async () => { if (!prompts) return; await onCopy(prompts[provider]); setCopied(true); window.setTimeout(() => setCopied(false), 1600); };

  return (
    <main className="view workshop-view">
      <section className="workshop-main">
        <div className="view-heading compact"><div><span className="eyebrow">Non-destructive workflow</span><h1>AI Workshop</h1><p>One source, one explicit recipe, every result traceable.</p></div></div>
        {!asset ? <div className="empty-state"><Sparkles size={42} /><h2>Select a photograph</h2><p>Choose a frame in the Library or Triage view first.</p></div> : (
          <>
            <div className="workbench">
              <div className="source-preview"><img src={asset.preferredVersionUrl ?? asset.previewUrl} alt={asset.filename} /><span>{asset.preferredVersionUrl ? "Preferred derived version" : "Protected original"}</span></div>
              <div className="workbench-controls">
                <label><span>What should change?</span><div className="select-wrap"><select value={intent} onChange={(event) => setIntent(event.target.value as EditIntent)}>{intents.map((item) => <option key={item} value={item}>{displayIntent(item)}</option>)}</select><ChevronDown size={15} /></div></label>
                <label><span>Your brief</span><textarea value={brief} onChange={(event) => setBrief(event.target.value)} rows={4} /></label>
                <button className="primary-button full" onClick={analyse} disabled={busy}>{busy ? <LoaderCircle className="spin" size={17} /> : <Sparkles size={17} />} {busy ? "Analysing this frame…" : "Build image-specific recipe"}</button>
                <small className="privacy-line"><Cpu size={13} /> Analysis is local; no photograph is uploaded.</small>
              </div>
            </div>
            <section className="version-panel" aria-labelledby="versions-heading">
              <div className="version-heading"><div><span className="step-number">V</span><div><h2 id="versions-heading">Versions & comparison</h2><p>Changing the displayed version never changes the photograph’s date, GPS, tags or decision.</p></div></div><button className="quiet-button" onClick={async () => { await onReplace(asset); await refreshVersions(asset); }}><Upload size={15} /> Replace with image</button></div>
              <div className="comparison-grid">
                <figure><img src={asset.previewUrl} alt={`Protected original ${asset.filename}`} /><figcaption>Protected original</figcaption></figure>
                <figure>{selectedVersion ? <img src={selectedVersion.imageUrl} alt={`${selectedVersion.kind} version of ${asset.filename}`} /> : <div className="comparison-empty">No derived version yet</div>}<figcaption>{selectedVersion ? `${displayIntent(selectedVersion.kind)} · ${selectedVersion.provider ?? "Keepframe"}` : "Select or import a version"}</figcaption></figure>
              </div>
              <div className="version-controls"><label><span>Compare version</span><select aria-label="Compare version" value={selectedVersionId} onChange={(event) => setSelectedVersionId(event.target.value)}>{versions.map((item) => <option key={item.id} value={item.id}>{item.isPreferred ? "Preferred · " : ""}{displayIntent(item.kind)} · {new Date(item.createdAt).toLocaleString()}</option>)}</select></label><button className="primary-button" disabled={!selectedVersion || selectedVersion.isPreferred} onClick={async () => { if (!selectedVersion) return; await onSetPreferred(asset.id, selectedVersion.kind === "original" ? undefined : selectedVersion.id); await refreshVersions(asset); }}>Make displayed version preferred</button></div>
            </section>
            {recipe && prompts ? (
              <div className="recipe-panel">
                <div className="recipe-heading"><div><span className="step-number">01</span><div><h2>Review the recipe</h2><p>{recipe.analysisModel} · {recipe.strength}</p></div></div><button className="icon-button" onClick={analyse} title="Analyse again"><RotateCcw size={16} /></button></div>
                <div className="observations">{recipe.observations.map((item) => <div key={item}><Check size={14} />{item}</div>)}</div>
                <h3>Preserve</h3><div className="choice-chips">{preservation.map((item) => <button key={item} className={recipe.preserve.includes(item) ? "selected" : ""} onClick={() => togglePreserve(item)}>{displayIntent(item)}</button>)}</div>
                <div className="prompt-stage">
                  <div className="prompt-tabs">{(["local", "chatgpt", "gemini"] as const).map((item) => <button key={item} className={provider === item ? "active" : ""} onClick={() => setProvider(item)}>{item === "local" ? <Cpu size={14} /> : <Cloud size={14} />}{item === "chatgpt" ? "ChatGPT" : displayIntent(item)}</button>)}</div>
                  <textarea value={prompts[provider]} onChange={(event) => setPrompts({ ...prompts, [provider]: event.target.value })} rows={7} aria-label={`${provider} prompt`} />
                  <div className="prompt-actions"><button className="quiet-button" onClick={copy}>{copied ? <Check size={16} /> : <Clipboard size={16} />} {copied ? "Copied" : "Copy prompt"}</button>{provider !== "local" ? <><button className="quiet-button" onClick={() => onPrepare(asset.id, provider, prompts[provider])}><Download size={16} /> Library export</button><button className="quiet-button" onClick={() => onExportExternal(asset.id, provider, prompts[provider])}><FolderOutput size={16} /> Export to folder</button><button className="quiet-button" onClick={() => onImportReturned(asset.id, provider, prompts[provider])}>Import result</button></> : null}{provider === "local" ? <button className="primary-button" disabled={!serviceHealth.localAiAvailable || serviceHealth.localAiBusy} onClick={() => onRunLocal(asset.id, recipe, prompts.local)}><Play size={16} /> Approve and start local edit</button> : null}</div>
                  {provider === "local" ? <p className={serviceHealth.localAiAvailable && !serviceHealth.localAiBusy ? "service-ready" : "local-warning"}>{serviceHealth.localAiDetail}</p> : null}
                </div>
              </div>
            ) : null}
          </>
        )}
      </section>
      <aside className="queue-panel">
        <div className="queue-heading"><div><span className="eyebrow">Persistent queue</span><h2>Batch desk</h2></div><span>{jobs.length}</span></div>
        <button className="quiet-button full" disabled={!assets.length} onClick={() => onEnqueue(assets.filter((item) => item.decision === "keep").map((item) => item.id), brief)}><Sparkles size={15} /> Queue all keepers</button>
        {jobs.some((job) => job.state === "review_required" && job.recipe) ? <button className="primary-button full" onClick={() => onApproveJobs(jobs.filter((job) => job.state === "review_required" && job.recipe).map((job) => job.id))}><Play size={15} /> Approve all reviewed</button> : null}
        <div className="job-list">{jobs.length ? jobs.map((job) => <article key={job.id} className="job-card"><div className="job-card-head"><strong>{job.assetName}</strong><span className={`job-state ${job.state}`}>{displayIntent(job.state)}</span></div>{job.outputUrl ? <img className="job-preview" src={job.outputUrl} alt={`Edited candidate for ${job.assetName}`} /> : null}{job.state === "analysing" ? <p className="job-progress"><LoaderCircle className="spin" size={14} /> Building this photograph’s recipe…</p> : null}{job.state === "review_required" ? <details className="batch-review"><summary>Review image-specific recipe</summary><BatchRecipeReview job={job} onSave={onSaveJobReview} /></details> : job.prompt ? <p>{job.prompt}</p> : null}{job.error ? <p className="job-error">{job.error}</p> : null}{job.attempts.length ? <details className="attempt-history"><summary>{job.attempts.length} {job.attempts.length === 1 ? "attempt" : "attempts"}</summary>{job.attempts.map((attempt) => <div key={attempt.attemptNumber}><strong>#{attempt.attemptNumber} · {displayIntent(attempt.state)}</strong><small>{new Date(attempt.startedAt).toLocaleString()}</small>{attempt.error ? <span>{attempt.error}</span> : null}</div>)}</details> : null}<div className="job-actions">{job.state === "review_required" && job.recipe ? <button onClick={() => onJob(job.id, "approve")}>Approve</button> : null}{job.state === "failed" && job.recipe ? <button onClick={() => onJob(job.id, "retry")}>Retry</button> : null}{job.state === "succeeded" ? <><button onClick={() => onJob(job.id, "accept")}>Accept</button><button onClick={() => onJob(job.id, "reject")}>Reject</button></> : null}{job.state === "review_required" ? <button onClick={() => onJob(job.id, "cancel")}>Exclude</button> : null}{["analysing", "queued", "running"].includes(job.state) ? <button onClick={() => onJob(job.id, "cancel")}>Cancel</button> : null}</div></article>) : <div className="queue-empty">No jobs yet. Build a recipe or queue the current keepers.</div>}</div>
        <div className="gpu-rule"><span className="status-dot online" /><div><strong>One GPU job at a time</strong><small>Queue survives application restarts.</small></div></div>
      </aside>
    </main>
  );
}

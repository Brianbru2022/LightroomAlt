import { Check, Clipboard, Cloud, Cpu, Download, LoaderCircle, Play, RotateCcw, SlidersHorizontal, Sparkles, SunMedium, Upload, WandSparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { neutralAdjustments, type AiAction, type Asset, type AssetVersion, type BasicAdjustments, type BatchJob, type EditIntent, type EditRecipe, type PreserveConstraint, type PromptSet, type ServiceHealth } from "../types";
import { displayIntent } from "../lib/format";
import { PhotoAdjustmentControls } from "../components/PhotoAdjustmentControls";
import { useAdjustmentPreview } from "../hooks/useAdjustmentPreview";

const preservation: PreserveConstraint[] = ["identity_faces", "composition", "text", "period_detail", "skin_texture", "grain", "monochrome_tonality"];

type Props = {
  asset: Asset | null;
  assets: Asset[];
  jobs: BatchJob[];
  serviceHealth: ServiceHealth;
  onAnalyse: (asset: Asset, intent: EditIntent, action?: AiAction, brief?: string) => Promise<EditRecipe>;
  onPrompts: (recipe: EditRecipe) => Promise<PromptSet>;
  onCopy: (text: string) => Promise<void>;
  onPrepare: (assetId: string, provider: "chatgpt" | "gemini", prompt: string) => Promise<void>;
  onExportExternal: (assetId: string, provider: "chatgpt" | "gemini", prompt: string) => Promise<void>;
  onImportReturned: (assetId: string, provider: "chatgpt" | "gemini", prompt: string, recipe?: EditRecipe) => Promise<void>;
  onLoadVersions: (asset: Asset) => Promise<AssetVersion[]>;
  onSetPreferred: (assetId: string, versionId?: string) => Promise<void>;
  onExport: (asset: Asset) => Promise<void>;
  onReplace: (asset: Asset) => Promise<void>;
  onAutoAdjustments: (assetId: string) => Promise<BasicAdjustments>;
  onPreviewAdjustments: (asset: Asset, adjustments: BasicAdjustments) => Promise<string>;
  onApplyAdjustments: (asset: Asset, adjustments: BasicAdjustments) => Promise<AssetVersion>;
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

export function WorkshopView({ asset, assets, jobs, serviceHealth, onAnalyse, onPrompts, onCopy, onPrepare, onExportExternal, onImportReturned, onLoadVersions, onSetPreferred, onExport, onReplace, onAutoAdjustments, onPreviewAdjustments, onApplyAdjustments, onEnqueue, onRunLocal, onJob, onSaveJobReview, onApproveJobs }: Props) {
  const actions: { id: AiAction; title: string; description: string; intent: EditIntent; brief: string }[] = [
    { id: "improve_photo", title: "Improve photo", description: "A careful overall tidy-up", intent: "denoise", brief: "Improve this photograph naturally. Retain its character and make no unrequested changes." },
    { id: "improve_lighting", title: "Improve lighting", description: "Balance light without an HDR look", intent: "lighting_correction", brief: "Improve the lighting and tonal balance naturally." },
    { id: "enhance_colour", title: "Enhance colour", description: "Restore believable colour and contrast", intent: "lighting_correction", brief: "Enhance natural colour and contrast without changing the scene." },
    { id: "restore_old_photo", title: "Restore old photo", description: "Repair age and fading conservatively", intent: "restoration", brief: "Restore visible age, fading, dust or damage while retaining authentic detail." },
    { id: "remove_distraction", title: "Remove distraction", description: "Tell Keepframe what to remove", intent: "object_removal", brief: "Remove only the distraction described below and reconstruct from surrounding evidence." },
    { id: "custom_instruction", title: "Custom instruction", description: "Describe one careful edit", intent: "custom", brief: "" },
  ];
  const [action, setAction] = useState<AiAction>("improve_photo");
  const [brief, setBrief] = useState(actions[0].brief);
  const [recipe, setRecipe] = useState<EditRecipe | null>(null);
  const [prompts, setPrompts] = useState<PromptSet | null>(null);
  const [provider, setProvider] = useState<"local" | "chatgpt" | "gemini">("local");
  const [busy, setBusy] = useState(false);
  const [showCompleted, setShowCompleted] = useState(false);
  const [copied, setCopied] = useState(false);
  const [versions, setVersions] = useState<AssetVersion[]>([]);
  const [selectedVersionId, setSelectedVersionId] = useState<string>("");
  const [adjustments, setAdjustments] = useState<BasicAdjustments>(neutralAdjustments);
  const [adjusting, setAdjusting] = useState<"auto" | "apply" | null>(null);
  const [adjustmentPreviewUrl, setAdjustmentPreviewUrl] = useState<string | null>(null);
  const adjustmentsRef = useRef(neutralAdjustments);
  adjustmentsRef.current = adjustments;
  const refreshVersions = async (current: Asset) => {
    const next = await onLoadVersions(current);
    setVersions(next);
    setSelectedVersionId((selected) => next.some((item) => item.id === selected) ? selected : next.find((item) => item.isPreferred)?.id ?? next[0]?.id ?? "");
  };
  const { request: requestAdjustmentPreview, cancel: cancelAdjustmentPreview } = useAdjustmentPreview(onPreviewAdjustments, setAdjustmentPreviewUrl);
  useEffect(() => {
    cancelAdjustmentPreview();
    adjustmentsRef.current = neutralAdjustments;
    setRecipe(null); setPrompts(null); setVersions([]); setSelectedVersionId(""); setAdjustments(neutralAdjustments); setAdjustmentPreviewUrl(null);
    if (asset) void refreshVersions(asset);
  }, [asset?.id, cancelAdjustmentPreview]);
  const selectedVersion = versions.find((item) => item.id === selectedVersionId) ?? versions[0];

  const selectedAction = actions.find((item) => item.id === action)!;
  const analyse = async () => {
    if (!asset) return; setBusy(true);
    try { const next = await onAnalyse(asset, selectedAction.intent, action, brief); setRecipe(next); setPrompts(await onPrompts(next)); } finally { setBusy(false); }
  };
  const togglePreserve = async (value: PreserveConstraint) => {
    if (!recipe) return;
    const next = { ...recipe, preserve: recipe.preserve.includes(value) ? recipe.preserve.filter((item) => item !== value) : [...recipe.preserve, value] };
    setRecipe(next); setPrompts(await onPrompts(next));
  };
  const updateRecipe = async (next: EditRecipe) => {
    setRecipe(next);
    setPrompts(await onPrompts(next));
  };
  const recipeLines = (value: string) => value.split("\n").map((line) => line.trim()).filter(Boolean);
  const copy = async () => { if (!prompts) return; await onCopy(prompts[provider]); setCopied(true); window.setTimeout(() => setCopied(false), 1600); };
  const setAdjustment = (key: keyof BasicAdjustments, value: number) => {
    if (!asset) return;
    const next = { ...adjustmentsRef.current, [key]: value };
    adjustmentsRef.current = next;
    setAdjustments(next);
    requestAdjustmentPreview(asset, next);
  };
  const autoAdjust = async () => {
    if (!asset) return;
    setAdjusting("auto");
    try {
      const next = await onAutoAdjustments(asset.id);
      adjustmentsRef.current = next;
      setAdjustments(next);
      cancelAdjustmentPreview();
      const url = await onPreviewAdjustments(asset, next);
      setAdjustmentPreviewUrl(url);
    } finally { setAdjusting(null); }
  };
  const applyAdjustments = async () => {
    if (!asset) return;
    setAdjusting("apply");
    try {
      const version = await onApplyAdjustments(asset, adjustments);
      await refreshVersions(asset);
      setSelectedVersionId(version.id);
    } finally { setAdjusting(null); }
  };
  return (
    <main className="view workshop-view">
      <section className="workshop-main">
        <div className="view-heading compact"><div><span className="eyebrow">Non-destructive workflow</span><h1>AI Workshop</h1><p>One source, one explicit recipe, every result traceable.</p></div></div>
        {!asset ? <div className="empty-state"><Sparkles size={42} /><h2>Select a photograph</h2><p>Choose a frame in the Library or Triage view first.</p></div> : (
          <>
            <div className="workbench">
              <div className="source-preview"><img src={adjustmentPreviewUrl || asset.preferredVersionUrl || asset.previewUrl} alt={asset.filename} /><span>{asset.preferredVersionUrl ? "Preferred derived version" : "Protected original"} · accurate adjustment preview</span></div>
              <div className="workbench-controls">
                <div><span className="field-label">What would you like done?</span><div className="ai-actions">{actions.map((item) => <button key={item.id} className={action === item.id ? "selected" : ""} onClick={() => { setAction(item.id); setBrief(item.brief); }}><strong>{item.title}</strong><small>{item.description}</small></button>)}</div></div>
                {(action === "custom_instruction" || action === "remove_distraction") ? <label><span>{action === "remove_distraction" ? "What should be removed?" : "What would you like changed?"}</span><textarea value={brief} onChange={(event) => setBrief(event.target.value)} rows={3} placeholder="Describe one specific, careful edit" /></label> : null}
                <button className="primary-button full" onClick={analyse} disabled={busy || !brief.trim()}>{busy ? <LoaderCircle className="spin" size={17} /> : <Sparkles size={17} />} {busy ? "Preparing your edit…" : "Continue"}</button>
                <small className={serviceHealth.analysisAvailable ? "privacy-line" : "local-warning"}><Cpu size={13} /> {serviceHealth.analysisAvailable ? "Optional vision analysis runs locally." : "A structured recipe will be created from your chosen action; " + serviceHealth.analysisDetail}</small>
              </div>
            </div>
            <section className="adjustment-panel" aria-labelledby="adjustments-heading">
              <div className="adjustment-heading"><div><span className="step-number"><SlidersHorizontal size={15} /></span><div><h2 id="adjustments-heading">Simple adjustments</h2><p>Preview changes here, then save them as a separate candidate version.</p></div></div><button className="quiet-button" disabled={adjusting !== null} onClick={() => { cancelAdjustmentPreview(); adjustmentsRef.current = neutralAdjustments; setAdjustments(neutralAdjustments); setAdjustmentPreviewUrl(null); }}><RotateCcw size={15} /> Reset</button></div>
              <PhotoAdjustmentControls value={adjustments} onChange={setAdjustment} />
              <div className="auto-range"><SunMedium size={20} /><div><strong>Protected maximum range</strong><span>Sets measured shadows and highlights near the clipping points, opens dark midtones and adds only a slight adaptive colour boost.</span></div><button className="primary-button" disabled={adjusting !== null} onClick={autoAdjust}>{adjusting === "auto" ? <LoaderCircle className="spin" size={16} /> : <WandSparkles size={16} />} Maximise range</button></div>
              <div className="adjustment-actions"><span>The preview and saved PNG use the same processing pipeline; the saved version is rendered from the full-resolution source.</span><button className="primary-button" disabled={adjusting !== null} onClick={applyAdjustments}>{adjusting === "apply" ? <LoaderCircle className="spin" size={16} /> : <Check size={16} />} Save candidate version</button></div>
            </section>
            <section className="version-panel" aria-labelledby="versions-heading">
              <div className="version-heading"><div><span className="step-number">V</span><div><h2 id="versions-heading">Versions & comparison</h2><p>Changing the displayed version never changes the photograph’s date, GPS, tags or decision.</p></div></div><div className="version-heading-actions"><button className="quiet-button" onClick={() => void onExport(asset)}><Download size={15} /> Export image</button><button className="quiet-button" onClick={async () => { await onReplace(asset); await refreshVersions(asset); }}><Upload size={15} /> Import replacement</button></div></div>
              <div className="comparison-grid">
                <figure><img src={asset.previewUrl} alt={`Protected original ${asset.filename}`} /><figcaption>Protected original</figcaption></figure>
                <figure>{selectedVersion ? <img src={selectedVersion.imageUrl} alt={`${selectedVersion.kind} version of ${asset.filename}`} /> : <div className="comparison-empty">No derived version yet</div>}<figcaption>{selectedVersion ? `${displayIntent(selectedVersion.kind)} · ${selectedVersion.provider ?? "Keepframe"}` : "Select or import a version"}</figcaption></figure>
              </div>
              <div className="version-controls"><label><span>Compare version</span><select aria-label="Compare version" value={selectedVersionId} onChange={(event) => setSelectedVersionId(event.target.value)}>{versions.map((item) => <option key={item.id} value={item.id}>{item.isPreferred ? "Preferred · " : ""}{displayIntent(item.kind)} · {new Date(item.createdAt).toLocaleString()}</option>)}</select></label><button className="primary-button" disabled={!selectedVersion || selectedVersion.isPreferred} onClick={async () => { if (!selectedVersion) return; await onSetPreferred(asset.id, selectedVersion.kind === "original" ? undefined : selectedVersion.id); await refreshVersions(asset); }}>Make displayed version preferred</button></div>
            </section>
            {recipe && prompts ? (
              <section className="ai-run-panel">
                <div className="recipe-heading"><div><span className="step-number">2</span><div><h2>Choose how to complete this edit</h2><p>{selectedAction.title} · the protected original will not be changed</p></div></div><button className="icon-button" onClick={analyse} title="Start again"><RotateCcw size={16} /></button></div>
                <div className="provider-choice">{(["local", "chatgpt", "gemini"] as const).map((item) => <button key={item} className={provider === item ? "active" : ""} onClick={() => setProvider(item)}>{item === "local" ? <Cpu size={15} /> : <Cloud size={15} />}<span>{item === "local" ? "On this computer" : item === "chatgpt" ? "ChatGPT hand-off" : "Gemini hand-off"}</span></button>)}</div>
                {provider === "local" ? <div className={serviceHealth.localAiAvailable && !serviceHealth.localAiBusy ? "provider-status ready" : "provider-status warning"}><strong>{serviceHealth.localAiAvailable ? "Local editor ready" : "Local editor unavailable"}</strong><span>{serviceHealth.localAiDetail}</span><button className="primary-button" disabled={!serviceHealth.localAiAvailable || serviceHealth.localAiBusy} onClick={() => onRunLocal(asset.id, recipe, prompts.local)}><Play size={16} /> Start local edit</button></div> : <div className="provider-status manual"><strong>Manual, explicit hand-off</strong><span>1. Prepare an sRGB PNG and instruction locally. 2. Upload them to {provider === "chatgpt" ? "ChatGPT" : "Gemini"} yourself. 3. Import the returned image below.</span><div className="prompt-actions"><button className="primary-button" onClick={() => onPrepare(asset.id, provider, prompts[provider])}><Download size={16} /> Prepare image and instruction</button><button className="quiet-button" onClick={copy}>{copied ? <Check size={16} /> : <Clipboard size={16} />} {copied ? "Instruction copied" : "Copy instruction"}</button><button className="quiet-button" onClick={() => onImportReturned(asset.id, provider, prompts[provider], recipe)}>Import returned image</button></div></div>}
                <details className="advanced-ai"><summary>Advanced recipe and provider instruction</summary>
              <div className="recipe-panel">
                <div className="recipe-heading"><div><span className="step-number">01</span><div><h2>Review the recipe</h2><p>{recipe.analysisModel} · {recipe.strength}</p></div></div><button className="icon-button" onClick={analyse} title="Analyse again"><RotateCcw size={16} /></button></div>
                <label><span>Analysis notes</span><textarea rows={4} value={recipe.observations.join("\n")} onChange={(event) => setRecipe({ ...recipe, observations: recipeLines(event.target.value) })} onBlur={(event) => void updateRecipe({ ...recipe, observations: recipeLines(event.currentTarget.value) })} /></label>
                <h3>Preserve</h3><div className="choice-chips">{preservation.map((item) => <button key={item} className={recipe.preserve.includes(item) ? "selected" : ""} onClick={() => togglePreserve(item)}>{displayIntent(item)}</button>)}</div>
                <div className="recipe-edit-grid"><label><span>Strength</span><select value={recipe.strength} onChange={(event) => void updateRecipe({ ...recipe, strength: event.target.value as EditRecipe["strength"] })}><option value="subtle">Subtle</option><option value="balanced">Balanced</option><option value="strong">Strong</option></select></label><label><span>Restrictions</span><textarea rows={4} value={recipe.negativeConstraints.join("\n")} onChange={(event) => setRecipe({ ...recipe, negativeConstraints: recipeLines(event.target.value) })} onBlur={(event) => void updateRecipe({ ...recipe, negativeConstraints: recipeLines(event.currentTarget.value) })} /></label></div>
                <div className="prompt-stage">
                  <div className="prompt-tabs">{(["local", "chatgpt", "gemini"] as const).map((item) => <button key={item} className={provider === item ? "active" : ""} onClick={() => setProvider(item)}>{item === "local" ? <Cpu size={14} /> : <Cloud size={14} />}{item === "chatgpt" ? "ChatGPT" : displayIntent(item)}</button>)}</div>
                  <textarea value={prompts[provider]} onChange={(event) => setPrompts({ ...prompts, [provider]: event.target.value })} rows={7} aria-label={`${provider} prompt`} />
                  <div className="prompt-actions"><button className="quiet-button" onClick={copy}>{copied ? <Check size={16} /> : <Clipboard size={16} />} {copied ? "Copied" : "Copy prompt"}</button>{provider !== "local" ? <button className="quiet-button" onClick={() => onExportExternal(asset.id, provider, prompts[provider])}>Export to folder</button> : null}</div>
                  {provider === "local" ? <p className={serviceHealth.localAiAvailable && !serviceHealth.localAiBusy ? "service-ready" : "local-warning"}>{serviceHealth.localAiDetail}</p> : <p className="privacy-line"><Cloud size={13} /> Keepframe prepares the image and prompt locally; it does not submit them to {provider === "chatgpt" ? "ChatGPT" : "Gemini"}.</p>}
                </div>
              </div></details>
              </section>
            ) : null}
          </>
        )}
      </section>
      <aside className="queue-panel">
        <div className="queue-heading"><div><span className="eyebrow">Persistent queue</span><h2>Batch desk</h2></div><span>{jobs.filter((job) => showCompleted || !["accepted", "rejected", "cancelled"].includes(job.state)).length}</span></div>
        <button className="quiet-button full" disabled={!assets.length} onClick={() => onEnqueue(assets.filter((item) => item.decision === "keep").map((item) => item.id), brief)}><Sparkles size={15} /> Queue all keepers</button>
        {jobs.some((job) => job.state === "review_required" && job.recipe) ? <button className="primary-button full" onClick={() => onApproveJobs(jobs.filter((job) => job.state === "review_required" && job.recipe).map((job) => job.id))}><Play size={15} /> Approve all reviewed</button> : null}
        <button className="quiet-button queue-filter" onClick={() => setShowCompleted((value) => !value)}>{showCompleted ? "Hide completed" : "Show completed"}</button>
        <div className="job-list">{jobs.filter((job) => showCompleted || !["accepted", "rejected", "cancelled"].includes(job.state)).length ? jobs.filter((job) => showCompleted || !["accepted", "rejected", "cancelled"].includes(job.state)).map((job) => <article key={job.id} className="job-card"><div className="job-card-head"><strong>{job.assetName}</strong><span className={`job-state ${job.state}`}>{displayIntent(job.state)}</span></div>{job.outputUrl ? <img className="job-preview" src={job.outputUrl} alt={`Edited candidate for ${job.assetName}`} /> : null}{job.state === "analysing" ? <p className="job-progress"><LoaderCircle className="spin" size={14} /> Building this photograph’s recipe…</p> : null}{job.state === "waiting_external" ? <p className="job-progress"><Cloud size={14} /> Waiting for the image you edit externally.</p> : null}{job.state === "review_required" ? <details className="batch-review"><summary>Review image-specific recipe</summary><BatchRecipeReview job={job} onSave={onSaveJobReview} /></details> : job.prompt ? <p>{job.prompt}</p> : null}{job.error ? <p className="job-error">{job.error}</p> : null}{job.attempts.length ? <details className="attempt-history"><summary>{job.attempts.length} {job.attempts.length === 1 ? "attempt" : "attempts"}</summary>{job.attempts.map((attempt) => <div key={attempt.attemptNumber}><strong>#{attempt.attemptNumber} · {displayIntent(attempt.state)}</strong><small>{new Date(attempt.startedAt).toLocaleString()}</small>{attempt.error ? <span>{attempt.error}</span> : null}</div>)}</details> : null}<div className="job-actions">{job.state === "review_required" && job.recipe ? <button onClick={() => onJob(job.id, "approve")}>Approve</button> : null}{job.state === "failed" && job.recipe ? <button onClick={() => onJob(job.id, "retry")}>Retry</button> : null}{job.state === "succeeded" ? <><button onClick={() => onJob(job.id, "accept")}>Accept</button><button onClick={() => onJob(job.id, "reject")}>Reject</button></> : null}{job.state === "review_required" ? <button onClick={() => onJob(job.id, "cancel")}>Exclude</button> : null}{["analysing", "queued", "running", "waiting_external"].includes(job.state) ? <button onClick={() => onJob(job.id, "cancel")}>Cancel</button> : null}</div></article>) : <div className="queue-empty">No active jobs. Completed items are hidden.</div>}</div>
        <div className="gpu-rule"><span className="status-dot online" /><div><strong>One GPU job at a time</strong><small>Queue survives application restarts.</small></div></div>
      </aside>
    </main>
  );
}

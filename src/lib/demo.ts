import type { Asset, BatchJob, Decision, EditRecipe, LibraryStatus, PromptSet } from "../types";

const svgPhoto = (title: string, sky: string, ground: string, accent: string, motif: string) => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="800" viewBox="0 0 1200 800">
    <defs><linearGradient id="s" x2="0" y2="1"><stop stop-color="${sky}"/><stop offset="1" stop-color="#f5dfba"/></linearGradient></defs>
    <rect width="1200" height="800" fill="url(#s)"/><path d="M0 510 Q210 420 410 500T820 470T1200 500V800H0Z" fill="${ground}"/>
    <circle cx="910" cy="165" r="72" fill="#fff4cf" opacity=".85"/><path d="M0 620 Q280 540 520 640T920 600T1200 650V800H0Z" fill="${accent}" opacity=".82"/>
    <g fill="#24382f" opacity=".85">${motif}</g><text x="70" y="715" fill="white" font-family="system-ui" font-size="35" font-weight="600">${title}</text>
  </svg>`;
  return `data:image/svg+xml;charset=UTF-8,${encodeURIComponent(svg)}`;
};

const scenes = [
  svgPhoto("West Sands · April 2025", "#88aebd", "#647660", "#c99554", '<path d="M175 520l55-180 55 180zM235 520l70-235 70 235z"/>'),
  svgPhoto("Elie Harbour · September 2024", "#7393a8", "#435d5d", "#a87854", '<rect x="190" y="390" width="150" height="150"/><path d="M170 400l95-75 95 75z"/>'),
  svgPhoto("Hazel at the shore · June 2024", "#b1c7c2", "#64715a", "#c9a56e", '<circle cx="310" cy="390" r="45"/><path d="M255 560q55-170 110 0z"/>'),
  svgPhoto("Snow on Lomond Hills · January 2024", "#aebcc6", "#5b6b69", "#dce1dc", '<path d="M110 550l220-340 210 340zM390 550l180-280 190 280z"/>'),
  svgPhoto("Crail evening · August 2023", "#d18e78", "#5b6556", "#8e6659", '<rect x="220" y="410" width="55" height="130"/><rect x="290" y="370" width="70" height="170"/>'),
  svgPhoto("Family archive · circa 1968", "#b7a78b", "#6d6555", "#8d785b", '<circle cx="300" cy="390" r="42"/><path d="M250 560q50-150 100 0z"/><circle cx="430" cy="400" r="38"/><path d="M385 560q45-140 90 0z"/>'),
];

export const demoAssets: Asset[] = scenes.map((previewUrl, index) => ({
  id: `asset-${index + 1}`,
  filename: ["DSC_1842.NEF", "IMG_7721.CR3", "P1060420.RW2", "DSC_0904.ARW", "IMG_4128.HEIC", "scan_0064.tif"][index],
  sourcePath: `D:\\Photo Library\\Originals\\${["2025\\04\\18", "2024\\09\\07", "2024\\06\\22", "2024\\01\\14", "2023\\08\\03", "1968\\07\\01"][index]}`,
  previewUrl,
  thumbnailUrl: previewUrl,
  decision: (["keep", "undecided", "keep", "discard", "undecided", "keep"] as Decision[])[index],
  capturedAt: ["2025-04-18T14:21:00Z", "2024-09-07T19:12:00Z", "2024-06-22T11:05:00Z", "2024-01-14T10:42:00Z", "2023-08-03T20:03:00Z", "1968-07-01T12:00:00Z"][index],
  dateFallback: index === 5,
  camera: ["Nikon Z8", "Canon EOS R5", "Panasonic S5 II", "Sony α7R V", "iPhone 15 Pro", "Epson V850 scan"][index],
  width: 8256,
  height: 5504,
  latitude: index === 5 ? undefined : [56.3398, 56.1918, 56.2114, 56.2451, 56.2608][index],
  longitude: index === 5 ? undefined : [-2.8092, -2.8211, -2.9351, -3.0122, -2.6266][index],
  tags: [["Scotland", "Coast"], ["Fife", "Harbour"], ["People/Hazel", "Beach"], ["Landscape", "Winter"], ["Fife", "Evening"], ["Archive", "Family"]][index],
  representationCount: index < 4 ? 2 : 1,
}));

export const demoStatus = (assets = demoAssets): LibraryStatus => ({
  configured: true,
  libraryRoot: "D:\\Photo Library",
  counts: {
    total: assets.length,
    keep: assets.filter((a) => a.decision === "keep").length,
    undecided: assets.filter((a) => a.decision === "undecided").length,
    discard: assets.filter((a) => a.decision === "discard").length,
  },
});

export const makeRecipe = (asset: Asset, intent: EditRecipe["intents"][number] = "restoration"): EditRecipe => ({
  schemaVersion: 1,
  assetId: asset.id,
  observations: asset.tags.includes("Archive")
    ? ["Fine dust and scratches are visible across the upper half.", "The faces are slightly soft but recognisable.", "The monochrome tonal range is compressed."]
    : ["Highlights are slightly flat.", "Fine detail would benefit from restrained sharpening.", "The composition and subject identity should remain unchanged."],
  intents: [intent],
  preserve: asset.tags.includes("Archive")
    ? ["identity_faces", "composition", "period_detail", "grain", "monochrome_tonality"]
    : ["identity_faces", "composition", "skin_texture"],
  negativeConstraints: ["Do not reshape faces.", "Do not invent objects, text or jewellery.", "Avoid plastic skin and excessive sharpening."],
  strength: "subtle",
  output: { format: "png", preserveDimensions: true, colourSpace: "sRGB" },
  analysisModel: "deterministic-fallback",
  analysisCreatedAt: new Date().toISOString(),
});

export const renderPrompts = (recipe: EditRecipe): PromptSet => {
  const instructions: Record<EditRecipe["intents"][number], string> = {
    restoration: "Restore only visible age, fading, dust or damage while retaining authentic photographic detail.",
    scratch_repair: "Remove visible scratches, dust marks and small surface defects; reconstruct only from neighbouring evidence.",
    denoise: "Reduce distracting noise without smearing faces, edges, texture or natural grain.",
    sharpen: "Apply restrained, edge-aware sharpening without halos or invented detail.",
    upscale: "Increase usable resolution while preserving identity, geometry and believable detail.",
    lighting_correction: "Correct exposure, contrast and colour balance naturally without an HDR look.",
    object_removal: "Remove only the object identified in the user brief and fill the area consistently.",
    sky_replacement: "Replace only the sky described in the user brief and match the scene lighting and horizon.",
    colourisation: "Colourise plausibly while preserving tonal structure and period detail.",
    custom: "Apply only the change explicitly described in the user brief.",
  };
  const requestedWork = recipe.intents.map((intent) => instructions[intent]).join(" ");
  const observations = recipe.observations.join(" ");
  const preserve = recipe.preserve.join(", ").replaceAll("_", " ");
  const constraints = recipe.negativeConstraints.join(" ");
  const brief = recipe.commonBrief?.trim() || "No additional user brief was supplied; do not make changes beyond the selected intent.";
  const core = `Use the attached photograph as the sole visual source. Goal: ${brief} Selected editing instructions: ${requestedWork} Relevant notes: ${observations} Preserve: ${preserve}. Restrictions: ${constraints} Editing strength: ${recipe.strength}. Preserve the original composition and dimensions. Output one colour-managed sRGB PNG.`;
  return {
    local: `Qwen Image Edit instruction. ${core} Make local, targeted changes only. Retain natural photographic texture and leave unaffected areas unchanged.`,
    chatgpt: `Edit the attached photograph rather than generating a replacement scene. ${core} Inspect the image itself before editing and return only the finished photograph.`,
    gemini: `Perform a faithful image edit on the attached photograph. ${core} Maintain subject and scene consistency; do not add unrequested generative content.`,
    negative: recipe.negativeConstraints.join(", "),
  };
};

export const demoJobs: BatchJob[] = [
  { id: "job-1", batchId: "batch-1", assetId: "asset-6", assetName: "scan_0064.tif", state: "review_required", prompt: "Restore archival photograph", recipe: makeRecipe(demoAssets[5], "restoration"), attempts: [] },
  { id: "job-2", batchId: "batch-2", assetId: "asset-2", assetName: "IMG_7721.CR3", state: "queued", prompt: "Correct light and retain harbour detail", recipe: makeRecipe(demoAssets[1], "lighting_correction"), attempts: [] },
];

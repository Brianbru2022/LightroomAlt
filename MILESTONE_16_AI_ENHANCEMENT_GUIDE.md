# Milestone 16 — AI Denoise, Super Resolution and derived masters

## Product boundary

AI Enhance is a local, explicit, two-stage workflow in Develop. **Preview crop** processes a movable 25% review selection capped at 1024 × 1024 source pixels and never changes the catalogue. The paired comparison has a 100–200% review zoom. **Apply full enhancement** re-decodes the authoritative full-resolution source, processes every tile, validates a staged lossless file, then creates a new physical source and primary catalogue item. The protected original is never overwritten. This milestone does not add generative detail synthesis, cloud processing, face enhancement, or a Milestone 17 feature.

Virtual versions remain lightweight recipes over one physical source. An accepted AI result is different: it is an immutable managed pixel master beneath `.keepframe/derivatives/ai/<parent-source-id>/`. It has a new source id, representation hash, primary item and thumbnail, plus parent and root-source lineage. The selected Develop recipe, including accepted masks, is copied to the new item so adjustments remain editable downstream. Re-generation creates another child; it never mutates an earlier result.

## Models and runtime

The selected denoiser is **SCUNet colour real PSNR**, using upstream code revision `52e440a80a655b01e0b41e9dd9bfe599bc11625e` and KAIR weights release v1.0 under Apache-2.0. Its checkpoint is pinned to 71,982,841 bytes and SHA-256 `fa78899ba2caec9d235a900e91d96c689da71c42029230c2028b00f09f809c2e`. The selected super-resolution model is **Real-ESRGAN x4plus**, using upstream code revision `a4abfb2979a7bbff3f69f58f58ae324608821e27` and weights release v0.1.0 under BSD-3-Clause. Its checkpoint is pinned to 67,040,989 bytes and SHA-256 `4fa0d38905f75ac06eb49a7951b426670021be3018265fd191d2125df9d682f1`.

The qualification record is:

| Operation | Candidate artefact | Evaluated revision | Licence and weight gate | Size | Inference path | Decision |
| --- | --- | --- | --- | ---: | --- | --- |
| Denoise | SCUNet `scunet_color_real_psnr.pth` | code `52e440a80a655b01e0b41e9dd9bfe599bc11625e`, weights KAIR v1.0 | Apache-2.0 project/release | 71,982,841 bytes | PyTorch through Spandrel; CUDA FP16/CPU FP32; reflect padding and external tiled composition | Selected and pinned. It targets real colour-image noise, fits the existing runtime and has a compact verified artefact. |
| Denoise | SwinIR `005_colorDN_DFWB_s128w8_SwinIR-M_noise25.pth` | code `6545850fbf8df298df73d81f3e8cba638787c8bd`, weights release v0.0 | Apache-2.0 project; no separately stated checkpoint redistribution grant was found | 122,905,743 bytes | Native PyTorch/CUDA or CPU; upstream exposes tile overlap | Rejected for this release: fixed-sigma synthetic-noise semantics are a poorer product fit, the checkpoint is larger, and the weight-rights statement is less explicit than the selected release. |
| Denoise | Restormer `real_denoising.pth` | code `68dc6ac472db26f16361150cb7a96a1bc87da93f`, upstream Drive checkpoint last modified 2024-03-22 | MIT code; no separate checkpoint redistribution grant was found | 104,611,957 bytes | Native PyTorch/CUDA or CPU; would need a new adapter and independent tiler qualification | Rejected: no existing Spandrel adapter, higher integration cost, and insufficiently explicit weight redistribution terms for an application-managed install. |
| Super resolution | Real-ESRGAN `RealESRGAN_x4plus.pth` | code `a4abfb2979a7bbff3f69f58f58ae324608821e27`, weights v0.1.0 | BSD-3-Clause project/release | 67,040,989 bytes | PyTorch through Spandrel; CUDA FP16/CPU FP32; native 4× with deterministic Lanczos to 2× | Selected and pinned. It is directly supported by the runtime and cleanly exposes one native learned scale. |
| Super resolution | SwinIR `003_realSR_BSRGAN_DFO_s64w8_SwinIR-M_x4_GAN.pth` | code `6545850fbf8df298df73d81f3e8cba638787c8bd`, weights release v0.0 | Apache-2.0 project; no separately stated checkpoint redistribution grant was found | 67,129,861 bytes | Native PyTorch/CUDA or CPU; upstream recommends tiled inference with overlap | Rejected: equivalent scope but a new runtime adapter and weaker standalone weight-rights evidence than the selected model. |
| Detail | No model | Not applicable | Not applicable | 0 bytes | Existing deterministic Detail controls | Rejected as a separate AI operation because it would duplicate sharpening/SR semantics and increase invented-detail risk. |

Nothing downloads automatically. Each model has its own explicit install and cancellable progress. Temporary `.partial` files are not promoted until exact size and SHA-256 pass. Weights remain under `D:\AI Models\Keepframe\enhancement`; the Python runtime remains under `D:\AI Models\Keepframe\runtime`. Catalogue exports never embed model weights.

CUDA uses FP16 and CPU uses FP32. GPU is preferred. The application selects a conservative tile size from reported free VRAM: denoise 512/384/256 px and super resolution 256/192/128 px across high/medium/low tiers; CPU uses 192/96 px. On CUDA out-of-memory, the current attempt is abandoned, the cache is cleared and the tile size is halved at a safe boundary up to three times. CPU fallback occurs only when the user explicitly enables it. AI work shares the existing single GPU gate with masks and semantic indexing.

## Processing and pixel contract

The authoritative renderer opens the original or current derived source at full resolution, including the existing LibRaw path where applicable. Denoise is 1×. Super Resolution uses the native 4× model; a requested 2× result is deterministically downsampled with Lanczos per tile before composition. A 120-million-output-pixel cap and a preflight free-space estimate fail before inference.

Tiles overlap by 32 source pixels for denoise and 16 for super resolution. Cosine/sine-squared edge weights form a partition of unity; boundaries and partial edge tiles are covered exactly. Progress reports real completed tiles and named phases. Cancellation is checked between tiles and before acceptance. Preview and final Apply use the same tiler and model path.

Accepted files are lossless **8-bit sRGB PNG**. This is deliberate and honest: the current authoritative renderer exposes RGB8, so this build does not claim a 16-bit linear or wide-gamut derivative. Every record stores operation, provider/version, model/revision/SHA-256, execution provider, scale, tile size, overlap, source and output hashes, dimensions, pixel format/bit depth, parameters, timings and provenance schema version.

## Catalogue, export and recovery

Downstream Develop, masks, virtual versions, thumbnails and professional export treat the derivative representation as the new item's authoritative source. Export therefore re-renders the accepted derived pixels with that item's current recipe; it does not require the model or silently re-run inference. XMP sidecar export is deliberately refused for AI-derived sources so catalogue-only AI provenance is never written beside the managed pixel file.

Portable catalogue schema 5 serialises derivative lineage and complete provenance, while excluding rebuildable semantic vectors and model weights. Import validates all ids, hashes, dimensions, enums, parameters and the managed `.keepframe/derivatives/ai/` boundary before one transaction. Moved-library recovery can rebase verified derivative representations by exact hash. Original relinking does not rewrite child lineage.

Rescan reports missing or modified AI derivatives separately from missing or modified protected originals. It also reports missing parents, unsupported model provenance and untracked managed derivative files; it never silently regenerates or deletes them. Interrupted non-terminal enhancement jobs become failed on restart, and uncommitted staging remains outside the catalogue for conservative inspection. Deletion is explicit and recoverable: the managed file is first moved under `.keepframe/Trash/ai`, then the derived source is removed transactionally; the parent remains.

Semantic indexing, search and duplicate discovery exclude derived sources and continue using the root source identity by default. This avoids duplicate results and unnecessary embeddings while preserving explicit lineage for future policy changes.

## Verification and profiling

Automated tests cover model metadata and installation validation, invalid operations/scales, exact 2×/4× dimensions, tile edge coverage, identity composites over gradients/checkerboards/diagonals/textures, accepted-source lineage, recipe preservation, original hash preservation, semantic exclusion and safe deletion. Portable schema, integrity, XMP, catalogue migration, renderer, export, Library, organisation, semantic discovery and earlier UI suites remain in the full gate.

Run the worker unit suite with a provisioned runtime:

```powershell
$env:PYTHONPATH = (Resolve-Path .\ai-worker)
& 'D:\AI Models\Keepframe\runtime\Scripts\python.exe' -m unittest discover -s .\ai-worker\tests -v
```

Run the complete repository gate:

```powershell
pnpm check:all
```

For native acceptance use legal, non-private JPEG/TIFF and camera-specific RAW fixtures. Record cold and warm model load, preprocess, inference, postprocess, write/validation, total time, peak VRAM/RAM, chosen tile size and retry state for denoise and both 2×/4× output paths. Inspect fine natural texture, flat noise, hard diagonals, high-contrast edges, faces, foliage, text and every tile boundary at 100–200%. Confirm exact output dimensions, output/source hashes, parent/root lineage, recipe/mask retention, export after worker shutdown, reopen after restart, portable round-trip, moved-library rebasing, missing/modified derivative findings, cancellation at each phase, low disk, GPU OOM retry and explicit CPU fallback. Do not claim camera-specific RAW or broad real-world quality until those native fixtures pass.

## Current limits

- Model-backed output quality is image-dependent; users must review the crop and the accepted full image.
- Preview selection is a fixed 25% normalised region, capped at 1024 × 1024 source pixels, with horizontal and vertical positioning rather than a freeform loupe box.
- CPU processing can be very slow and is never an automatic fallback.
- A 2× output still incurs native 4× inference before deterministic downsampling.
- AI provenance is catalogue/portable-catalogue metadata, not general XMP interoperability.
- The derivative is RGB8 sRGB PNG; 16-bit, HDR and wide-gamut masters remain out of scope.
- Native checks on representative redistributable real photographs and multiple camera RAW families remain a release acceptance responsibility.

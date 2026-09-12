# Milestone 10 renderer performance guide

## Scope and safety boundary

Milestone 10 optimises the existing Rust Develop renderer. It does not add editing controls, change recipe schema v2, change catalogue schema v9, alter mask order, weaken full-resolution validation, or write to a protected original. Preview artefacts and in-memory caches remain rebuildable derivatives; full export still decodes the protected representation and writes a new lossless sRGB PNG.

## Reproducible checkpoint

Run the opt-in checkpoint from the repository root:

```powershell
.\scripts\benchmark-renderer.ps1
```

The deterministic fixture is generated locally from a fixed pixel function, encoded as JPEG, decoded through the real image path, rendered at 720×480, and expanded to 2400×1600 for the export checkpoint. Timings use `std::time::Instant` around actual renderer stages. Performance tests are ignored by the authoritative suite so variable CI hosts cannot create false functional failures.

Machine used for the recorded checkpoint: Windows, AMD Radeon integrated graphics, NVIDIA GeForce RTX 5090, and a CPU exposing AVX2/AVX-512 when compiled for the native target. Measurements are single-run checkpoints rather than universal hardware guarantees.

## Before and after

The before figures were captured at commit `2b6e1a34c419b8ae2f9dba3d03da065849f81e28`, before implementation. The after figures use the same checkout, fixture families, and test profile after implementation.

| Operation | Before | After | Target |
| --- | ---: | ---: | ---: |
| Global 720×480 preview | 1,440 ms | 81 ms cold / 1.2 ms warm | under 250 ms |
| One geometric mask | 290 ms | 9 ms | under 300 ms |
| Five geometric masks | 871 ms | 37 ms | under 800 ms |
| Ten geometric masks | 1,858 ms | 64 ms | under 1,500 ms |
| Semantic preparation, 640×427 to 720×480 | 152 ms | 12 ms | under 500 ms practical target |
| Accepted semantic preview | 416 ms | 22 ms | under 500 ms practical target |
| 2400×1600 render plus lossless PNG | 8,506 ms | 1,150 ms | under 3,000 ms |

The detailed after checkpoint recorded: JPEG decode 10.5 ms cold and 0.069 ms cached; orientation plus decode 2.1 ms; RGB working-buffer conversion 0.043 ms; tone/exposure 4.3 ms; white balance 4.2 ms; Texture/Clarity/Dehaze 113.9 ms; colour 4.4 ms; one mask raster 1.4 ms; cached mask render 11.6 ms cold and 5.4 ms warm; semantic render 17.8 ms cold and 5.7 ms warm; transform 3.1 ms; 360×240 resize 4.5 ms; 720×480 PNG encode 24.4 ms; 2400×1600 render 313.7 ms and PNG encode 836.2 ms.

## Profiling findings

Stage timers, controlled adjustment-only runs, cache counters, and byte accounting identified the following:

1. Texture, Clarity, and Dehaze were the dominant CPU stage. Each active control performs a bounded resize, Gaussian blur, resize-back, and full-image luminance remap. The combined presence probe remains the largest 720×480 stage at about 114 ms.
2. The earlier test profile compiled application and image-processing code without optimisation. Setting optimisation level 1 for development and test preserves debuggability while allowing normal loop optimisation; release builds remain governed by Cargo's release profile.
3. Every mask previously allocated a full output clone, recomputed its adjustment image, evaluated coverage per pixel, and composited serially. The adjusted image remains necessary to preserve ordered mask semantics, but raster generation and compositing are now parallel and reusable.
4. Semantic coverage was previously base64-decoded, checksum-verified, PNG-decoded, resized, and feathered on every render. It is now cached independently from the final mask raster.
5. Preview JPEG data was decoded and resized for every slider request. It is now reused while path, byte length, modification time, and requested dimensions remain unchanged.
6. PNG encoding is material at full resolution (about 807 ms), but `Balanced` compression keeps output lossless and avoids the very large files observed with the fastest setting. It remains below the end-to-end target, so format or quality was not weakened.

The working format remains contiguous RGB8 after decode; mask coverage is a contiguous `f32` plane and semantic coverage is Gray8. This avoids repeated dynamic pixel-format conversions in inner loops. Full-frame clones remain where ordered masks require before/after source and target buffers, but decoded frames, global intermediates, and coverage planes are no longer repeatedly allocated for unchanged dependencies.

The hot functions were `apply_protected_local_contrast`, `apply_colour_adjustments_to_image`, `prepared_semantic_coverage`, per-pixel `mask_coverage_prepared`, and the mask composite in `render_develop_recipe`. Allocation pressure came from decoded buffers, global intermediates, mask-adjusted full frames, blurred working buffers, resized semantic planes, and PNG output buffers. Cache metrics report actual resident derivative bytes, hits, misses, and evictions in the opt-in benchmark.

## Renderer architecture

The pipeline remains deterministic:

```text
oriented source
  -> bounded preview decode
  -> global tone/white-balance/presence/colour intermediate
  -> ordered enabled masks (coverage -> local adjustment -> composite)
  -> flip/rotate/straighten/crop
  -> lossless sRGB PNG
```

Rayon uses one shared, bounded process pool, so independent pixels and raster indices run in parallel without creating a thread pool per render. Per-pixel calculations retain their original operation order and rounding. Tests compare cached/parallel output byte-for-byte with the uncached renderer and repeat renders concurrently.

The frontend coalesces to the latest queued slider value, permits no more than two in-flight requests, and rejects completions from older generations. The Rust preview command also assigns a generation and stops superseded work at global and per-mask safe boundaries. A superseded request cannot publish a preview file.

A lower-resolution progressive first pass was not added: the measured cold 720×480 result is already about 81 ms, so a second render would increase total work and risk a distracting resolution change.

## Cache ownership and invalidation

All caches are process-local, non-authoritative LRU stores guarded by a mutex. Values are immutable `Arc` buffers; expensive work occurs outside the lock except for short key lookup/insertion sections.

| Cache | Limit | Identity | Invalidated by | Deliberately retained across |
| --- | ---: | --- | --- | --- |
| Decoded source | 96 MiB | path text, byte length, modification timestamp, requested dimensions, plus catalogue source hash for full resolution | thumbnail/original replacement or change, decode size, or catalogue source change | recipe and mask changes |
| Global colour intermediate | 96 MiB | source identity plus colour/tone/presence settings | source or any global colour setting | crop, rotation, flip, straighten, mask-only changes |
| Final mask coverage | 64 MiB | geometry, feather, inversion, opacity, output dimensions | any coverage-affecting change | mask name/id and local adjustment value changes |
| Semantic coverage plane | 64 MiB | verified payload checksum, dimensions, feather | payload, size, or feather change | opacity, inversion, refinement, and local adjustment changes |

Eviction is least-recently-used. Oversize entries are not cached. Cache replacement never mutates a referenced buffer. A changed source produces a new preview artefact key as well as a decode miss. Switching images naturally selects a different source identity; prefetch was not added because measured cold JPEG decode was only about 10.5 ms and speculative buffers would reduce the useful cache budget.

## Local contrast, SIMD, and GPU decisions

The local-contrast algorithm, scale, endpoint protection, and three-operation ordering are unchanged. Its per-pixel luminance remap is parallel; the image crate's bounded resize/blur implementation is retained. This produced 72 ms on the original Develop checkpoint while the byte-for-byte equivalence tests stayed green.

The native CPU advertises AVX2/AVX-512, and optimised Rust/image dependencies can auto-vectorise suitable loops. Explicit architecture intrinsics were not added: the hot logic is branch-heavy floating-point work, explicit SIMD would require a scalar fallback and could change rounding, and the measured CPU result already exceeds the target.

The available RTX 5090 was considered after CPU work. A GPU spike was not justified by the measured evidence: the complete cold preview is about 81 ms, warm global render about 1.2 ms, ten masks about 64 ms, and full render plus PNG about 1.15 s. A new texture upload/readback, shader implementation, device lifecycle, fallback path, and CPU/GPU equivalence surface would cost more complexity than the remaining latency. GPU acceleration is therefore not a milestone deliverable and no GPU performance claim is made.

## Verification

Functional coverage includes bounded LRU eviction, unchanged decode reuse, changed-source invalidation, geometry-only global reuse, colour-setting invalidation, mask reuse/invalidation, semantic sub-cache reuse, cached versus uncached byte equality, concurrent determinism, stale generation rejection, frontend supersession, disabled/overlapping/manual/semantic masks, geometry parity, export geometry, and protected-original hashes.

Run all authoritative checks:

```powershell
.\scripts\check-all.ps1
```

Run only the performance evidence:

```powershell
.\scripts\benchmark-renderer.ps1
```

No external RAW fixture directory was present at `D:\Keepframe-private-fixtures` during this milestone. RAW performance and native end-to-end interaction therefore remain explicit external checks; no embedded preview or reduced-quality fallback was introduced.

## Native acceptance checklist

- Open an installed Windows build against a copied test library.
- Switch quickly between at least three JPEG/PNG/TIFF assets and confirm no prior image flashes after selection.
- Drag Exposure, Temperature, Tint, Texture, Clarity, and Dehaze continuously; confirm controls remain responsive and only the newest preview settles.
- Repeat with one, five, and ten enabled masks, including brush and overlapping masks.
- Accept a real semantic mask, adjust opacity/inversion/feather/refinements, switch away and back, and confirm alignment and cache-safe invalidation.
- Exercise crop, rotate, straighten, horizontal flip, vertical flip, before/after, undo/redo, preset preview/apply/cancel, and mask solo.
- Export a developed JPEG, PNG, TIFF, and each available RAW fixture; inspect dimensions, orientation, sRGB declaration, and visible parity.
- Hash protected originals before and after the session and confirm every digest is unchanged.
- Observe Task Manager during rapid editing and image switching; confirm memory remains bounded and falls within the documented 320 MiB aggregate cache ceiling plus active render buffers.

## Remaining limitations

- Texture/Clarity/Dehaze remains the largest CPU stage and may scale with unusually large preview dimensions, although interactive previews are intentionally bounded to 720×540.
- Full-resolution PNG compression is now the largest measured export stage. It remains lossless and inside the target; a user-facing JPEG export policy was not invented in this milestone.
- Cancellation occurs at stage boundaries, not inside an individual blur or encoder call.
- Cache limits cover retained derivatives, not transient current/blur/encode buffers.
- Native WebView timing and visual click-through, real RAW render timing, clean-machine packaging, and third-party colour-management inspection require external/native execution and are not claimed by automated checks.

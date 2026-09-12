# Milestone 7 — Intelligent Editing and Adaptive Presets

## Non-destructive boundary

Develop remains the only authority for photographic adjustments. Accepted Auto and preset values become ordinary version-1 Develop recipe values, use the existing preview cache and full-resolution renderer, and can be copied, pasted, undone, redone and exported. Neither feature writes a source photograph, metadata, a path, cached pixels or UI state into a recipe or preset.

## Presets

The built-ins are Natural, Clean, Warm, Cool, High Contrast, Soft Contrast, Vivid, Muted, Portrait, Landscape, Black & White, High-Key B&W and Low-Key B&W. They are code-defined and apply only named photographic categories: white balance, tone, presence and colour. They intentionally exclude crop, rotation and flips.

Custom presets are stored per catalogue in schema-v8 `develop_presets`, survive restart, and can be saved, exported, imported and deleted. Deletion never changes recipes that previously used a preset. The portable UTF-8 `.keepframe-preset` JSON has schema version, name, category list and validated Develop settings only. Import is untrusted: unknown categories, unsupported schema versions, malformed JSON, invalid numeric ranges and files over 128 KiB fail safely. Built-ins cannot be deleted.

## Auto and recommendations

Auto uses no cloud service and no model. It analyses a bounded 720×540-or-smaller source-derived thumbnail off the UI thread. It measures RGB and luminance histograms, average and percentile luminance, dynamic range, shadow/highlight clip fractions, average saturation and RGB channel averages. It then adds restrained deltas to the *current* recipe while preserving crop, rotation and flips. This avoids silently replacing an existing creative edit.

Rules are explicit: under-represented midtones can raise exposure, clipping can reduce highlights, limited dark detail can raise shadows, flat range can add modest contrast, and low saturation can add modest vibrance. White balance uses a low-saturation grey-world estimate only when confidence is sufficient; otherwise it remains unchanged. Auto never recommends or applies black and white. Portrait, Landscape and Soft Contrast may be suggested from the same measured rules, but are never applied automatically.

Auto and preset selection are proposal workflows: analyse/select, inspect the rendered preview and explanation, then Apply or Cancel. Cancel restores the exact prior recipe. Apply is one history item. Request tokens reject stale results after asset changes; the proposal is not persisted until Apply.

## Histogram and clipping guide

The Develop histogram reads the displayed, bounded preview and shows luminance plus RGB distributions, so it updates when the preview updates. It reports the number of near-black and near-white pixels. The optional clipping guide is display-only and does not affect recipes or exports. Its thresholds are luminance <= 2% and >= 98% in analysis (the UI preview reports approximately 2%/98% equivalents).

## Performance and privacy

Analysis avoids full RAW decode and runs in a blocking worker, leaving React responsive. Preview rendering remains content-addressed and bounded; warm cache hits do not re-render. On the debug synthetic 720×480 checkpoint, histogram sampling took 162.8 ms, Auto proposal creation 160.7 ms, Auto preview rendering 124.1 ms and a warm-preset render 123.5 ms. No scene-classification model was added: the existing optional Qwen worker is unrelated to Develop and is not required or invoked here. This avoids a large new model/runtime dependency and keeps photographs local.

The M6 debug benchmark remains a renderer checkpoint, not a native performance claim. Re-measured on this checkout: 720×480 preview 1.458 s, warm cache-key 310.9 µs, and 2400×1600 render plus PNG 7.842 s. M7 adds only bounded histogram/statistics sampling and does not rewrite the renderer. Native validation should measure a real RAW separately, because its thumbnail generation and decoding path can differ.

## Known limitations

The heuristic is intentionally conservative and cannot identify subjects, skin tones or photographic intent. The histogram is derived from the displayed preview, not a colour-managed full-resolution pipeline. The clipping guide is an indicator rather than a per-pixel overlay. Presets are catalogue-local until exported/imported. Real native RAW, restart and export visual comparison remain manual acceptance checks.

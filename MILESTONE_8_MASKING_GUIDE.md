# Milestone 8 masking guide

## Storage and migration

Catalogue schema v9 introduces no pixel table or authoritative mask cache. It records that the application understands Develop recipe v2. A v2 recipe contains the existing global `settings` object and an ordered `masks` array. Each mask stores an ID, name, enabled flag, inversion, opacity, feather, geometry and local adjustments.

The v8-to-v9 catalogue migration deliberately leaves existing recipe JSON unchanged. Recipe v1 is decoded with an empty mask list and upgraded in memory to v2. It therefore produces the same photograph until the user next saves it. A neutral recipe with no masks is still represented by the absence of a `develop_recipes` row.

Disabled masks remain persisted and count as edits because they are editable recipe intent. Removing the last mask and returning all global controls to neutral clears Edited state and deletes the recipe row.

## Canonical coordinates and geometry

All coordinates are normalised to `[0, 1]` on the decoded, EXIF-oriented, uncropped source image. They do not use preview pixels, viewport pixels, zoom or pan. The renderer applies masks before user crop and transform operations, so a later crop, flip, quarter rotation or straighten operation moves the already-adjusted photograph and mask together.

The UI converts pointer positions through the inverse presentation transform. Its conversion accounts for crop, horizontal/vertical flips, quarter rotations, straightening and source aspect ratio. Fit, 100%, arbitrary zoom and pan affect only the image layer presentation and never persisted coordinates.

- Linear gradients store normalised start and end points. Feather controls the smooth transition about their midpoint; swapping the points reverses direction.
- Radial gradients store centre, x/y radii and rotation. Feather blends from the solid inner ellipse to the outer boundary. Inversion switches inside and outside coverage.
- Brush masks store ordered strokes. Each stroke contains normalised points, an image-relative radius (a fraction of the shorter source dimension), feather, flow and an erase flag. A continuous pointer gesture is one stroke and one history entry. Erase strokes subtract coverage without destructively rewriting earlier paths.

The vector stroke approach keeps catalogue data compact, editable and independent of output resolution. It avoids authoritative full-resolution bitmaps. Very large untrusted recipes are rejected (64 masks, 5,000 strokes or 200,000 points maximum).

## Local controls and render order

Every mask supports Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Temperature, Tint, Saturation, Texture, Clarity and Dehaze. The controls reuse the real photographic renderer; none are overlay-only simulations.

The deterministic render order is:

1. decode the protected source and apply EXIF orientation;
2. apply global white balance, dynamic range, exposure, tone, colour and protected local contrast;
3. process enabled masks in recipe order;
4. for each mask, render its local adjustments from the current result and blend it by clamped coverage multiplied by opacity;
5. apply horizontal/vertical flips, quarter rotation and straighten;
6. crop;
7. encode a preview or full-resolution export.

Coverage and colour values are clamped. Paint strokes use alpha-over accumulation, erase strokes multiply existing coverage by inverse stroke alpha, and overlapping masks compose sequentially in list order. This makes the same source and recipe deterministic, although changing mask list order would intentionally change the result when nonlinear adjustments overlap.

## Interaction and history

The Masks panel creates, selects, renames, enables/disables, duplicates, inverts and deletes masks. Only the active mask shows geometry. Linear endpoints and radial centre/radius handles remain editable. Radial rotation is numeric. Brush Paint and Erase use an immediate SVG overlay; the photographic preview is recomposed only when the stroke finishes.

Slider and text changes are coalesced into a single history step after 350 ms. Create, duplicate, invert, enable/disable, delete, clear and each completed pointer gesture are immediate logical steps. Undo and redo store complete recipes, so masks and global controls travel together. Escape cancels the drawing tool; Delete removes the selected mask when focus is not in an input. Switching assets cancels stale preview delivery, flushes pending persistence for the previous asset and clears UI-only overlay/tool state.

The overlay and solo toggle are UI-only. They are not stored and cannot appear in exports. Solo preview temporarily disables other masks in the preview request without changing persisted flags.

## Copy, presets and Auto

`Copy settings` and Ctrl/Cmd+C copy global settings only. `Copy with masks` is the explicit mask category. Pasting masks creates new IDs and retains normalised geometry so it occupies the equivalent relative position on another photograph.

Develop presets remain schema v1 global looks and cannot contain masks. Saving a custom preset reads only global settings. Built-in presets, imported presets and Milestone 7 Auto change global controls only; they preserve the existing mask list and never create masks.

## Preview cache and export

Preview filenames are derived from the asset ID and the complete validated recipe JSON, with a v2 cache namespace. Any geometry, stroke, opacity, inversion, enablement or local-control change therefore yields a new derived preview. Missing preview files are regenerated. No mask cache is portable or authoritative, and catalogue integrity/rescan sees only files already inside the existing disposable preview area.

Full-resolution export decodes the protected original and rasterises each authoritative mask at that resolution. It never enlarges a preview mask. The same `render_develop_recipe` function serves preview and export, and generated tests cover gradients, radial masks, brush paint/erase, inversion, overlap, crop, rotation, flips, resolution-equivalent coverage, export decode and original SHA-256 preservation.

## Performance findings

Debug-profile checkpoints on the development PC (synthetic images, 12 September 2026) measured:

| Stage | Time |
| --- | ---: |
| Existing 720×480 global Develop preview | 1.479 s |
| Existing warm recipe/cache-key calculation | 0.255 ms |
| Existing 2400×1600 render plus PNG encode | 8.411 s |
| 720×480 PNG decode | 20.4 ms |
| 720×480 global pipeline with Clarity | 983 ms |
| One 720×480 linear mask raster pass | 7.1 ms |
| 720×480 render, one local mask | 269 ms |
| 720×480 render, five local masks | 847 ms |
| 720×480 render, ten local masks | 1.717 s |
| 720×480 PNG encode | 131 ms |

The protected local-contrast/global adjustment pipeline dominates; mask geometry rasterisation is comparatively small. Each local mask currently evaluates a full adjusted working image before coverage blending. Region-aware local-contrast evaluation or a broader renderer redesign could reduce multi-mask cost, but is deferred to a dedicated performance milestone because changing that seam risks preview/export parity.

## Limits and deferred work

- Luminance-range and colour-range masks are deferred to protect the core gradient/brush architecture.
- Subject, sky, person and object segmentation are explicitly deferred; a future intelligent mask can add another geometry/coverage provider to recipe v2.
- Masks are an ordered list; boolean add/subtract/intersect combinations between separate masks are not exposed. Brush erase provides subtraction within a brush mask.
- Brush paths remain editable as a mask and can be cleared, but individual historical strokes are not selectable as separate UI objects; Undo/Redo removes or restores them one logical stroke at a time.
- Debug profiling is a comparative engineering checkpoint, not a release-mode hardware guarantee.

## Disposable native smoke checklist

Use a disposable catalogue: import JPEG; open Develop; create and move a linear mask; adjust exposure; restart and verify persistence; create/invert/rotate a radial mask; create a brush mask; paint and erase; zoom to 100% and continue painting; crop, rotate and flip; verify masks stay attached; undo/redo; explicitly copy with masks to a second image; export PNG; compare the full-resolution result; reset; verify Edited clears and the original hash is unchanged. Repeat with one genuine supported RAW file. Native WebView interaction and a private RAW file remain external manual acceptance items unless explicitly reported as completed.

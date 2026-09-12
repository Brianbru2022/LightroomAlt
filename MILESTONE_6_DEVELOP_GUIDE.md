# Keepframe Develop workspace

Develop is a non-destructive workspace. It stores one small, versioned recipe per photograph in the local catalogue; it never changes, renames, recompresses or writes metadata into the protected original. Previews in `.keepframe/previews` are disposable derived data and can be rebuilt at any time.

## Controls and ranges

The recipe supports Exposure (-3 to +3 EV); Contrast, Highlights, Shadows, Whites, Blacks, Temperature, Tint, Texture, Clarity, Dehaze, Vibrance and Saturation (-100 to +100); quarter-turn rotation; horizontal and vertical flips; Straighten (-15 to +15 degrees in the interface; the validated catalogue range is -45 to +45); and a normalised free crop. Crop presets are Original/free, 1:1, 4:3/3:4, 3:2/2:3 and 16:9/9:16. The original/as-shot reset returns every setting to its central default.

Each control has a keyboard-accessible slider, a numeric input, and a per-control reset. The complete recipe can be reset. A recipe is marked edited only when it differs from the central defaults.

## Recipe and render order

`develop_recipes` is introduced by atomic catalogue migration v7. Each asset has at most one record:

```json
{ "schemaVersion": 1, "settings": { "exposure": 0, "cropWidth": 1, "horizontalFlip": false } }
```

The renderer applies source orientation during decoding, then white balance, protected range/tone and exposure, presence, colour, flips/rotation/straightening, crop and output scaling. It uses the same Rust routine for previews and full-resolution export. The intermediate is float arithmetic until the existing sRGB PNG encoder; it intentionally protects endpoint clipping where the legacy tone renderer already does so.

## Preview, history and export

Interactive previews use the existing thumbnail-resolution decode, bounded to 720×540, and a content-addressed `.keepframe/previews/develop-*` cache. Requests are coalesced to the latest setting and stale completions are ignored. A final recipe is persisted after a brief idle period; a slider drag is grouped into one undo step, with a bounded 80-step session history. History is session-local, while the final recipe survives restart.

Copy/Paste copies only the Develop settings object. It cannot transfer filename, capture time, triage state, tags, GPS, title/caption, identifiers or paths because none are part of the type or stored recipe.

An active Develop recipe takes precedence when exporting. Keepframe decodes the protected full-resolution original (including the existing RAW path where LibRaw succeeds), applies the recipe, and writes an explicit PNG derivative to the user-chosen export folder. It never exports an upscaled interactive preview or bakes the change into the source.

## Boundaries

Colour handling remains predictable sRGB output. Keepframe honours source orientation and emits tagged sRGB PNGs, but does not yet implement a full ICC wide-gamut working-space pipeline or soft proofing.

RAW continues to use the existing LibRaw full-resolution decoder. Camera-specific success remains a manual native test; a missing/failed RAW decode fails rather than substituting a preview. HEIC full-resolution export remains unavailable when the platform decoder cannot provide a full image.

Crop is normalised to the oriented image dimensions, so it is independent of preview pixel size. The crop overlay is deliberately free-form; ratio choices initialise a ratio but do not lock subsequent handle movements.

## Shortcuts

- Ctrl/Cmd+Z: undo; Ctrl/Cmd+Shift+Z: redo
- Ctrl/Cmd+C and Ctrl/Cmd+V: copy/paste Develop settings
- Backslash: toggle original
- R: reset Develop settings
- 0: fit image; 1: 100% zoom
- Left/Right: previous/next photograph when a form input is not focused

## Native smoke checklist

Use a disposable catalogue: import JPEG, and an approved RAW fixture if available; adjust several controls; check interaction and before/after; close/reopen and confirm persistence; undo/redo; copy to another asset while checking metadata is unchanged; crop and reopen; export; compare export with the preview; reset; and verify the original SHA-256 remains unchanged.

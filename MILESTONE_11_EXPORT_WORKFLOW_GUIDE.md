# Milestone 11 — Professional Export & Output Workflow

## Scope and safety contract

Milestone 11 adds a first-class local output workflow above the accepted Milestone 10 renderer. Export is derivative-only: it never rewrites a representation, XMP sidecar, catalogue asset, Develop recipe, accepted mask, or preferred version. Each queued asset is resolved independently, decoded from its protected full-resolution representation (including genuine LibRaw decoding for supported RAW files), rendered through the accepted global/geometry/local/semantic-mask pipeline, resized, sharpened, metadata-filtered, encoded to a temporary file, verified, then promoted into the chosen destination.

Previews, thumbnails, solo-mask previews, unaccepted Auto proposals, and unaccepted intelligent-mask predictions are not export inputs. Existing M10 cache keys remain source/recipe/geometry sensitive; the bounded full-resolution decode and intermediate caches are reused without admitting preview-sized content.

## Architecture

The native pipeline is:

`validated ExportConfig → per-asset source/recipe resolution → full-resolution Develop render → Lanczos3 resize → output sharpening → format encoder + sRGB ICC → standard metadata policy → decode/profile verification → temporary-file promotion → per-item report`

`start_export_batch` runs on Tauri's blocking pool so the UI remains responsive. The queue is deliberately serial (`concurrency: 1`): this limits simultaneous full-resolution buffers, avoids encoder and disk contention, and gives cancellation a predictable safe boundary. A generation token cancels both an active renderer and all remaining items. Already promoted files remain valid; unfinished temporary files are removed. Failures are isolated per item and returned alongside complete, skipped, and cancelled counts.

The catalogue schema is version 10. Only user-authored export presets are persisted in `export_presets`; built-ins remain read-only in code. Queue state and destination paths are session-only.

## Formats and colour

- JPEG: RGB 8-bit, 4:2:2 encoder, quality 1–100, default 90, JFIF PPI metadata, embedded standard sRGB ICC.
- PNG: lossless RGB 8-bit, Fast/Balanced/Best DEFLATE controls (not a false quality slider), embedded standard sRGB ICC.
- TIFF: lossless RGB 8-bit, uncompressed, embedded standard sRGB ICC. This build does not offer or claim 16-bit TIFF, alpha, layers, or TIFF compression.
- Colour: this milestone offers sRGB only. A valid RGB `sRGB Color Space Profile.icm` is loaded from Windows and embedded in all three formats; export fails clearly rather than writing an unprofiled file. No Adobe RGB or Display P3 option is shown because no wider-gamut transform is implemented.

The final output is reopened, its actual container and dimensions are checked, and its RGB ICC profile is validated. TIFF ICC verification uses bundled ExifTool because the current image decoder does not expose that tag reliably.

## Sizing and sharpening

Sizing modes are original dimensions, bounding box, long edge, short edge, and percentage. Aspect ratio is always preserved. “Do not enlarge” is on by default. Pixel dimensions control resampling; PPI is metadata assistance only and never changes pixel dimensions. Lanczos3 is used deliberately for final resampling.

Output sharpening is applied after resize. None, Low, Standard, and High are bounded unsharp-mask strengths; they are output sharpening, not new Develop controls. The dialog estimates final dimensions and megapixels from catalogue dimensions plus the accepted crop/quarter-turn recipe. It shows format, colour, metadata/GPS choice, and a filename preview. File-size text is explicitly non-exact because photographic content and metadata affect compression.

## Metadata and location privacy

Policies are All practical metadata, Copyright & contact, Copyright only, and None. “All” may preserve standard capture date, title, caption, copyright/contact, camera, and other ordinary source metadata; catalogue keywords and the embedded source rating are individually controllable. GPS/location is an independent opt-in and is off by default. If enabled, the current catalogue location is written; if disabled, all GPS groups are removed. Orientation is set to 1 because pixels have already been normalised.

Copyright-only modes also remove GPS, keywords, subjects, and ratings. Internal asset ids, paths, recipe JSON, mask payloads, cache keys, and job provenance are never added. Export does not create or alter the Milestone 5 sidecar. Standard EXIF/IPTC/XMP fields may be embedded in the derivative container only; no private Keepframe namespace is written.

## Filenames, destinations, and collisions

Supported tokens are `{filename}`, `{stem}`, `{sequence}`, `{capturedate}`, `{exportdate}`, `{rating}`, and `{custom}`. Sequence start and padding are explicit. The selected format owns the extension. Windows-invalid characters and control characters are replaced, trailing spaces/periods are removed, empty names become `photograph`, device names such as `CON` are prefixed, and the stem is capped at 180 characters. The stem is shortened further where necessary to keep the complete destination path inside a conservative 240-character Windows interoperability budget. Templates containing path separators, empty text, unclosed tokens, or unknown tokens fail before export; the canonical destination check prevents folder escape.

Exports target one explicitly selected folder. Collision policy is Unique (default), Skip, Replace, or Ask/stop-at-conflict. Batch work never raises repeated native dialogs. Unique adds a deterministic numeric suffix. Replace writes and verifies a UUID temporary file first, then uses the platform rename/replace operation. A failed encode, metadata write, validation, cancellation, or promotion attempts temporary cleanup and reports any item failure without deleting earlier successful output.

Before work begins, a conservative four-bytes-per-final-pixel estimate is checked against destination free space. Assets with unknown dimensions reserve 512 MiB each rather than being underestimated. This is intentionally cautious, especially for JPEG.

## Queue, progress, results, and presets

The Develop export dialog contains a dedicated queue of the currently loaded catalogue results, initially containing the active photograph. It does not pretend the existing single-selection library is multi-select. Progress events expose Waiting, Rendering, Resizing and sharpening, Encoding, Writing, and Complete phases with current/completed/total, filename, failures, skipped, and cancelled counts. Final per-item states are Complete, Failed, Cancelled, or Skipped, with useful user-facing errors rather than raw stack traces.

The final panel gives counts, elapsed time, per-file dimensions/path/error, destination, and an Open Folder action. Built-in presets are Web JPEG, Full-size JPEG, and Archive TIFF. User presets can be saved, renamed by saving the selected user preset under a new name, duplicated, deleted, applied, imported, and exported as versioned `.keepframe-export-preset` JSON. Portable presets intentionally exclude destination paths.

## Verification and benchmarks

Automated coverage includes settings validation, resize modes/aspect/no-enlarge, sharpening geometry, Unicode/Windows filename sanitisation, unknown/path token rejection, deterministic collision policies, JPEG/PNG/TIFF decode and ICC validation, full-resolution Develop/mask lossless pixel parity, protected-original hash preservation, temporary cleanup, partial failure, cancellation, GPS inclusion/removal, keyword removal, internal-field non-leakage, preset persistence/portability, safe replace, unique naming, and the atomic schema migration. Existing M10 cache, renderer, preview, mask, geometry, RAW fail-closed, and full-resolution parity tests remain in the authoritative suite.

The opt-in benchmark is `scripts/benchmark-export.ps1`; raw output is written to `artifacts/m11-export-benchmark.txt`. On this checkout, the deterministic 3600×2400 RGB source used an accepted local mask; resized variants were 2400×1600:

| Format | Render | Resize | Sharpen | Encode + staged write | Metadata | Final promotion | Total | Bytes |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| JPEG q90 full | 427 ms | 0 ms | 218 ms | 10,218 ms | 435 ms | 0 ms | 11,298 ms | 7,009,777 |
| JPEG q90 resized | 427 ms | 362 ms | 102 ms | 5,365 ms | 452 ms | 0 ms | 6,708 ms | 3,695,622 |
| PNG balanced full | 427 ms | 0 ms | 227 ms | 1,089 ms | 808 ms | 0 ms | 2,551 ms | 5,051,975 |
| PNG balanced resized | 427 ms | 367 ms | 112 ms | 636 ms | 784 ms | 0 ms | 2,326 ms | 4,459,292 |
| TIFF 8-bit uncompressed full | 427 ms | 0 ms | 235 ms | 15 ms | 471 ms | 0 ms | 1,148 ms | 25,923,640 |

The ten-image 3600×2400 JPEG workload used mixed Develop settings and accepted-mask/no-mask states: 99,815 ms total, 9,981 ms average, serial concurrency 1. The largest tracked live RGB allocation was 51,840,000 bytes for the retained source plus one developed item; this excludes allocator, encoder, cache, and whole-process overhead. M10 cache budgets remain 96 MiB decoded, 96 MiB intermediates, 64 MiB mask coverage, and 64 MiB semantic masks. The serial policy is retained because JPEG encoding dominates here and extra simultaneous full-resolution buffers would increase memory/disk pressure without evidence of useful throughput.

## Honest limitations

- TIFF is 8-bit uncompressed only.
- Only sRGB output is implemented.
- “All” metadata is limited by what ExifTool can safely copy into the selected output container; unsupported source tags may be omitted.
- PPI is metadata, not a print-layout or physical-size engine.
- The queue covers currently loaded catalogue results; it is not catalogue-wide selection persistence.
- Ask collision mode reports the conflict for batch-safe resolution rather than raising one dialog per file.
- HEIC full-resolution decoding remains fail-closed. Supported RAW export requires the bundled LibRaw decoder and a genuinely decodable file; embedded previews are never substituted.
- Automated fixtures exercise standard JPEG/PNG/TIFF and deliberate invalid RAW. A representative user-owned real RAW file was not available in the repository.

## Native Windows acceptance checklist

These checks require the packaged Windows WebView and real user-owned files; do not infer them from unit tests.

1. Open Develop and launch Export photographs.
2. Confirm the active photograph is the initial queue item.
3. Add and remove several loaded photographs.
4. Apply each built-in output preset.
5. Save, rename, duplicate, and delete a user preset.
6. Export and re-import a portable preset; confirm no destination is restored.
7. Choose JPEG and test quality 1, 90, and 100.
8. Choose PNG and test Fast, Balanced, and Best compression.
9. Choose TIFF and confirm the 8-bit/uncompressed disclosure.
10. Confirm only sRGB is offered.
11. Test original-dimension output.
12. Test bounding-box resize in landscape and portrait orientation.
13. Test long-edge and short-edge resize.
14. Test percentage resize.
15. Confirm no-enlarge prevents upscaling; disable it and confirm opt-in upscaling.
16. Verify PPI changes metadata but not pixel dimensions.
17. Compare None/Low/Standard/High output sharpening at 100%.
18. Verify accepted crop, rotation, flips, global settings, and every accepted mask on output.
19. Confirm an unaccepted Auto proposal is absent.
20. Confirm an unaccepted intelligent-mask prediction is absent.
21. Inspect All, Copyright & contact, Copyright, and None in ExifTool or another trusted inspector.
22. Confirm GPS is absent by default and present only when explicitly enabled.
23. Confirm keywords/rating toggles and capture/title/caption behaviour on suitable source files.
24. Exercise every filename token, Unicode, invalid characters, trailing dots, and a reserved device name.
25. Exercise Unique, Skip, Replace, and Ask against existing files.
26. Cancel during rendering and confirm current/remaining states and no residual temporary files.
27. Export a batch containing one unavailable/corrupt asset; confirm later valid items continue.
28. Use Open Folder and confirm it targets the selected destination.
29. Resize the window and use keyboard-only navigation; check focus, labels, clipping, scrolling, and contrast.
30. Export a real supported RAW and compare it with the Develop preview at crop/mask boundaries and 100% detail.

## Validation commands

```powershell
& .\scripts\check-all.ps1
& .\scripts\benchmark-export.ps1
```

Milestone 11 is complete in source and automated verification. Packaged native click-through and real user-owned RAW acceptance remain external verification gates.
